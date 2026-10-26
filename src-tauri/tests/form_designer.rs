use std::{collections::BTreeMap, path::Path, sync::Arc};

use folio_core::{
    document::{Edit, Session},
    forms::{
        FieldChange, FieldKind, FieldValue, FormField,
        designer::{Change, ControlKind, Definition, OptionDefinition, WidgetPlacement},
    },
    workspace::Workspace,
};
use lopdf::{
    Document, LoadOptions, Object, ObjectId, Stream,
    content::Content,
    decode_text_string, dictionary,
    encryption::{
        EncryptionState, EncryptionVersion, Permissions,
        crypt_filters::{Aes128CryptFilter, Aes256CryptFilter, CryptFilter},
    },
    text_string,
};

fn numbers(values: &[f32]) -> Object {
    Object::Array(values.iter().copied().map(Object::Real).collect())
}

fn bytes(document: &mut Document) -> Vec<u8> {
    let mut bytes = vec![];
    document.save_to(&mut bytes).unwrap();
    bytes
}

fn load(bytes: &[u8]) -> Document {
    Document::load_mem(bytes).unwrap()
}
fn evidence_file(name: &str, bytes: &[u8]) {
    if let Ok(directory) = std::env::var("FOLIO_FORM_EVIDENCE_DIR") {
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(Path::new(&directory).join(name), bytes).unwrap();
    }
}
fn key(id: ObjectId) -> String {
    format!("{}:{}", id.0, id.1)
}
fn object_key(value: &str) -> ObjectId {
    let (a, b) = value.split_once(':').unwrap();
    (a.parse().unwrap(), b.parse().unwrap())
}
fn state(value: &str) -> String {
    value.bytes().map(|byte| format!("{byte:02x}")).collect()
}

fn appearance_text(document: &Document) -> String {
    let mut text = String::new();
    for (number, page) in document.get_pages() {
        let resources = document
            .get_dictionary(page)
            .unwrap()
            .get(b"Resources")
            .unwrap()
            .as_dict()
            .unwrap();
        let xobjects = resources.get(b"XObject").unwrap().as_dict().unwrap();
        for (_, value) in xobjects {
            let id = value.as_reference().unwrap();
            let stream = document.get_object(id).unwrap().as_stream().unwrap();
            let mut isolated = document.clone();
            let target = isolated.get_dictionary_mut(page).unwrap();
            target.set("Contents", id);
            target.set("Resources", stream.dict.get(b"Resources").unwrap().clone());
            text.push_str(&isolated.extract_text(&[number]).unwrap());
        }
    }
    text
}

fn definition(kind: ControlKind, name: &str) -> Definition {
    Definition {
        kind,
        name: name.into(),
        label: format!("Label for {name}"),
        required: false,
        read_only: false,
        multiline: false,
        password: false,
        comb: false,
        max_length: 40,
        multiple: false,
        editable_choice: false,
        no_toggle_to_off: false,
        options: match kind {
            ControlKind::Checkbox => vec![OptionDefinition {
                label: "Yes label".into(),
                export_value: "Yes".into(),
            }],
            ControlKind::Radio | ControlKind::List | ControlKind::Combo => vec![
                OptionDefinition {
                    label: "Alpha label".into(),
                    export_value: "Alpha".into(),
                },
                OptionDefinition {
                    label: "Beta label".into(),
                    export_value: "Beta".into(),
                },
            ],
            _ => vec![],
        },
        default_value: None,
        font_size: 10.0,
        alignment: 0,
    }
}

fn widget(page: usize, x: f32, y: f32, width: f32, height: f32) -> WidgetPlacement {
    WidgetPlacement {
        page,
        placement: [x, y, width, height],
        option: None,
    }
}

fn create(definition: Definition, widgets: Vec<WidgetPlacement>) -> Change {
    Change::Create {
        definition,
        widgets,
    }
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

fn props(session: &Session, name: &str) -> Definition {
    session
        .form_designer()
        .unwrap()
        .fields
        .into_iter()
        .find(|item| item.field.name == name)
        .unwrap()
        .definition
        .unwrap()
}

fn fill_text(session: &mut Session, name: &str, text: &str) {
    let id = field(session, name).id;
    session
        .fill_form_fields(vec![FieldChange {
            id,
            value: FieldValue::Text { text: text.into() },
        }])
        .unwrap();
}

fn geometry_document() -> Document {
    let mut document = load(Session::blank(640.0, 800.0, 1).unwrap().bytes());
    let first = document.get_pages()[&1];
    let parent = document
        .get_dictionary(first)
        .unwrap()
        .get(b"Parent")
        .unwrap()
        .as_reference()
        .unwrap();
    let mut pages = vec![];
    for (index, rotation) in [0, 90, 180, 270].into_iter().enumerate() {
        let mut page = document.get_dictionary(first).unwrap().clone();
        page.set("MediaBox", numbers(&[-80.0, -40.0, 640.0, 800.0]));
        page.set("CropBox", numbers(&[-30.0, 70.0, 390.0, 610.0]));
        page.set("Rotate", rotation);
        page.set("UserUnit", if index % 2 == 0 { 1.5 } else { 2.0 });
        page.remove(b"Annots");
        pages.push(document.add_object(page));
    }
    let parent = document.get_dictionary_mut(parent).unwrap();
    parent.set(
        "Kids",
        pages
            .iter()
            .copied()
            .map(Object::Reference)
            .collect::<Vec<_>>(),
    );
    parent.set("Count", 4);
    document
}

#[test]
fn creates_supported_real_controls_and_unsigned_slot_with_stable_object_keys() {
    let mut session = Session::blank(612.0, 792.0, 1).unwrap();
    let mut text = definition(ControlKind::Text, "name");
    text.required = true;
    text.default_value = Some(FieldValue::Text {
        text: "Zoë".into()
    });
    let mut checkbox = definition(ControlKind::Checkbox, "approved");
    checkbox.default_value = Some(FieldValue::Button {
        state: Some(state("Yes")),
    });
    let mut radio = definition(ControlKind::Radio, "shipping");
    radio.default_value = Some(FieldValue::Button {
        state: Some(state("Beta")),
    });
    let mut combo = definition(ControlKind::Combo, "country");
    combo.editable_choice = true;
    combo.default_value = Some(FieldValue::Choice {
        indices: vec![],
        custom: Some("Custom".into()),
    });
    let mut list = definition(ControlKind::List, "services");
    list.multiple = true;
    list.default_value = Some(FieldValue::Choice {
        indices: vec![0, 1],
        custom: None,
    });
    let mut a = widget(0, 40.0, 180.0, 20.0, 20.0);
    a.option = Some(0);
    let mut b = widget(0, 40.0, 210.0, 20.0, 20.0);
    b.option = Some(1);
    let result = session
        .design_form_fields(vec![
            create(
                text,
                vec![
                    widget(0, 40.0, 40.0, 200.0, 30.0),
                    widget(0, 300.0, 40.0, 200.0, 30.0),
                ],
            ),
            create(checkbox, vec![widget(0, 40.0, 120.0, 20.0, 20.0)]),
            create(radio, vec![a, b]),
            create(combo, vec![widget(0, 40.0, 280.0, 200.0, 30.0)]),
            create(list, vec![widget(0, 40.0, 340.0, 200.0, 100.0)]),
            create(
                definition(ControlKind::Signature, "approval_signature"),
                vec![widget(0, 40.0, 500.0, 220.0, 40.0)],
            ),
        ])
        .unwrap();
    assert!(result.can_undo);
    assert!(
        result.read_only_reason.is_none(),
        "unsigned slots must not lock the PDF"
    );
    let fields = session.form_fields().unwrap();
    assert!(fields.fillable);
    assert_eq!(fields.fields.len(), 6);
    assert_eq!(field(&session, "name").widgets.len(), 2);
    assert_eq!(
        field(&session, "approval_signature").kind,
        FieldKind::Signature
    );
    assert_eq!(
        field(&session, "shipping")
            .options
            .iter()
            .find(|option| option.id == state("Alpha"))
            .unwrap()
            .export_value,
        "Alpha"
    );
    let document = load(session.bytes());
    let signature = document
        .get_dictionary(object_key(&field(&session, "approval_signature").id))
        .unwrap();
    assert!(!signature.has(b"V"));
    assert_eq!(signature.get(b"Ff").unwrap().as_i64().unwrap(), 0);
    let acro = document
        .dereference(document.catalog().unwrap().get(b"AcroForm").unwrap())
        .unwrap()
        .1
        .as_dict()
        .unwrap();
    assert!(!acro.has(b"SigFlags"));
    for field in &fields.fields {
        let id = object_key(&field.id);
        for widget in &field.widgets {
            let annotation = document.get_dictionary(object_key(&widget.id)).unwrap();
            assert_eq!(
                annotation.get(b"Parent").unwrap().as_reference().unwrap(),
                id
            );
            assert_eq!(
                annotation.get(b"P").unwrap().as_reference().unwrap(),
                document.get_pages()[&1]
            );
            assert!(annotation.has(b"AP"));
        }
    }
    let temp = tempfile::tempdir().unwrap();
    session.save_as(&temp.path().join("authored.pdf")).unwrap();
    let reopened = Session::from_path(temp.path().join("authored.pdf"), 2).unwrap();
    assert_eq!(field(&reopened, "name").id, field(&session, "name").id);
    assert_eq!(
        field(&reopened, "name").value,
        Some(FieldValue::Text {
            text: "Zoë".into()
        })
    );
    assert_eq!(
        props(&reopened, "name").default_value,
        field(&reopened, "name").value
    );
    assert!(reopened.info().unwrap().read_only_reason.is_none());
    let before = session.bytes().to_vec();
    session.undo().unwrap();
    assert!(session.form_fields().unwrap().fields.is_empty());
    session.redo().unwrap();
    assert_eq!(session.bytes(), before);
}

#[test]
fn required_blank_creation_is_not_a_filling_permission_bypass_and_batches_roll_back() {
    let mut session = Session::blank(612.0, 792.0, 1).unwrap();
    let mut required = definition(ControlKind::Text, "required");
    required.required = true;
    session
        .design_form_fields(vec![create(
            required,
            vec![widget(0, 20.0, 20.0, 200.0, 30.0)],
        )])
        .unwrap();
    let id = field(&session, "required").id;
    assert!(
        session
            .fill_form_fields(vec![FieldChange {
                id,
                value: FieldValue::Text { text: "".into() }
            }])
            .is_err()
    );
    let before = session.bytes().to_vec();
    let revision = session.revision();
    let mut invalid = definition(ControlKind::Text, "bad");
    invalid.comb = true;
    invalid.multiline = true;
    assert!(
        session
            .design_form_fields(vec![
                create(
                    definition(ControlKind::Text, "would-have-been-created"),
                    vec![widget(0, 30.0, 90.0, 200.0, 30.0)]
                ),
                create(invalid, vec![widget(0, 30.0, 140.0, 200.0, 30.0)]),
            ])
            .is_err()
    );
    assert_eq!(session.bytes(), before);
    assert_eq!(session.revision(), revision);
    assert_eq!(session.form_fields().unwrap().fields.len(), 1);
    assert!(
        session
            .design_form_fields(vec![create(
                definition(ControlKind::Text, "required"),
                vec![widget(0, 30.0, 190.0, 200.0, 30.0)]
            )])
            .is_err()
    );
    assert_eq!(session.bytes(), before);
}

#[test]
fn geometry_uses_visible_rotated_physical_points_and_moves_only_the_target_widget() {
    let mut document = geometry_document();
    let mut session =
        Session::from_bytes(bytes(&mut document), "Geometry.pdf".into(), None, 1).unwrap();
    let placements = (0..4)
        .map(|page| widget(page, 30.0, 40.0, 120.0, 24.0))
        .collect();
    let mut definition = definition(ControlKind::Text, "repeated");
    definition.font_size = 0.0;
    session
        .design_form_fields(vec![create(definition, placements)])
        .unwrap();
    let fields = field(&session, "repeated");
    let expected = [
        [-10.0, 610.0 - 64.0 / 1.5, 70.0, 610.0 - 40.0 / 1.5],
        [-10.0, 85.0, 2.0, 145.0],
        [290.0, 70.0 + 40.0 / 1.5, 370.0, 70.0 + 64.0 / 1.5],
        [358.0, 535.0, 370.0, 595.0],
    ];
    for (widget, expected) in fields.widgets.iter().zip(expected) {
        for (actual, expected) in widget.rect.iter().zip(expected) {
            assert!((actual - expected).abs() < 0.001, "{actual} != {expected}");
        }
    }
    fill_text(&mut session, "repeated", "Shared");
    let before = field(&session, "repeated");
    session
        .design_form_fields(vec![Change::PlaceWidget {
            id: before.id.clone(),
            widget_id: before.widgets[0].id.clone(),
            widget: widget(3, 100.0, 120.0, 160.0, 40.0),
        }])
        .unwrap();
    let after = field(&session, "repeated");
    assert_eq!(after.id, before.id);
    assert_eq!(after.widgets[0].id, before.widgets[0].id);
    assert_eq!(after.widgets[0].page, 3);
    assert_eq!(after.widgets[1].rect, before.widgets[1].rect);
    assert_eq!(after.value, before.value);
    let output = load(session.bytes());
    let first_annots = output
        .get_dictionary(output.get_pages()[&1])
        .unwrap()
        .get(b"Annots")
        .unwrap()
        .as_array()
        .unwrap();
    assert!(
        first_annots
            .iter()
            .all(|value| value.as_reference().unwrap() != object_key(&before.widgets[0].id))
    );
    let unchanged = session.bytes().to_vec();
    assert!(
        session.apply(Edit::Duplicate { page: 0 }).is_err(),
        "legacy form page guards remain"
    );
    assert_eq!(session.bytes(), unchanged);
}

fn inherited_fixture() -> Document {
    let mut document = load(Session::blank(612.0, 792.0, 1).unwrap().bytes());
    let page = document.get_pages()[&1];
    let parent = document.add_object(dictionary! {
        "T" => text_string("person"), "FT" => "Tx", "V" => text_string("Inherited"),
        "DV" => text_string("Parent default"), "Ff" => 0, "MaxLen" => 40,
        "DA" => Object::string_literal("/Helv 10 Tf 0 g"),
    });
    let first =
        document.add_object(dictionary! { "T" => text_string("first"), "Parent" => parent });
    let repeated = [0, 1].into_iter().map(|index| document.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "Parent" => first, "P" => page,
        "Rect" => numbers(&[40.0, 650.0 - index as f32 * 50.0, 260.0, 680.0 - index as f32 * 50.0]), "F" => 4,
    })).collect::<Vec<_>>();
    document.get_dictionary_mut(first).unwrap().set(
        "Kids",
        repeated
            .iter()
            .copied()
            .map(Object::Reference)
            .collect::<Vec<_>>(),
    );
    let second = document.add_object(dictionary! {
        "T" => text_string("second"), "Parent" => parent, "Subtype" => "Widget", "Type" => "Annot",
        "P" => page, "Rect" => numbers(&[40.0, 500.0, 260.0, 530.0]), "F" => 4,
    });
    document.get_dictionary_mut(parent).unwrap().set(
        "Kids",
        vec![Object::Reference(first), Object::Reference(second)],
    );
    let mut annotations = repeated
        .iter()
        .copied()
        .map(Object::Reference)
        .collect::<Vec<_>>();
    annotations.push(second.into());
    document
        .get_dictionary_mut(page)
        .unwrap()
        .set("Annots", annotations);
    let font = document.add_object(
        dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" },
    );
    let acroform = document.add_object(dictionary! {
        "Fields" => vec![Object::Reference(parent)],
        "DR" => dictionary! { "Font" => dictionary! { "Helv" => font } }, "NeedAppearances" => true,
    });
    document.catalog_mut().unwrap().set("AcroForm", acroform);
    document
}

#[test]
fn inherited_parent_values_defaults_and_siblings_survive_property_layout_and_removal() {
    let mut document = inherited_fixture();
    let mut session =
        Session::from_bytes(bytes(&mut document), "Inherited.pdf".into(), None, 1).unwrap();
    let mut definition = props(&session, "person.first");
    definition.name = "renamed".into();
    definition.label = "Accessible new label".into();
    definition.default_value = None;
    definition.read_only = true;
    let before = field(&session, "person.first");
    session
        .design_form_fields(vec![Change::Update {
            id: before.id.clone(),
            definition,
        }])
        .unwrap();
    let renamed = field(&session, "person.renamed");
    assert_eq!(renamed.id, before.id);
    assert_eq!(
        renamed.value,
        Some(FieldValue::Text {
            text: "Inherited".into()
        })
    );
    assert!(renamed.read_only);
    assert!(props(&session, "person.renamed").default_value.is_none());
    assert_eq!(
        props(&session, "person.second").default_value,
        Some(FieldValue::Text {
            text: "Parent default".into()
        })
    );
    assert_eq!(field(&session, "person.second").value, before.value);
    session
        .design_form_fields(vec![Change::RemoveWidget {
            id: renamed.id.clone(),
            widget_id: renamed.widgets[0].id.clone(),
        }])
        .unwrap();
    assert_eq!(field(&session, "person.renamed").widgets.len(), 1);
    assert_eq!(field(&session, "person.renamed").value, before.value);
    session
        .design_form_fields(vec![Change::RemoveField { id: renamed.id }])
        .unwrap();
    assert_eq!(session.form_fields().unwrap().fields.len(), 1);
    assert_eq!(field(&session, "person.second").name, "person.second");
    let sibling = field(&session, "person.second");
    session
        .design_form_fields(vec![Change::RemoveWidget {
            id: sibling.id,
            widget_id: sibling.widgets[0].id.clone(),
        }])
        .unwrap();
    assert!(session.form_fields().unwrap().fields.is_empty());
    session.undo().unwrap();
    assert_eq!(field(&session, "person.second").value, before.value);
    session.redo().unwrap();
    assert!(session.form_fields().unwrap().fields.is_empty());
}

#[test]
fn updating_options_preserves_current_exports_and_rejects_destructive_constraints() {
    let mut session = Session::blank(612.0, 792.0, 1).unwrap();
    let mut choice = definition(ControlKind::Combo, "choices");
    choice.default_value = Some(FieldValue::Choice {
        indices: vec![1],
        custom: None,
    });
    session
        .design_form_fields(vec![create(
            choice,
            vec![widget(0, 40.0, 40.0, 220.0, 32.0)],
        )])
        .unwrap();
    let mut definition = props(&session, "choices");
    definition.options.swap(0, 1);
    definition.default_value = Some(FieldValue::Choice {
        indices: vec![0],
        custom: None,
    });
    session
        .design_form_fields(vec![Change::Update {
            id: field(&session, "choices").id,
            definition,
        }])
        .unwrap();
    assert_eq!(
        field(&session, "choices").value,
        Some(FieldValue::Choice {
            indices: vec![0],
            custom: None
        })
    );
    let before = session.bytes().to_vec();
    let mut definition = props(&session, "choices");
    definition.options.remove(0);
    assert!(
        session
            .design_form_fields(vec![Change::Update {
                id: field(&session, "choices").id,
                definition
            }])
            .is_err()
    );
    assert_eq!(session.bytes(), before);
}

#[test]
fn flatten_rebuilds_stale_and_missing_appearances_masks_passwords_and_preserves_source_history() {
    let mut document = inherited_fixture();
    let mut session =
        Session::from_bytes(bytes(&mut document), "Stale.pdf".into(), None, 1).unwrap();
    let mut password = definition(ControlKind::Text, "password");
    password.password = true;
    password.default_value = Some(FieldValue::Text {
        text: "not-for-static-copy".into(),
    });
    let mut locked = definition(ControlKind::Text, "locked");
    locked.read_only = true;
    locked.default_value = Some(FieldValue::Text {
        text: "Visible read-only".into(),
    });
    session
        .design_form_fields(vec![
            create(password, vec![widget(0, 40.0, 340.0, 220.0, 32.0)]),
            create(locked, vec![widget(0, 40.0, 400.0, 220.0, 32.0)]),
            create(
                definition(ControlKind::Signature, "unsigned"),
                vec![widget(0, 40.0, 450.0, 220.0, 32.0)],
            ),
        ])
        .unwrap();
    fill_text(&mut session, "person.first", "Current value");
    let before = session.bytes().to_vec();
    let info = session.info().unwrap();
    let plan = session.flattened_form_info().unwrap();
    assert_eq!(plan.unsigned_signature_count, 1);
    let flat = session.flattened_form_copy().unwrap();
    evidence_file("stale-source.pdf", session.bytes());
    evidence_file("stale-static.pdf", &flat);
    let output = load(&flat);
    assert!(!output.catalog().unwrap().has(b"AcroForm"));
    assert!(
        output
            .objects
            .values()
            .all(|object| object.as_dict().map_or(true, |dict| dict
                .get(b"Subtype")
                .and_then(Object::as_name)
                .ok()
                != Some(b"Widget")))
    );
    let text = appearance_text(&output);
    assert!(text.contains("Current value"));
    assert!(text.contains("Inherited"));
    assert!(text.contains("Visible read-only"));
    assert!(text.contains("*******************"));
    assert!(!text.contains("not-for-static-copy"));
    for object in output.objects.values() {
        if let Ok(dict) = object.as_dict() {
            assert!(!dict.has(b"FT"));
            assert!(!dict.has(b"DV"));
        }
    }
    assert_eq!(session.bytes(), before);
    assert_eq!(session.info().unwrap().revision, info.revision);
    assert_eq!(session.info().unwrap().can_undo, info.can_undo);
    session.undo().unwrap();
    session.redo().unwrap();
    assert_eq!(session.bytes(), before);
    let reopened = Session::from_bytes(flat, "Static.pdf".into(), None, 2).unwrap();
    assert!(reopened.form_fields().unwrap().fields.is_empty());
}

#[test]
fn flatten_resolves_button_value_over_stale_state_and_preserves_bbox_matrix_and_inherited_resources()
 {
    let mut session = Session::blank(612.0, 792.0, 1).unwrap();
    let mut checkbox = definition(ControlKind::Checkbox, "approved");
    checkbox.default_value = Some(FieldValue::Button {
        state: Some(state("Yes")),
    });
    session
        .design_form_fields(vec![create(
            checkbox,
            vec![widget(0, 40.0, 80.0, 40.0, 20.0)],
        )])
        .unwrap();
    let field = field(&session, "approved");
    let mut document = load(session.bytes());
    let widget = object_key(&field.widgets[0].id);
    let font = document.add_object(
        dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" },
    );
    let ap = document.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => numbers(&[10.0, 20.0, 30.0, 60.0]),
            "Matrix" => numbers(&[0.0, 1.0, -1.0, 0.0, 70.0, -5.0]),
        },
        b"q BT /Inherited 3 Tf 12 30 Td (ON VALUE) Tj ET Q".to_vec(),
    ));
    let off = document.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => numbers(&[0.0, 0.0, 20.0, 20.0]),
        },
        b"q 0 G 1 1 18 18 re S Q".to_vec(),
    ));
    document.get_dictionary_mut(widget).unwrap().set(
        "AP",
        dictionary! { "N" => dictionary! { "Off" => off, "Yes" => ap } },
    );
    document
        .get_dictionary_mut(widget)
        .unwrap()
        .set("AS", "Off");
    let form = document
        .catalog()
        .unwrap()
        .get(b"AcroForm")
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    let mut form = form;
    form.set(
        "DR",
        dictionary! { "Font" => dictionary! { "Inherited" => font } },
    );
    document.catalog_mut().unwrap().set("AcroForm", form);
    let session =
        Session::from_bytes(bytes(&mut document), "Transforms.pdf".into(), None, 1).unwrap();
    let flat = session.flattened_form_copy().unwrap();
    evidence_file("button-transform-source.pdf", session.bytes());
    evidence_file("button-transform-static.pdf", &flat);
    let output = load(&flat);
    let page = output.get_pages()[&1];
    let content = Content::decode(&output.get_page_content(page)).unwrap();
    assert!(
        !content
            .operations
            .iter()
            .any(|operation| matches!(operation.operator.as_str(), "W" | "W*")),
        "The fitted Form BBox must clip once, without an additional fractional-edge Rect clip"
    );
    let resources = output
        .get_dictionary(page)
        .unwrap()
        .get(b"Resources")
        .unwrap()
        .as_dict()
        .unwrap();
    let appearance = resources
        .get(b"XObject")
        .unwrap()
        .as_dict()
        .unwrap()
        .iter()
        .next()
        .unwrap()
        .1;
    let appearance = output
        .dereference(appearance)
        .unwrap()
        .1
        .as_stream()
        .unwrap();
    let bounds = appearance
        .dict
        .get(b"BBox")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_float().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(bounds, field.widgets[0].rect);
    let content = Content::decode(&appearance.get_plain_content().unwrap()).unwrap();
    let matrix = &content
        .operations
        .iter()
        .find(|operation| operation.operator == "cm")
        .unwrap()
        .operands;
    let values = matrix
        .iter()
        .map(|value| value.as_float().unwrap())
        .collect::<Vec<_>>();
    let rect = field.widgets[0].rect;
    assert_eq!(
        values,
        vec![1.0, 0.0, 0.0, 1.0, rect[0] - 10.0, rect[1] - 5.0]
    );
    let text = appearance_text(&output);
    assert!(text.contains("ON VALUE"), "{text}");
    let original_matrix = content
        .operations
        .iter()
        .filter(|operation| operation.operator == "cm")
        .nth(1)
        .unwrap();
    assert_eq!(
        original_matrix
            .operands
            .iter()
            .map(|value| value.as_float().unwrap())
            .collect::<Vec<_>>(),
        vec![0.0, 1.0, -1.0, 0.0, 70.0, -5.0]
    );
}

fn encrypted(mut document: Document, aes256: bool, permissions: Permissions) -> Vec<u8> {
    document.trailer.set(
        "ID",
        vec![Object::string_literal("original-synthetic-id"); 2],
    );
    let version = if aes256 {
        let filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
        EncryptionVersion::V5 {
            owner_password: "owner",
            user_password: "user",
            permissions,
            encrypt_metadata: true,
            file_encryption_key: &[37; 32],
            crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
            stream_filter: b"StdCF".to_vec(),
            string_filter: b"StdCF".to_vec(),
        }
    } else {
        let filter: Arc<dyn CryptFilter> = Arc::new(Aes128CryptFilter);
        EncryptionVersion::V4 {
            document: &document,
            owner_password: "owner",
            user_password: "user",
            permissions,
            encrypt_metadata: true,
            crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
            stream_filter: b"StdCF".to_vec(),
            string_filter: b"StdCF".to_vec(),
        }
    };
    let state = EncryptionState::try_from(version).unwrap();
    document.encrypt(&state).unwrap();
    bytes(&mut document)
}

#[test]
fn filling_only_user_password_cannot_author_or_flatten_and_authorized_saves_history_keep_encryption()
 {
    for aes256 in [false, true] {
        let mut document = inherited_fixture();
        let protected = encrypted(document.clone(), aes256, Permissions::FILLABLE);
        let mut user = Session::from_bytes_with_password(
            protected.clone(),
            "Restricted.pdf".into(),
            None,
            1,
            Some("user"),
        )
        .unwrap();
        fill_text(&mut user, "person.first", "Allowed fill");
        let before = user.bytes().to_vec();
        assert!(!user.form_designer().unwrap().authorable);
        assert!(
            user.design_form_fields(vec![create(
                definition(ControlKind::Text, "unauthorized"),
                vec![widget(0, 10.0, 10.0, 200.0, 30.0)]
            )])
            .is_err()
        );
        assert!(user.flattened_form_copy().is_err());
        assert_eq!(user.bytes(), before);
        let mut owner = Session::from_bytes_with_password(
            protected.clone(),
            "Owner.pdf".into(),
            None,
            2,
            Some("owner"),
        )
        .unwrap();
        owner
            .design_form_fields(vec![create(
                definition(ControlKind::Text, "authorized"),
                vec![widget(0, 40.0, 350.0, 200.0, 30.0)],
            )])
            .unwrap();
        assert!(load(owner.bytes()).is_encrypted());
        let current = owner.bytes().to_vec();
        let flat = owner.flattened_form_copy().unwrap();
        evidence_file(
            if aes256 {
                "aes256-authored.pdf"
            } else {
                "aes128-authored.pdf"
            },
            owner.bytes(),
        );
        evidence_file(
            if aes256 {
                "aes256-static.pdf"
            } else {
                "aes128-static.pdf"
            },
            &flat,
        );
        for password in ["user", "owner"] {
            let flat = Document::load_mem_with_options(&flat, LoadOptions::with_password(password))
                .unwrap();
            let original =
                Document::load_mem_with_options(&protected, LoadOptions::with_password(password))
                    .unwrap();
            assert!(flat.was_encrypted());
            assert!(!flat.catalog().unwrap().has(b"AcroForm"));
            assert_eq!(
                flat.encryption_state
                    .as_ref()
                    .unwrap()
                    .file_encryption_key(),
                original
                    .encryption_state
                    .as_ref()
                    .unwrap()
                    .file_encryption_key()
            );
            assert_eq!(
                flat.trailer.get(b"ID").unwrap(),
                original.trailer.get(b"ID").unwrap()
            );
        }
        owner.undo().unwrap();
        assert_eq!(owner.bytes(), protected);
        owner.redo().unwrap();
        assert_eq!(owner.bytes(), current);
        let dir = tempfile::tempdir().unwrap();
        owner
            .save_as(&dir.path().join("protected-authored.pdf"))
            .unwrap();
        document = Document::load_mem_with_options(
            &std::fs::read(dir.path().join("protected-authored.pdf")).unwrap(),
            LoadOptions::with_password("user"),
        )
        .unwrap();
        assert!(document.was_encrypted());
        assert!(document.objects.values().any(|object| {
            object.as_dict().is_ok_and(|dict| {
                dict.get(b"T").is_ok_and(|value| {
                    decode_text_string(value).ok().as_deref() == Some("authorized")
                })
            })
        }));
    }
}

#[test]
fn signed_certified_hidden_scripted_richtext_xfa_and_unknown_sources_fail_without_rewriting() {
    for mode in [
        "signed",
        "certified",
        "hidden",
        "script",
        "rich",
        "xfa",
        "unknown",
        "orphan",
        "matrix",
    ] {
        let mut document = inherited_fixture();
        let root = document
            .catalog()
            .unwrap()
            .get(b"AcroForm")
            .unwrap()
            .as_reference()
            .unwrap();
        let parent = document
            .get_dictionary(root)
            .unwrap()
            .get(b"Fields")
            .unwrap()
            .as_array()
            .unwrap()[0]
            .as_reference()
            .unwrap();
        let first = document
            .get_dictionary(parent)
            .unwrap()
            .get(b"Kids")
            .unwrap()
            .as_array()
            .unwrap()[0]
            .as_reference()
            .unwrap();
        let widget = document
            .get_dictionary(first)
            .unwrap()
            .get(b"Kids")
            .unwrap()
            .as_array()
            .unwrap()[0]
            .as_reference()
            .unwrap();
        match mode {
            "signed" => {
                let sig = document.add_object(dictionary! { "Type" => "Sig", "ByteRange" => vec![Object::Integer(0); 4], "Contents" => Object::string_literal("original synthetic signature marker") });
                document.catalog_mut().unwrap().set("Synthetic", sig);
            }
            "certified" => {
                document.catalog_mut().unwrap().set("Perms", dictionary! {});
            }
            "hidden" => {
                document.get_dictionary_mut(widget).unwrap().set("F", 2);
            }
            "script" => {
                document.get_dictionary_mut(parent).unwrap().set("AA", dictionary! { "K" => dictionary! { "S" => "JavaScript", "JS" => Object::string_literal("throw 'never run'") } });
            }
            "rich" => {
                document
                    .get_dictionary_mut(first)
                    .unwrap()
                    .set("RV", text_string("<p>Rich content</p>"));
            }
            "xfa" => {
                document
                    .get_dictionary_mut(root)
                    .unwrap()
                    .set("XFA", Object::string_literal("synthetic unsupported XFA"));
            }
            "unknown" => {
                document
                    .get_dictionary_mut(first)
                    .unwrap()
                    .set("FT", "Unknown");
            }
            "orphan" => {
                let orphan = document.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Widget", "Rect" => numbers(&[20.0, 20.0, 60.0, 60.0]), "FT" => "Tx", "V" => text_string("Do not drop") });
                document
                    .get_dictionary_mut(document.get_pages()[&1])
                    .unwrap()
                    .get_mut(b"Annots")
                    .unwrap()
                    .as_array_mut()
                    .unwrap()
                    .push(orphan.into());
            }
            // Malformed page content must fail instead of get_page_content's
            // best-effort behavior silently omitting an unreadable stream.
            "matrix" => {
                document
                    .get_dictionary_mut(document.get_pages()[&1])
                    .unwrap()
                    .set("Contents", Object::Integer(7));
            }
            _ => unreachable!(),
        }
        let mut session =
            Session::from_bytes(bytes(&mut document), "Unsupported.pdf".into(), None, 1).unwrap();
        let before = session.bytes().to_vec();
        assert!(session.flattened_form_copy().is_err(), "{mode}");
        assert_eq!(session.bytes(), before, "{mode}");
        if matches!(
            mode,
            "signed" | "certified" | "script" | "rich" | "xfa" | "unknown"
        ) {
            assert!(
                session
                    .design_form_fields(vec![Change::RemoveField { id: key(first) }])
                    .is_err(),
                "{mode}"
            );
            assert_eq!(session.bytes(), before, "{mode}");
        }
    }
}

#[test]
fn workspace_export_rechecks_drafts_revisions_source_saved_aliases_and_other_tabs() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.pdf");
    let saved = dir.path().join("saved.pdf");
    let other = dir.path().join("other.pdf");
    let mut document = inherited_fixture();
    std::fs::write(&source, bytes(&mut document)).unwrap();
    std::fs::write(&other, Session::blank(612.0, 792.0, 99).unwrap().bytes()).unwrap();
    let mut workspace = Workspace::default();
    let info = workspace.open_path(source.clone()).unwrap();
    let second = workspace.open_path(other.clone()).unwrap();
    workspace
        .current_mut(info.id, info.revision)
        .unwrap()
        .save_as(&saved)
        .unwrap();
    let original = workspace.document(info.id).unwrap().bytes().to_vec();
    for path in [&source, &saved, &other] {
        assert!(
            workspace
                .export_flattened_form_copy(info.id, info.revision, path)
                .is_err()
        );
    }
    #[cfg(unix)]
    {
        let alias = dir.path().join("source-alias.pdf");
        std::os::unix::fs::symlink(&source, &alias).unwrap();
        assert!(
            workspace
                .export_flattened_form_copy(info.id, info.revision, &alias)
                .is_err()
        );
    }
    workspace.set_draft(info.id, true).unwrap();
    assert!(
        workspace
            .flattened_form_copy(info.id, info.revision)
            .is_err()
    );
    assert!(
        workspace
            .close_document(info.id, info.revision, false)
            .is_err()
    );
    workspace.set_draft(info.id, false).unwrap();
    let id = field(workspace.document(info.id).unwrap(), "person.first").id;
    let changed = workspace
        .current_mut(info.id, info.revision)
        .unwrap()
        .fill_form_fields(vec![FieldChange {
            id,
            value: FieldValue::Text {
                text: "Unsaved".into(),
            },
        }])
        .unwrap();
    assert!(
        workspace
            .flattened_form_copy(info.id, info.revision)
            .is_err()
    );
    let before = workspace.document(info.id).unwrap().bytes().to_vec();
    let output = dir.path().join("static.pdf");
    workspace
        .export_flattened_form_copy(info.id, changed.revision, &output)
        .unwrap();
    assert!(appearance_text(&load(&std::fs::read(output).unwrap())).contains("Unsaved"));
    assert_eq!(workspace.document(info.id).unwrap().bytes(), before);
    assert!(workspace.document(info.id).unwrap().is_dirty());
    assert_eq!(std::fs::read(&source).unwrap(), original);
    assert_eq!(std::fs::read(&saved).unwrap(), original);
    assert_eq!(workspace.current().unwrap().id, second.id);
    assert_eq!(
        workspace.document(second.id).unwrap().bytes(),
        std::fs::read(other).unwrap()
    );
}

#[test]
fn flattened_list_never_silently_drops_selected_values_beyond_the_visible_rows() {
    let mut session = Session::blank(612.0, 792.0, 1).unwrap();
    let mut list = definition(ControlKind::List, "many");
    list.multiple = true;
    list.options = (0..10)
        .map(|i| OptionDefinition {
            label: format!("Option {i}"),
            export_value: format!("export{i}"),
        })
        .collect();
    list.default_value = Some(FieldValue::Choice {
        indices: vec![0, 9],
        custom: None,
    });
    session
        .design_form_fields(vec![create(list, vec![widget(0, 30.0, 40.0, 220.0, 36.0)])])
        .unwrap();
    assert!(
        session
            .flattened_form_copy()
            .unwrap_err()
            .to_string()
            .contains("outside the visible widget")
    );
    let field = field(&session, "many");
    session
        .design_form_fields(vec![Change::PlaceWidget {
            id: field.id,
            widget_id: field.widgets[0].id.clone(),
            widget: widget(0, 30.0, 40.0, 220.0, 300.0),
        }])
        .unwrap();
    let output = load(&session.flattened_form_copy().unwrap());
    assert!(appearance_text(&output).contains("Option 9"));
}

#[test]
fn button_labels_do_not_rewrite_existing_submission_exports_and_selected_radio_removal_is_blocked()
{
    let mut session = Session::blank(612.0, 792.0, 1).unwrap();
    let mut radio = definition(ControlKind::Radio, "radio");
    radio.default_value = Some(FieldValue::Button {
        state: Some(state("Beta")),
    });
    let mut a = widget(0, 30.0, 40.0, 20.0, 20.0);
    a.option = Some(0);
    let mut b = widget(0, 30.0, 70.0, 20.0, 20.0);
    b.option = Some(1);
    session
        .design_form_fields(vec![create(radio, vec![a, b])])
        .unwrap();
    let id = field(&session, "radio").id;
    let mut document = load(session.bytes());
    document.get_dictionary_mut(object_key(&id)).unwrap().set(
        "Opt",
        vec![
            text_string("original-export-a"),
            text_string("original-export-b"),
        ],
    );
    let mut session =
        Session::from_bytes(bytes(&mut document), "Button exports.pdf".into(), None, 1).unwrap();
    let mut definition = props(&session, "radio");
    definition.options[1].label = "New accessible option label".into();
    session
        .design_form_fields(vec![Change::Update {
            id: id.clone(),
            definition,
        }])
        .unwrap();
    let actual = field(&session, "radio");
    let beta = actual
        .options
        .iter()
        .find(|option| option.id == state("Beta"))
        .unwrap();
    assert_eq!(beta.label, "New accessible option label");
    assert_eq!(beta.export_value, "original-export-b");
    assert_eq!(
        actual.value,
        Some(FieldValue::Button {
            state: Some(state("Beta"))
        })
    );
    let before = session.bytes().to_vec();
    assert!(
        session
            .design_form_fields(vec![Change::RemoveWidget {
                id,
                widget_id: actual.widgets[1].id.clone()
            }])
            .is_err()
    );
    assert_eq!(session.bytes(), before);
}

#[test]
fn removal_rejects_external_references_instead_of_creating_dangling_tagged_or_custom_objects() {
    let mut document = inherited_fixture();
    let mut session =
        Session::from_bytes(bytes(&mut document), "References.pdf".into(), None, 1).unwrap();
    let selected = field(&session, "person.first");
    let mut document = load(session.bytes());
    let external = document
        .add_object(dictionary! { "Type" => "OBJR", "Obj" => object_key(&selected.widgets[0].id) });
    document
        .catalog_mut()
        .unwrap()
        .set("SyntheticStructure", external);
    session = Session::from_bytes(bytes(&mut document), "References.pdf".into(), None, 1).unwrap();
    let original = session.bytes().to_vec();
    for change in [
        Change::RemoveField {
            id: selected.id.clone(),
        },
        Change::RemoveWidget {
            id: selected.id.clone(),
            widget_id: selected.widgets[0].id.clone(),
        },
    ] {
        let error = session
            .design_form_fields(vec![change])
            .unwrap_err()
            .to_string();
        assert!(error.contains("outside its form tree"), "{error}");
        assert_eq!(session.bytes(), original);
    }
    assert!(
        session.flattened_form_copy().is_err(),
        "a reachable tagged widget must not be silently retained in a static copy"
    );
}

#[test]
fn exact_design_batch_and_widget_limits_are_enforced_transactionally() {
    let mut session = Session::blank(612.0, 792.0, 1).unwrap();
    let changes = (0..64)
        .map(|index| {
            create(
                definition(ControlKind::Checkbox, &format!("check{index}")),
                vec![widget(0, 20.0, 20.0, 4.0, 4.0)],
            )
        })
        .collect();
    session.design_form_fields(changes).unwrap();
    assert_eq!(session.form_fields().unwrap().fields.len(), 64);
    let before = session.bytes().to_vec();
    assert!(session.design_form_fields(vec![]).is_err());
    let changes = (0..65)
        .map(|index| {
            create(
                definition(ControlKind::Checkbox, &format!("overflow{index}")),
                vec![widget(0, 20.0, 20.0, 4.0, 4.0)],
            )
        })
        .collect();
    assert!(session.design_form_fields(changes).is_err());
    assert_eq!(session.bytes(), before);
    session
        .design_form_fields(vec![create(
            definition(ControlKind::Checkbox, "repeated64"),
            (0..64).map(|_| widget(0, 50.0, 50.0, 4.0, 4.0)).collect(),
        )])
        .unwrap();
    assert_eq!(field(&session, "repeated64").widgets.len(), 64);
    let before = session.bytes().to_vec();
    for count in [0, 65] {
        assert!(
            session
                .design_form_fields(vec![create(
                    definition(ControlKind::Checkbox, "too-many"),
                    (0..count)
                        .map(|_| widget(0, 50.0, 50.0, 4.0, 4.0))
                        .collect()
                )])
                .is_err()
        );
        assert_eq!(session.bytes(), before);
    }
    assert!(
        session
            .design_form_fields(vec![create(
                definition(ControlKind::Checkbox, "too-small"),
                vec![widget(0, 10.0, 10.0, 3.999, 4.0)]
            )])
            .is_err()
    );
    assert_eq!(session.bytes(), before);
}

#[test]
fn inherited_empty_choice_defaults_and_new_partial_names_preserve_parent_semantics() {
    let mut document = inherited_fixture();
    let form = document
        .catalog()
        .unwrap()
        .get(b"AcroForm")
        .unwrap()
        .as_reference()
        .unwrap();
    let parent = document
        .get_dictionary(form)
        .unwrap()
        .get(b"Fields")
        .unwrap()
        .as_array()
        .unwrap()[0]
        .as_reference()
        .unwrap();
    let first = document
        .get_dictionary(parent)
        .unwrap()
        .get(b"Kids")
        .unwrap()
        .as_array()
        .unwrap()[0]
        .as_reference()
        .unwrap();
    document.get_dictionary_mut(first).unwrap().remove(b"T");
    let mut session =
        Session::from_bytes(bytes(&mut document), "Unnamed leaf.pdf".into(), None, 1).unwrap();
    let selected = field(&session, "person");
    let mut definition = props(&session, "person");
    definition.name = "second".into();
    let before = session.bytes().to_vec();
    assert!(
        session
            .design_form_fields(vec![Change::Update {
                id: selected.id.clone(),
                definition
            }])
            .is_err()
    );
    assert_eq!(session.bytes(), before);
    let mut definition = props(&session, "person");
    definition.name = "renamed".into();
    session
        .design_form_fields(vec![Change::Update {
            id: selected.id.clone(),
            definition,
        }])
        .unwrap();
    assert_eq!(field(&session, "person.renamed").id, selected.id);
    assert_eq!(field(&session, "person.second").value, selected.value);

    let mut document = inherited_fixture();
    let form = document
        .catalog()
        .unwrap()
        .get(b"AcroForm")
        .unwrap()
        .as_reference()
        .unwrap();
    let parent = document
        .get_dictionary(form)
        .unwrap()
        .get(b"Fields")
        .unwrap()
        .as_array()
        .unwrap()[0]
        .as_reference()
        .unwrap();
    let dictionary = document.get_dictionary_mut(parent).unwrap();
    dictionary.set("FT", "Ch");
    dictionary.set("Ff", 1 << 17);
    dictionary.set("Opt", vec![text_string("a"), text_string("b")]);
    dictionary.set("V", text_string("a"));
    dictionary.set("DV", text_string("a"));
    let mut session = Session::from_bytes(
        bytes(&mut document),
        "Inherited choices.pdf".into(),
        None,
        1,
    )
    .unwrap();
    let mut definition = props(&session, "person.first");
    definition.default_value = Some(FieldValue::Choice {
        indices: vec![],
        custom: None,
    });
    let id = field(&session, "person.first").id;
    session
        .design_form_fields(vec![Change::Update { id, definition }])
        .unwrap();
    assert_eq!(props(&session, "person.first").default_value, None);
    assert_eq!(
        props(&session, "person.second").default_value,
        Some(FieldValue::Choice {
            indices: vec![0],
            custom: None
        })
    );
    assert_eq!(
        field(&session, "person.first").value,
        Some(FieldValue::Choice {
            indices: vec![0],
            custom: None
        })
    );
}

#[test]
fn unsigned_signature_slots_keep_signer_compatible_mk_rotation_when_created_and_moved() {
    let mut document = geometry_document();
    let mut session = Session::from_bytes(
        bytes(&mut document),
        "Unsigned slot geometry.pdf".into(),
        None,
        1,
    )
    .unwrap();
    session
        .design_form_fields(
            (0..4)
                .map(|page| {
                    create(
                        definition(ControlKind::Signature, &format!("slot{page}")),
                        vec![widget(page, 40.0, 170.0, 240.0, 40.0)],
                    )
                })
                .collect(),
        )
        .unwrap();
    for moved in [false, true] {
        if moved {
            let changes = (0..4)
                .map(|page| {
                    let field = field(&session, &format!("slot{page}"));
                    Change::PlaceWidget {
                        id: field.id,
                        widget_id: field.widgets[0].id.clone(),
                        widget: widget(3 - page, 300.0, 240.0, 220.0, 40.0),
                    }
                })
                .collect();
            session.design_form_fields(changes).unwrap();
        }
        let document = load(session.bytes());
        for page in 0..4 {
            let field = field(&session, &format!("slot{page}"));
            let dictionary = document.get_dictionary(object_key(&field.id)).unwrap();
            assert!(!dictionary.has(b"V"));
            assert_eq!(dictionary.get(b"Ff").unwrap().as_i64().unwrap(), 0);
            let widget = document
                .get_dictionary(object_key(&field.widgets[0].id))
                .unwrap();
            assert_eq!(
                widget
                    .get(b"MK")
                    .unwrap()
                    .as_dict()
                    .unwrap()
                    .get(b"R")
                    .unwrap()
                    .as_i64()
                    .unwrap(),
                0
            );
            assert_eq!(
                widget.get(b"P").unwrap().as_reference().unwrap(),
                document.get_pages()[&(field.widgets[0].page as u32 + 1)]
            );
        }
        assert!(session.info().unwrap().read_only_reason.is_none());
        let reopened = Session::from_bytes(
            session.bytes().to_vec(),
            "Reopened unsigned slots.pdf".into(),
            None,
            2,
        )
        .unwrap();
        assert!(reopened.form_designer().unwrap().authorable);
        if let Ok(directory) = std::env::var("FOLIO_FORM_EVIDENCE_DIR") {
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                Path::new(&directory).join(if moved {
                    "unsigned-slots-moved.pdf"
                } else {
                    "unsigned-slots-created.pdf"
                }),
                session.bytes(),
            )
            .unwrap();
            std::fs::write(
                Path::new(&directory).join(if moved {
                    "unsigned-slots-moved.json"
                } else {
                    "unsigned-slots-created.json"
                }),
                serde_json::to_vec_pretty(&session.form_fields().unwrap()).unwrap(),
            )
            .unwrap();
        }
    }
}

// Set FOLIO_FORM_EVIDENCE_DIR to retain original synthetic outputs for the
// independent PDF.js/pdf-lib evidence runner; ordinary tests write no artifacts.
#[test]
fn writes_original_interoperability_evidence_when_requested() {
    let Ok(directory) = std::env::var("FOLIO_FORM_EVIDENCE_DIR") else {
        return;
    };
    let path = Path::new(&directory);
    std::fs::create_dir_all(path).unwrap();
    let mut document = geometry_document();
    let mut session = Session::from_bytes(
        bytes(&mut document),
        "Form designer evidence.pdf".into(),
        None,
        1,
    )
    .unwrap();
    let mut text = definition(ControlKind::Text, "evidence_name");
    text.default_value = Some(FieldValue::Text {
        text: "Zoë PDF".into(),
    });
    let placements = (0..4)
        .map(|page| widget(page, 40.0, 40.0, 240.0, 40.0))
        .collect();
    let mut check = definition(ControlKind::Checkbox, "evidence_check");
    check.default_value = Some(FieldValue::Button {
        state: Some(state("Yes")),
    });
    let mut password = definition(ControlKind::Text, "evidence_password");
    password.password = true;
    password.font_size = 0.0;
    password.default_value = Some(FieldValue::Text {
        text: "original-synthetic-password".into(),
    });
    let mut radio = definition(ControlKind::Radio, "evidence_radio");
    radio.default_value = Some(FieldValue::Button {
        state: Some(state("Beta")),
    });
    let mut alpha = widget(0, 40.0, 180.0, 20.0, 20.0);
    alpha.option = Some(0);
    let mut beta = widget(0, 40.0, 210.0, 20.0, 20.0);
    beta.option = Some(1);
    let mut list = definition(ControlKind::List, "evidence_list");
    list.multiple = true;
    list.read_only = true;
    list.default_value = Some(FieldValue::Choice {
        indices: vec![0, 1],
        custom: None,
    });
    let mut combo = definition(ControlKind::Combo, "evidence_combo");
    combo.editable_choice = true;
    combo.default_value = Some(FieldValue::Choice {
        indices: vec![],
        custom: Some("Custom choice".into()),
    });
    let mut comb = definition(ControlKind::Text, "evidence_comb");
    comb.comb = true;
    comb.max_length = 4;
    comb.default_value = Some(FieldValue::Text {
        text: "1234".into(),
    });
    let mut multiline = definition(ControlKind::Text, "evidence_multiline");
    multiline.multiline = true;
    multiline.default_value = Some(FieldValue::Text {
        text: "Line one\nLine two".into(),
    });
    session
        .design_form_fields(vec![
            create(text, placements),
            create(
                check,
                (0..4)
                    .map(|page| widget(page, 40.0, 110.0, 24.0, 24.0))
                    .collect(),
            ),
            create(password, vec![widget(1, 40.0, 170.0, 240.0, 40.0)]),
            create(
                definition(ControlKind::Signature, "evidence_unsigned"),
                vec![widget(2, 40.0, 170.0, 240.0, 40.0)],
            ),
            create(radio, vec![alpha, beta]),
            create(
                list,
                vec![
                    widget(0, 40.0, 270.0, 240.0, 120.0),
                    widget(3, 40.0, 270.0, 240.0, 120.0),
                ],
            ),
            create(combo, vec![widget(1, 40.0, 250.0, 240.0, 40.0)]),
            create(comb, vec![widget(2, 40.0, 250.0, 120.0, 40.0)]),
            create(multiline, vec![widget(1, 40.0, 340.0, 240.0, 80.0)]),
        ])
        .unwrap();
    session.save_as(&path.join("authored.pdf")).unwrap();
    std::fs::write(
        path.join("static.pdf"),
        session.flattened_form_copy().unwrap(),
    )
    .unwrap();
    std::fs::write(
        path.join("field-contract.json"),
        serde_json::to_vec_pretty(&session.form_fields().unwrap()).unwrap(),
    )
    .unwrap();
}
