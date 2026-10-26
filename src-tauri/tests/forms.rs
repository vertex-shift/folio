use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use folio_core::{
    document::{Edit, Session},
    forms::{FieldChange, FieldKind, FieldValue, FormField},
    workspace::Workspace,
};
use lopdf::{
    Document, LoadOptions, Object, ObjectId, Stream, decode_text_string, dictionary,
    encryption::{
        EncryptionState, EncryptionVersion, Permissions,
        crypt_filters::{Aes128CryptFilter, Aes256CryptFilter, CryptFilter},
    },
    text_string,
};

struct Fixture {
    document: Document,
    fields: HashMap<&'static str, ObjectId>,
    widgets: HashMap<&'static str, Vec<ObjectId>>,
}

fn numbers(values: &[f32]) -> Object {
    Object::Array(values.iter().map(|value| Object::Real(*value)).collect())
}

fn appearance(document: &mut Document, on: bool) -> ObjectId {
    document.add_object(Stream::new(
        dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 20.into(), 20.into()], "Resources" => dictionary! {} },
        if on { b"q 0 g 4 4 12 12 re f Q".to_vec() } else { b"q 0 G 1 w 1 1 18 18 re S Q".to_vec() },
    ))
}

fn widget(
    document: &mut Document,
    page: ObjectId,
    rect: [f32; 4],
    state: Option<&str>,
) -> ObjectId {
    let off = appearance(document, false);
    let normal = if let Some(state) = state {
        let on = appearance(document, true);
        let mut states = dictionary! { "Off" => off };
        states.set(state, on);
        Object::Dictionary(states)
    } else {
        Object::Reference(off)
    };
    let id = document.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "P" => page, "Rect" => numbers(&rect),
        "F" => 4, "AP" => dictionary! { "N" => normal }, "AS" => "Off",
        "MK" => dictionary! { "BG" => numbers(&[1.0, 1.0, 1.0]), "BC" => numbers(&[0.3, 0.3, 0.3]) },
        "BS" => dictionary! { "W" => 1, "S" => "S" },
    });
    let page = document.get_dictionary_mut(page).unwrap();
    if !page.has(b"Annots") {
        page.set("Annots", Vec::<Object>::new());
    }
    page.get_mut(b"Annots")
        .unwrap()
        .as_array_mut()
        .unwrap()
        .push(id.into());
    id
}

fn fixture() -> Fixture {
    let mut document =
        Document::load_mem(Session::blank(612.0, 792.0, 1).unwrap().bytes()).unwrap();
    let first = document.get_pages()[&1];
    let parent = document
        .get_dictionary(first)
        .unwrap()
        .get(b"Parent")
        .unwrap()
        .as_reference()
        .unwrap();
    let second = document.add_object(document.get_dictionary(first).unwrap().clone());
    let pages = document.get_dictionary_mut(parent).unwrap();
    pages.set(
        "Kids",
        vec![Object::Reference(first), Object::Reference(second)],
    );
    pages.set("Count", 2);
    let font = document.add_object(
        dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" },
    );
    let mut fields = HashMap::new();
    let mut widgets = HashMap::new();

    let namespace = document.add_object(dictionary! {
        "T" => text_string("person"), "FT" => "Tx", "Ff" => 2,
        "V" => text_string("Inherited"), "MaxLen" => 40,
        "DA" => Object::string_literal("/Helv 0 Tf 0 g"),
    });
    let name = document.add_object(dictionary! { "T" => text_string("name"), "TU" => text_string("Full name"), "Parent" => namespace });
    let name_widgets = vec![
        widget(&mut document, first, [45.0, 700.0, 300.0, 730.0], None),
        widget(&mut document, second, [45.0, 700.0, 300.0, 730.0], None),
    ];
    for id in &name_widgets {
        document
            .get_dictionary_mut(*id)
            .unwrap()
            .set("Parent", name);
    }
    document.get_dictionary_mut(name).unwrap().set(
        "Kids",
        name_widgets
            .iter()
            .copied()
            .map(Object::Reference)
            .collect::<Vec<_>>(),
    );
    fields.insert("person.name", name);
    widgets.insert("person.name", name_widgets);
    let notes = widget(&mut document, first, [45.0, 590.0, 310.0, 675.0], None);
    document.get_dictionary_mut(notes).unwrap().extend(&dictionary! {
        "T" => text_string("notes"), "Parent" => namespace, "Ff" => 1 << 12, "V" => text_string(""),
        "MaxLen" => 300,
    });
    fields.insert("person.notes", notes);
    widgets.insert("person.notes", vec![notes]);
    document.get_dictionary_mut(namespace).unwrap().set(
        "Kids",
        vec![Object::Reference(name), Object::Reference(notes)],
    );

    let approved = widget(
        &mut document,
        first,
        [45.0, 540.0, 65.0, 560.0],
        Some("Accepted"),
    );
    document.get_dictionary_mut(approved).unwrap().extend(&dictionary! {
        "T" => text_string("approved"), "TU" => text_string("Approve request"), "FT" => "Btn", "V" => "Off",
    });
    fields.insert("approved", approved);
    widgets.insert("approved", vec![approved]);
    let radio = document.add_object(dictionary! {
        "T" => text_string("shipping"), "FT" => "Btn", "Ff" => (1 << 15) | (1 << 14),
        "V" => "Ground", "Opt" => vec![text_string("standard-export"), text_string("rush-export")],
    });
    let ground = widget(
        &mut document,
        first,
        [45.0, 495.0, 65.0, 515.0],
        Some("Ground"),
    );
    let express = widget(
        &mut document,
        second,
        [45.0, 495.0, 65.0, 515.0],
        Some("Express"),
    );
    for id in [ground, express] {
        document
            .get_dictionary_mut(id)
            .unwrap()
            .set("Parent", radio);
    }
    document
        .get_dictionary_mut(ground)
        .unwrap()
        .set("AS", "Ground");
    document.get_dictionary_mut(radio).unwrap().set(
        "Kids",
        vec![Object::Reference(ground), Object::Reference(express)],
    );
    fields.insert("shipping", radio);
    widgets.insert("shipping", vec![ground, express]);

    let choices = widget(&mut document, first, [45.0, 360.0, 300.0, 460.0], None);
    document
        .get_dictionary_mut(choices)
        .unwrap()
        .extend(&dictionary! {
            "T" => text_string("services"), "FT" => "Ch", "Ff" => 1 << 21,
            "Opt" => vec![
                Object::Array(vec![text_string("a"), text_string("Alpha service")]),
                Object::Array(vec![text_string("b"), text_string("Beta service")]),
                Object::Array(vec![text_string("c"), text_string("Gamma service")]),
            ],
            "V" => vec![text_string("a")], "I" => vec![0.into()],
        });
    fields.insert("services", choices);
    widgets.insert("services", vec![choices]);
    let combo = widget(&mut document, first, [45.0, 305.0, 300.0, 335.0], None);
    document
        .get_dictionary_mut(combo)
        .unwrap()
        .extend(&dictionary! {
            "T" => text_string("country"), "FT" => "Ch", "Ff" => (1 << 17) | (1 << 18),
            "Opt" => vec![
                Object::Array(vec![text_string("US"), text_string("United States")]),
                Object::Array(vec![text_string("CA"), text_string("Canada")]),
            ],
            "V" => text_string("CA"), "I" => vec![1.into()],
        });
    fields.insert("country", combo);
    widgets.insert("country", vec![combo]);

    for (name, flags, value, y) in [
        ("password", 1 << 13, "old-secret", 250.0),
        ("locked", 1, "Read only", 205.0),
        ("comb", 1 << 24, "1234", 160.0),
    ] {
        let id = widget(&mut document, first, [45.0, y, 300.0, y + 30.0], None);
        document
            .get_dictionary_mut(id)
            .unwrap()
            .extend(&dictionary! {
                "T" => text_string(name), "FT" => "Tx", "Ff" => flags, "V" => text_string(value),
            });
        if name == "comb" {
            document.get_dictionary_mut(id).unwrap().set("MaxLen", 4);
        }
        fields.insert(name, id);
        widgets.insert(name, vec![id]);
    }
    let signature = widget(&mut document, first, [45.0, 100.0, 300.0, 135.0], None);
    document
        .get_dictionary_mut(signature)
        .unwrap()
        .extend(&dictionary! { "T" => text_string("signature"), "FT" => "Sig" });
    fields.insert("signature", signature);
    widgets.insert("signature", vec![signature]);
    let mut roots = vec![Object::Reference(namespace)];
    for name in [
        "approved",
        "shipping",
        "services",
        "country",
        "password",
        "locked",
        "comb",
        "signature",
    ] {
        roots.push(Object::Reference(fields[name]));
    }
    let acroform = document.add_object(dictionary! {
        "Fields" => roots, "DA" => Object::string_literal("/Helv 0 Tf 0 g"),
        "DR" => dictionary! { "Font" => dictionary! { "Helv" => font } },
        "NeedAppearances" => true,
    });
    document.catalog_mut().unwrap().set("AcroForm", acroform);
    Fixture {
        document,
        fields,
        widgets,
    }
}

fn bytes(mut document: Document) -> Vec<u8> {
    let mut bytes = vec![];
    document.save_to(&mut bytes).unwrap();
    bytes
}

fn session(fixture: Fixture) -> Session {
    Session::from_bytes(
        bytes(fixture.document),
        "Synthetic forms.pdf".into(),
        None,
        1,
    )
    .unwrap()
}

fn field(session: &Session, name: &str) -> FormField {
    session
        .form_fields()
        .unwrap()
        .fields
        .into_iter()
        .find(|field| field.name == name)
        .unwrap()
}

fn change(session: &Session, name: &str, value: FieldValue) -> FieldChange {
    FieldChange {
        id: field(session, name).id,
        value,
    }
}

fn text_change(session: &Session, name: &str, value: &str) -> FieldChange {
    change(session, name, FieldValue::Text { text: value.into() })
}

fn normal_stream(document: &Document, widget: ObjectId) -> &Stream {
    let ap = document.get_dictionary(widget).unwrap().get(b"AP").unwrap();
    let ap = document.dereference(ap).unwrap().1.as_dict().unwrap();
    document
        .dereference(ap.get(b"N").unwrap())
        .unwrap()
        .1
        .as_stream()
        .unwrap()
}

#[test]
fn inspects_inherited_qualified_fields_repeated_widgets_and_export_display_values() {
    let session = session(fixture());
    let info = session.form_fields().unwrap();
    assert!(info.fillable);
    assert_eq!(info.fields.len(), 10);
    let name = field(&session, "person.name");
    assert_eq!(name.label, "Full name");
    assert!(name.required);
    assert_eq!(name.max_length, 40);
    assert_eq!(
        name.value,
        Some(FieldValue::Text {
            text: "Inherited".into()
        })
    );
    assert_eq!(
        name.widgets
            .iter()
            .map(|widget| widget.page)
            .collect::<Vec<_>>(),
        [0, 1]
    );
    assert!(field(&session, "person.notes").multiline);
    let list = field(&session, "services");
    assert!(list.multiple);
    assert_eq!(list.options[0].label, "Alpha service");
    assert_eq!(list.options[0].export_value, "a");
    assert_eq!(
        list.value,
        Some(FieldValue::Choice {
            indices: vec![0],
            custom: None
        })
    );
    assert!(field(&session, "country").editable_choice);
    assert!(field(&session, "password").password);
    assert!(!session.info().unwrap().password_protected);
    assert!(field(&session, "locked").read_only);
    assert_eq!(field(&session, "signature").kind, FieldKind::Signature);
    assert!(field(&session, "signature").read_only);
}

#[test]
fn all_supported_values_persist_with_embedded_appearances_and_one_atomic_undo_step() {
    let fixture = fixture();
    let ids = fixture.fields.clone();
    let widgets = fixture.widgets.clone();
    let original = bytes(fixture.document);
    let mut session =
        Session::from_bytes(original.clone(), "Synthetic forms.pdf".into(), None, 1).unwrap();
    let approved = field(&session, "approved").options[0].id.clone();
    let express = field(&session, "shipping")
        .options
        .iter()
        .find(|option| option.export_value == "rush-export")
        .unwrap()
        .id
        .clone();
    let changes = vec![
        text_change(&session, "person.name", "Zoë Martin"),
        text_change(&session, "person.notes", "First line\nΔεύτερη γραμμή"),
        change(
            &session,
            "approved",
            FieldValue::Button {
                state: Some(approved),
            },
        ),
        change(
            &session,
            "shipping",
            FieldValue::Button {
                state: Some(express),
            },
        ),
        change(
            &session,
            "services",
            FieldValue::Choice {
                indices: vec![0, 2],
                custom: None,
            },
        ),
        change(
            &session,
            "country",
            FieldValue::Choice {
                indices: vec![0],
                custom: None,
            },
        ),
        text_change(&session, "password", "masked-🔑-value"),
        text_change(&session, "comb", "9052"),
    ];
    let result = session.fill_form_fields(changes).unwrap();
    assert_eq!(result.revision, 1);
    assert!(result.dirty && result.can_undo && !result.can_redo);
    let output = Document::load_mem(session.bytes()).unwrap();
    assert_eq!(
        decode_text_string(
            output
                .get_dictionary(ids["country"])
                .unwrap()
                .get(b"V")
                .unwrap()
        )
        .unwrap(),
        "US"
    );
    assert_eq!(
        output
            .get_dictionary(ids["services"])
            .unwrap()
            .get(b"I")
            .unwrap()
            .as_array()
            .unwrap(),
        &[Object::Integer(0), Object::Integer(2)]
    );
    for id in &widgets["person.name"] {
        let stream = normal_stream(&output, *id);
        let font = stream
            .dict
            .get(b"Resources")
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Font")
            .unwrap()
            .as_dict()
            .unwrap()
            .iter()
            .next()
            .unwrap()
            .1;
        assert_eq!(
            output
                .dereference(font)
                .unwrap()
                .1
                .as_dict()
                .unwrap()
                .get(b"Subtype")
                .unwrap()
                .as_name()
                .unwrap(),
            b"Type0"
        );
        let content = stream.get_plain_content().unwrap();
        assert!(String::from_utf8_lossy(&content).contains("Tj"));
        assert!(stream.dict.has(b"BBox") && stream.dict.has(b"Matrix"));
    }
    let password = normal_stream(&output, ids["password"])
        .get_plain_content()
        .unwrap();
    assert!(!String::from_utf8_lossy(&password).contains("masked"));
    assert_eq!(
        output
            .get_dictionary(ids["approved"])
            .unwrap()
            .get(b"V")
            .unwrap()
            .as_name()
            .unwrap(),
        b"Accepted"
    );
    assert_eq!(
        output
            .get_dictionary(ids["approved"])
            .unwrap()
            .get(b"AS")
            .unwrap()
            .as_name()
            .unwrap(),
        b"Accepted"
    );
    assert_eq!(
        output
            .get_dictionary(widgets["shipping"][0])
            .unwrap()
            .get(b"AS")
            .unwrap()
            .as_name()
            .unwrap(),
        b"Off"
    );
    assert_eq!(
        output
            .get_dictionary(widgets["shipping"][1])
            .unwrap()
            .get(b"AS")
            .unwrap()
            .as_name()
            .unwrap(),
        b"Express"
    );
    let acroform = output
        .dereference(output.catalog().unwrap().get(b"AcroForm").unwrap())
        .unwrap()
        .1
        .as_dict()
        .unwrap();
    assert!(acroform.get(b"NeedAppearances").unwrap().as_bool().unwrap());
    let reopened =
        Session::from_bytes(session.bytes().to_vec(), "Reopened.pdf".into(), None, 2).unwrap();
    assert_eq!(
        field(&reopened, "person.name").value,
        Some(FieldValue::Text {
            text: "Zoë Martin".into()
        })
    );
    assert_eq!(
        field(&reopened, "person.notes").value,
        Some(FieldValue::Text {
            text: "First line\nΔεύτερη γραμμή".into()
        })
    );
    assert_eq!(
        field(&reopened, "password").value,
        Some(FieldValue::Text {
            text: "masked-🔑-value".into()
        })
    );
    assert_eq!(
        field(&reopened, "services").value,
        Some(FieldValue::Choice {
            indices: vec![0, 2],
            custom: None
        })
    );
    assert_eq!(session.undo().unwrap().revision, 0);
    assert_eq!(session.bytes(), original);
    assert!(!session.info().unwrap().can_undo);
    assert_eq!(session.redo().unwrap().revision, 1);
    assert_eq!(
        field(&session, "comb").value,
        Some(FieldValue::Text {
            text: "9052".into()
        })
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("Filled.pdf");
    session.save_as(&path).unwrap();
    assert!(!session.info().unwrap().dirty);
    let disk = Session::from_path(path, 3).unwrap();
    assert_eq!(
        field(&disk, "person.name").value,
        field(&session, "person.name").value
    );
    if let Some(directory) = std::env::var_os("FOLIO_FORM_FIXTURE_DIR") {
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            std::path::Path::new(&directory).join("original-acroform.pdf"),
            original,
        )
        .unwrap();
        std::fs::write(
            std::path::Path::new(&directory).join("filled-acroform.pdf"),
            session.bytes(),
        )
        .unwrap();
    }
}

#[test]
fn repeated_checkboxes_update_every_widget_without_changing_exports() {
    let mut fixture = fixture();
    let old = fixture.fields["approved"];
    let second_page = fixture.document.get_pages()[&2];
    let parent = fixture
        .document
        .add_object(dictionary! { "T" => text_string("approved"), "FT" => "Btn", "V" => "Off" });
    let additional = widget(
        &mut fixture.document,
        second_page,
        [90.0, 540.0, 110.0, 560.0],
        Some("Accepted"),
    );
    for id in [old, additional] {
        let widget = fixture.document.get_dictionary_mut(id).unwrap();
        widget.remove(b"T");
        widget.remove(b"FT");
        widget.remove(b"V");
        widget.set("Parent", parent);
    }
    fixture.document.get_dictionary_mut(parent).unwrap().set(
        "Kids",
        vec![Object::Reference(old), Object::Reference(additional)],
    );
    let acroform = fixture
        .document
        .catalog()
        .unwrap()
        .get(b"AcroForm")
        .unwrap()
        .as_reference()
        .unwrap();
    let roots = fixture
        .document
        .get_dictionary_mut(acroform)
        .unwrap()
        .get_mut(b"Fields")
        .unwrap()
        .as_array_mut()
        .unwrap();
    *roots
        .iter_mut()
        .find(|value| value.as_reference().ok() == Some(old))
        .unwrap() = parent.into();
    let mut session = session(fixture);
    let state = field(&session, "approved").options[0].id.clone();
    let change = change(
        &session,
        "approved",
        FieldValue::Button { state: Some(state) },
    );
    session.fill_form_fields(vec![change]).unwrap();
    let output = Document::load_mem(session.bytes()).unwrap();
    for id in [old, additional] {
        assert_eq!(
            output
                .get_dictionary(id)
                .unwrap()
                .get(b"AS")
                .unwrap()
                .as_name()
                .unwrap(),
            b"Accepted"
        );
    }
}

#[test]
fn radio_unison_requires_matching_export_states_and_updates_all_occurrences() {
    for unison in [false, true] {
        let mut fixture = fixture();
        let radio = fixture.fields["shipping"];
        let second = fixture.widgets["shipping"][1];
        let on = appearance(&mut fixture.document, true);
        let off = appearance(&mut fixture.document, false);
        fixture.document.get_dictionary_mut(second).unwrap().set(
            "AP",
            dictionary! { "N" => dictionary! { "Off" => off, "Ground" => on } },
        );
        fixture.document.get_dictionary_mut(radio).unwrap().set(
            "Opt",
            vec![text_string("shared-export"), text_string("shared-export")],
        );
        if unison {
            fixture
                .document
                .get_dictionary_mut(radio)
                .unwrap()
                .set("Ff", (1 << 15) | (1 << 25));
        }
        let widgets = fixture.widgets["shipping"].clone();
        let mut session = session(fixture);
        let field = field(&session, "shipping");
        assert_eq!(field.read_only, !unison);
        if unison {
            assert_eq!(field.options.len(), 1);
            assert_eq!(field.options[0].export_value, "shared-export");
            let change = change(
                &session,
                "shipping",
                FieldValue::Button {
                    state: Some(field.options[0].id.clone()),
                },
            );
            session.fill_form_fields(vec![change]).unwrap();
            let output = Document::load_mem(session.bytes()).unwrap();
            for widget in widgets {
                assert_eq!(
                    output
                        .get_dictionary(widget)
                        .unwrap()
                        .get(b"AS")
                        .unwrap()
                        .as_name()
                        .unwrap(),
                    b"Ground"
                );
            }
        }
    }
}

#[test]
fn inherited_read_only_flags_duplicate_names_and_nonportable_widget_styles_stay_explicit() {
    let mut source = fixture();
    let parent = source
        .document
        .get_dictionary(source.fields["person.name"])
        .unwrap()
        .get(b"Parent")
        .unwrap()
        .as_reference()
        .unwrap();
    source
        .document
        .get_dictionary_mut(parent)
        .unwrap()
        .set("Ff", 1);
    source
        .document
        .get_dictionary_mut(source.fields["country"])
        .unwrap()
        .set("BS", dictionary! { "W" => 1, "S" => "B" });
    let working = session(source);
    assert!(field(&working, "person.name").read_only);
    assert!(!field(&working, "person.notes").read_only);
    assert!(
        field(&working, "country")
            .reason
            .unwrap()
            .contains("Beveled")
    );

    let mut duplicate = fixture();
    let page = duplicate.document.get_pages()[&2];
    let extra = widget(
        &mut duplicate.document,
        page,
        [45.0, 600.0, 300.0, 630.0],
        None,
    );
    duplicate.document.get_dictionary_mut(extra).unwrap().extend(&dictionary! {
        "T" => text_string("country"), "FT" => "Tx", "V" => text_string("Different object, same qualified name"),
    });
    let acroform = duplicate
        .document
        .catalog()
        .unwrap()
        .get(b"AcroForm")
        .unwrap()
        .as_reference()
        .unwrap();
    duplicate
        .document
        .get_dictionary_mut(acroform)
        .unwrap()
        .get_mut(b"Fields")
        .unwrap()
        .as_array_mut()
        .unwrap()
        .push(extra.into());
    let duplicate = session(duplicate).form_fields().unwrap();
    let fields = duplicate
        .fields
        .iter()
        .filter(|field| field.name == "country")
        .collect::<Vec<_>>();
    assert_eq!(fields.len(), 2);
    assert!(fields.iter().all(|field| {
        field.read_only
            && field
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("qualified name"))
    }));
}

#[test]
fn clearing_inherited_choice_values_does_not_restore_an_ancestor_value() {
    let mut fixture = fixture();
    let country = fixture.fields["country"];
    let attributes = fixture.document.get_dictionary(country).unwrap().clone();
    let parent = fixture.document.add_object(dictionary! {
        "T" => text_string("region"), "FT" => "Ch", "Ff" => (1 << 17) | (1 << 18),
        "Opt" => attributes.get(b"Opt").unwrap().clone(), "V" => text_string("CA"),
        "Kids" => vec![Object::Reference(country)],
    });
    let page = fixture.document.get_pages()[&2];
    let sibling = widget(
        &mut fixture.document,
        page,
        [45.0, 305.0, 300.0, 335.0],
        None,
    );
    fixture
        .document
        .get_dictionary_mut(sibling)
        .unwrap()
        .extend(&dictionary! {
            "T" => text_string("sibling"), "Parent" => parent,
        });
    fixture.document.get_dictionary_mut(parent).unwrap().set(
        "Kids",
        vec![Object::Reference(country), Object::Reference(sibling)],
    );
    let child = fixture.document.get_dictionary_mut(country).unwrap();
    for name in [b"FT".as_slice(), b"Ff", b"Opt", b"V", b"I"] {
        child.remove(name);
    }
    child.set("Parent", parent);
    child.set("DV", text_string("CA"));
    let acroform = fixture
        .document
        .catalog()
        .unwrap()
        .get(b"AcroForm")
        .unwrap()
        .as_reference()
        .unwrap();
    let roots = fixture
        .document
        .get_dictionary_mut(acroform)
        .unwrap()
        .get_mut(b"Fields")
        .unwrap()
        .as_array_mut()
        .unwrap();
    *roots
        .iter_mut()
        .find(|value| value.as_reference().ok() == Some(country))
        .unwrap() = parent.into();
    let mut session = session(fixture);
    let change = change(
        &session,
        "region.country",
        FieldValue::Choice {
            indices: vec![],
            custom: None,
        },
    );
    session.fill_form_fields(vec![change]).unwrap();
    assert_eq!(
        field(&session, "region.country").value,
        Some(FieldValue::Choice {
            indices: vec![],
            custom: None
        })
    );
    assert_eq!(
        field(&session, "region.sibling").value,
        Some(FieldValue::Choice {
            indices: vec![1],
            custom: None
        })
    );
    let reopened = Document::load_mem(session.bytes()).unwrap();
    assert!(!reopened.get_dictionary(parent).unwrap().has(b"V"));
    assert_eq!(
        decode_text_string(
            reopened
                .get_dictionary(country)
                .unwrap()
                .get(b"DV")
                .unwrap()
        )
        .unwrap(),
        "CA"
    );
    if let Some(directory) = std::env::var_os("FOLIO_FORM_FIXTURE_DIR") {
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            std::path::Path::new(&directory).join("inherited-choice-cleared.pdf"),
            session.bytes(),
        )
        .unwrap();
    }
}

#[test]
fn existing_null_values_inherit_and_existing_false_appearance_flags_stay_false() {
    let mut fixture = fixture();
    let name = fixture.fields["person.name"];
    let widget = fixture.widgets["person.name"][0];
    fixture
        .document
        .get_dictionary_mut(name)
        .unwrap()
        .set("V", Object::Null);
    let acroform = fixture
        .document
        .catalog()
        .unwrap()
        .get(b"AcroForm")
        .unwrap()
        .as_reference()
        .unwrap();
    fixture
        .document
        .get_dictionary_mut(acroform)
        .unwrap()
        .set("NeedAppearances", false);
    let mut session = session(fixture);
    assert_eq!(
        field(&session, "person.name").value,
        Some(FieldValue::Text {
            text: "Inherited".into()
        })
    );
    let change = text_change(&session, "person.name", "Portable text");
    session.fill_form_fields(vec![change]).unwrap();
    let output = Document::load_mem(session.bytes()).unwrap();
    assert!(
        !output
            .get_dictionary(acroform)
            .unwrap()
            .get(b"NeedAppearances")
            .unwrap()
            .as_bool()
            .unwrap()
    );
    normal_stream(&output, widget);
}

#[test]
fn generated_font_resources_do_not_replace_an_existing_form_font_name() {
    let mut fixture = fixture();
    let acroform = fixture
        .document
        .catalog()
        .unwrap()
        .get(b"AcroForm")
        .unwrap()
        .as_reference()
        .unwrap();
    let fonts = fixture
        .document
        .get_dictionary_mut(acroform)
        .unwrap()
        .get_mut(b"DR")
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .get_mut(b"Font")
        .unwrap()
        .as_dict_mut()
        .unwrap();
    let original = fonts.get(b"Helv").unwrap().clone();
    for number in 0..512 {
        fonts.set(format!("FolioForm{number}").as_bytes(), original.clone());
    }
    let mut session = session(fixture);
    let change = text_change(&session, "person.name", "No collision");
    session.fill_form_fields(vec![change]).unwrap();
    let output = Document::load_mem(session.bytes()).unwrap();
    let fonts = output
        .get_dictionary(acroform)
        .unwrap()
        .get(b"DR")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Font")
        .unwrap()
        .as_dict()
        .unwrap();
    assert_eq!(fonts.len(), 514);
    for number in 0..512 {
        assert_eq!(
            fonts.get(format!("FolioForm{number}").as_bytes()).unwrap(),
            &original
        );
    }
}

#[test]
fn duplicate_choice_exports_use_indices_and_fixed_size_overflow_never_commits() {
    let mut fixture = fixture();
    let country = fixture.fields["country"];
    fixture.document.get_dictionary_mut(country).unwrap().set(
        "Opt",
        vec![
            Object::Array(vec![text_string("same"), text_string("First label")]),
            Object::Array(vec![text_string("same"), text_string("Second label")]),
        ],
    );
    fixture
        .document
        .get_dictionary_mut(country)
        .unwrap()
        .set("V", text_string("same"));
    fixture
        .document
        .get_dictionary_mut(country)
        .unwrap()
        .set("I", vec![1.into()]);
    fixture
        .document
        .get_dictionary_mut(fixture.fields["person.name"])
        .unwrap()
        .set("DA", Object::string_literal("/Helv 72 Tf 0 g"));
    let mut session = session(fixture);
    assert_eq!(
        field(&session, "country").value,
        Some(FieldValue::Choice {
            indices: vec![1],
            custom: None
        })
    );
    let change = change(
        &session,
        "country",
        FieldValue::Choice {
            indices: vec![0],
            custom: None,
        },
    );
    session.fill_form_fields(vec![change]).unwrap();
    assert_eq!(
        field(&session, "country").value,
        Some(FieldValue::Choice {
            indices: vec![0],
            custom: None
        })
    );
    let original = session.bytes().to_vec();
    let change = text_change(&session, "person.name", "Cannot fit");
    assert!(
        session
            .fill_form_fields(vec![change])
            .unwrap_err()
            .to_string()
            .contains("fit")
    );
    assert_eq!(session.bytes(), original);
    assert_eq!(session.info().unwrap().revision, 1);
}

#[test]
fn editable_combo_export_matches_use_the_corresponding_display_appearance() {
    let mut session = session(fixture());
    let change = change(
        &session,
        "country",
        FieldValue::Choice {
            indices: vec![],
            custom: Some("US".into()),
        },
    );
    session.fill_form_fields(vec![change]).unwrap();
    assert_eq!(
        field(&session, "country").value,
        Some(FieldValue::Choice {
            indices: vec![0],
            custom: None
        })
    );
    let output = Document::load_mem(session.bytes()).unwrap();
    let country = field(&session, "country")
        .id
        .split(':')
        .next()
        .unwrap()
        .parse::<u32>()
        .unwrap();
    let dict = output.get_dictionary((country, 0)).unwrap();
    assert_eq!(
        dict.get(b"I").unwrap().as_array().unwrap(),
        &[Object::Integer(0)]
    );
    assert_eq!(decode_text_string(dict.get(b"V").unwrap()).unwrap(), "US");
}

#[test]
fn editable_combos_clearable_choices_rotation_and_comb_constraints_are_real() {
    let mut fixture = fixture();
    fixture
        .document
        .get_dictionary_mut(fixture.fields["country"])
        .unwrap()
        .set("MK", dictionary! { "R" => 180 });
    let mut session = session(fixture);
    let changes = vec![
        change(
            &session,
            "country",
            FieldValue::Choice {
                indices: vec![],
                custom: Some("Custom country".into()),
            },
        ),
        change(
            &session,
            "services",
            FieldValue::Choice {
                indices: vec![],
                custom: None,
            },
        ),
    ];
    session.fill_form_fields(changes).unwrap();
    assert_eq!(
        field(&session, "country").value,
        Some(FieldValue::Choice {
            indices: vec![],
            custom: Some("Custom country".into())
        })
    );
    assert_eq!(
        field(&session, "services").value,
        Some(FieldValue::Choice {
            indices: vec![],
            custom: None
        })
    );
    let output = Document::load_mem(session.bytes()).unwrap();
    let id = field(&session, "country")
        .id
        .split(':')
        .next()
        .unwrap()
        .parse::<u32>()
        .unwrap();
    let matrix = normal_stream(&output, (id, 0))
        .dict
        .get(b"Matrix")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(
        matrix
            .iter()
            .map(|value| value.as_float().unwrap())
            .collect::<Vec<_>>(),
        [-1.0, 0.0, 0.0, -1.0, 255.0, 30.0]
    );
}

#[test]
fn invalid_requests_are_atomic_and_leave_bytes_history_and_values_untouched() {
    let mut session = session(fixture());
    let original = session.bytes().to_vec();
    let invalid = vec![
        text_change(&session, "person.name", ""),
        text_change(&session, "person.name", &"x".repeat(41)),
        text_change(&session, "person.name", "two\nlines"),
        text_change(&session, "person.name", "unsupported 漢"),
        text_change(&session, "locked", "changed"),
        text_change(&session, "comb", "12345"),
        text_change(&session, "signature", "not a signature"),
        change(&session, "shipping", FieldValue::Button { state: None }),
        change(
            &session,
            "approved",
            FieldValue::Button {
                state: Some("not-an-export".into()),
            },
        ),
        change(
            &session,
            "services",
            FieldValue::Choice {
                indices: vec![0, 0],
                custom: None,
            },
        ),
        change(
            &session,
            "services",
            FieldValue::Choice {
                indices: vec![0, 3],
                custom: None,
            },
        ),
        change(
            &session,
            "services",
            FieldValue::Choice {
                indices: vec![],
                custom: Some("not a combo".into()),
            },
        ),
        change(
            &session,
            "country",
            FieldValue::Choice {
                indices: vec![0, 1],
                custom: None,
            },
        ),
        FieldChange {
            id: "999999:0".into(),
            value: FieldValue::Text {
                text: "missing".into(),
            },
        },
    ];
    for invalid in invalid {
        let valid = text_change(
            &session,
            "person.notes",
            "First change must not partially commit",
        );
        assert!(session.fill_form_fields(vec![valid, invalid]).is_err());
        assert_eq!(session.bytes(), original);
        let info = session.info().unwrap();
        assert_eq!(info.revision, 0);
        assert!(!info.dirty && !info.can_undo && !info.can_redo);
    }
    let duplicate = vec![
        text_change(&session, "person.name", "One"),
        text_change(&session, "person.name", "Two"),
    ];
    assert!(session.fill_form_fields(duplicate).is_err());
    assert!(session.fill_form_fields(vec![]).is_err());
    assert_eq!(session.bytes(), original);
}

fn encrypted(mut document: Document, version: u8, permissions: Permissions) -> Vec<u8> {
    document.trailer.set(
        "ID",
        vec![
            Object::string_literal("original-synthetic-form-id"),
            Object::string_literal("original-synthetic-form-id"),
        ],
    );
    let method = match version {
        1 => EncryptionVersion::V1 {
            document: &document,
            owner_password: "owner",
            user_password: "reader",
            permissions,
        },
        4 => {
            let filter: Arc<dyn CryptFilter> = Arc::new(Aes128CryptFilter);
            EncryptionVersion::V4 {
                document: &document,
                owner_password: "owner",
                user_password: "reader",
                permissions,
                encrypt_metadata: true,
                crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
                stream_filter: b"StdCF".to_vec(),
                string_filter: b"StdCF".to_vec(),
            }
        }
        5 => {
            let filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
            EncryptionVersion::V5 {
                owner_password: "owner",
                user_password: "reader",
                permissions,
                encrypt_metadata: true,
                file_encryption_key: &[42; 32],
                crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
                stream_filter: b"StdCF".to_vec(),
                string_filter: b"StdCF".to_vec(),
            }
        }
        _ => unreachable!(),
    };
    document
        .encrypt(&EncryptionState::try_from(method).unwrap())
        .unwrap();
    bytes(document)
}

#[test]
fn form_permission_does_not_enable_general_edits_and_encryption_survives_saves_and_history() {
    for version in [1, 4, 5] {
        let permissions = if version == 1 {
            Permissions::ANNOTABLE
        } else {
            Permissions::FILLABLE
        };
        let original = encrypted(fixture().document, version, permissions);
        if version == 4
            && let Some(directory) = std::env::var_os("FOLIO_FORM_FIXTURE_DIR")
        {
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                std::path::Path::new(&directory).join("fillable-protected-acroform.pdf"),
                &original,
            )
            .unwrap();
        }
        let initial =
            Document::load_mem_with_options(&original, LoadOptions::with_password("reader"))
                .unwrap();
        let mut session = Session::from_bytes_with_password(
            original.clone(),
            "Restricted forms.pdf".into(),
            None,
            1,
            Some("reader"),
        )
        .unwrap();
        assert!(session.info().unwrap().read_only_reason.is_some());
        assert!(!session.info().unwrap().copy_allowed);
        assert!(session.form_fields().unwrap().fillable);
        assert!(
            session
                .apply(Edit::Rotate {
                    page: 0,
                    degrees: 90
                })
                .is_err()
        );
        assert!(session.extract_pages(&[0]).is_err());
        let change = text_change(&session, "person.name", "Restricted fill");
        session.fill_form_fields(vec![change]).unwrap();
        assert!(Document::load_mem(session.bytes()).unwrap().is_encrypted());
        for password in ["reader", "owner"] {
            let output = Document::load_mem_with_options(
                session.bytes(),
                LoadOptions::with_password(password),
            )
            .unwrap();
            assert_eq!(
                output.encryption_state.as_ref().unwrap().version(),
                i64::from(version)
            );
            assert_eq!(
                output
                    .encryption_state
                    .as_ref()
                    .unwrap()
                    .file_encryption_key(),
                initial
                    .encryption_state
                    .as_ref()
                    .unwrap()
                    .file_encryption_key()
            );
            assert_eq!(
                output.encryption_state.as_ref().unwrap().permissions(),
                initial.encryption_state.as_ref().unwrap().permissions()
            );
            let reopened = Session::from_bytes_with_password(
                session.bytes().to_vec(),
                "Saved forms.pdf".into(),
                None,
                2,
                Some(password),
            )
            .unwrap();
            assert_eq!(
                field(&reopened, "person.name").value,
                Some(FieldValue::Text {
                    text: "Restricted fill".into()
                })
            );
        }
        session.undo().unwrap();
        assert_eq!(session.bytes(), original);
        session.redo().unwrap();
        assert!(session.form_fields().unwrap().fillable);
        assert!(
            session
                .apply(Edit::Note {
                    page: 0,
                    point: [12.0, 12.0],
                    text: "not permitted".into()
                })
                .is_err()
        );
    }
    let original = encrypted(fixture().document, 4, Permissions::PRINTABLE);
    let mut reader = Session::from_bytes_with_password(
        original.clone(),
        "Not fillable.pdf".into(),
        None,
        1,
        Some("reader"),
    )
    .unwrap();
    assert!(!reader.form_fields().unwrap().fillable);
    let change = text_change(&reader, "person.name", "Must not change");
    assert!(reader.fill_form_fields(vec![change]).is_err());
    assert_eq!(reader.bytes(), original);
}

#[test]
fn actions_calculations_xfa_signed_and_ambiguous_fields_are_explicitly_read_only() {
    for boundary in ["XFA", "CO", "signature", "Perms", "JavaScript"] {
        let mut fixture = fixture();
        let acroform = fixture
            .document
            .catalog()
            .unwrap()
            .get(b"AcroForm")
            .unwrap()
            .as_reference()
            .unwrap();
        match boundary {
            "XFA" => fixture.document.get_dictionary_mut(acroform).unwrap().set(
                "XFA",
                Object::string_literal("original synthetic XFA marker"),
            ),
            "CO" => fixture
                .document
                .get_dictionary_mut(acroform)
                .unwrap()
                .set("CO", vec![Object::Reference(fixture.fields["person.name"])]),
            "signature" => {
                let signature = fixture.document.add_object(dictionary! { "Type" => "Sig", "ByteRange" => vec![0.into(), 1.into(), 2.into(), 3.into()], "Contents" => Object::string_literal("synthetic marker, not a valid signature") });
                fixture
                    .document
                    .get_dictionary_mut(fixture.fields["signature"])
                    .unwrap()
                    .set("V", signature);
            }
            "Perms" => fixture
                .document
                .catalog_mut()
                .unwrap()
                .set("Perms", dictionary! {}),
            "JavaScript" => fixture
                .document
                .catalog_mut()
                .unwrap()
                .set("Names", dictionary! { "JavaScript" => dictionary! {} }),
            _ => unreachable!(),
        }
        let mut session = session(fixture);
        let original = session.bytes().to_vec();
        assert!(!session.form_fields().unwrap().fillable, "{boundary}");
        assert!(session.form_fields().unwrap().reason.is_some());
        let change = text_change(&session, "person.name", "Cannot change");
        assert!(
            session.fill_form_fields(vec![change]).is_err(),
            "{boundary}"
        );
        assert_eq!(session.bytes(), original);
    }
    let mut fixture = fixture();
    fixture.document.get_dictionary_mut(fixture.fields["person.name"]).unwrap().set("AA", dictionary! { "V" => dictionary! { "S" => "JavaScript", "JS" => Object::string_literal("throw new Error('never execute')") } });
    let session = session(fixture);
    assert!(field(&session, "person.name").read_only);
    assert!(
        field(&session, "person.name")
            .reason
            .unwrap()
            .contains("scripts")
    );
    assert!(!field(&session, "person.notes").read_only);
}

#[test]
fn malformed_trees_oversized_choices_and_ambiguous_button_appearances_are_bounded() {
    let mut cyclic = fixture();
    let name = cyclic.fields["person.name"];
    cyclic
        .document
        .get_dictionary_mut(name)
        .unwrap()
        .set("Kids", vec![Object::Reference(name)]);
    assert!(session(cyclic).form_fields().is_err());
    let mut oversized = fixture();
    oversized
        .document
        .get_dictionary_mut(oversized.fields["services"])
        .unwrap()
        .set("Opt", vec![text_string("option"); 4097]);
    assert!(session(oversized).form_fields().is_err());
    let mut ambiguous = fixture();
    let approved = ambiguous.fields["approved"];
    ambiguous
        .document
        .get_dictionary_mut(approved)
        .unwrap()
        .set("AP", dictionary! { "N" => dictionary! {} });
    let mut session = session(ambiguous);
    assert!(field(&session, "approved").read_only);
    let change = change(&session, "approved", FieldValue::Button { state: None });
    assert!(session.fill_form_fields(vec![change]).is_err());
}

#[test]
fn form_drafts_and_revision_guards_remain_document_scoped_and_block_original_exports() {
    let mut workspace = Workspace::default();
    let first = workspace
        .open_bytes(bytes(fixture().document), "First form.pdf".into())
        .unwrap();
    let second = workspace
        .open_bytes(bytes(fixture().document), "Second form.pdf".into())
        .unwrap();
    let second_bytes = workspace.document(second.id).unwrap().bytes().to_vec();
    workspace.set_draft(first.id, true).unwrap();
    assert!(
        workspace
            .ensure_export_ready(first.id, first.revision)
            .is_err()
    );
    assert!(
        workspace
            .ensure_export_ready(second.id, second.revision)
            .is_ok()
    );
    assert!(
        workspace
            .close_document(first.id, first.revision, false)
            .is_err()
    );
    assert!(workspace.has_unsaved_changes());
    let change = text_change(
        workspace.document(first.id).unwrap(),
        "person.name",
        "First document only",
    );
    let changed = workspace
        .current_mut(first.id, first.revision)
        .unwrap()
        .fill_form_fields(vec![change])
        .unwrap();
    assert!(workspace.check_revision(first.id, first.revision).is_err());
    assert!(workspace.check_revision(first.id, changed.revision).is_ok());
    assert!(
        workspace
            .ensure_export_ready(first.id, changed.revision)
            .is_err()
    );
    workspace.set_draft(first.id, false).unwrap();
    assert!(
        workspace
            .ensure_export_ready(first.id, changed.revision)
            .is_ok()
    );
    assert_eq!(workspace.document(second.id).unwrap().bytes(), second_bytes);
    assert_eq!(workspace.current().unwrap().id, second.id);
    assert!(
        workspace
            .current_mut(first.id, changed.revision)
            .unwrap()
            .apply(Edit::Delete { page: 0 })
            .is_err()
    );
}
