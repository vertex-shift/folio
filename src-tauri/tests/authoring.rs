use folio_core::{
    document::{Edit, Session},
    workspace::Workspace,
};
use lopdf::{Document, Object, dictionary, text_string};

fn loaded(session: &Session) -> Document {
    Document::load_mem(session.bytes()).unwrap()
}

#[test]
fn creating_a_pdf_produces_real_blank_pages_and_unsaved_state() {
    let mut workspace = Workspace::default();
    let info = workspace.create_document(595.0, 842.0, false).unwrap();
    assert_eq!(info.page_count, 1);
    assert!(info.dirty);
    assert!(workspace.create_document(612.0, 792.0, false).is_err());
    let original = workspace.current().unwrap().bytes().to_vec();
    assert!(workspace.create_document(f32::NAN, 842.0, true).is_err());
    assert_eq!(workspace.current().unwrap().bytes(), original);
    let pdf = loaded(workspace.current().unwrap());
    assert_eq!(pdf.get_pages().len(), 1);
    assert_eq!(pdf.extract_text(&[1]).unwrap(), "");
}

#[test]
fn text_and_ink_are_portable_editable_annotations_with_embedded_appearances() {
    let mut session = Session::blank(612.0, 792.0, 1).unwrap();
    session
        .apply(Edit::Text {
            page: 0,
            rect: [50.0, 500.0, 500.0, 700.0],
            text: "Café résumé — Ελληνικά — Привет".into(),
            size: 14.0,
            color: [0.1, 0.2, 0.4],
        })
        .unwrap();
    session
        .apply(Edit::Ink {
            page: 0,
            points: vec![[40.0, 200.0], [80.0, 250.0], [160.0, 190.0]],
            color: [0.2, 0.4, 0.8],
            width: 2.5,
        })
        .unwrap();
    let info = session.info().unwrap();
    assert_eq!(info.annotations.len(), 2);
    assert!(info.annotations[0].editable);
    assert!(!info.annotations[1].editable);
    let pdf = loaded(&session);
    let annotations = pdf
        .get_page_annotations(*pdf.get_pages().values().next().unwrap())
        .unwrap();
    for annotation in &annotations {
        let appearance = pdf
            .dereference(annotation.get(b"AP").unwrap())
            .unwrap()
            .1
            .as_dict()
            .unwrap();
        let appearance = pdf
            .dereference(appearance.get(b"N").unwrap())
            .unwrap()
            .1
            .as_stream()
            .unwrap();
        assert!(!appearance.content.is_empty());
    }
    assert!(pdf.objects.values().any(|object| {
        object
            .as_dict()
            .is_ok_and(|dict| dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Type0"))
    }));
    session
        .apply(Edit::SetAnnotationText {
            id: info.annotations[0].id.clone(),
            text: "Updated café".into(),
        })
        .unwrap();
    assert_eq!(session.info().unwrap().annotations[0].text, "Updated café");
    session.undo().unwrap();
    assert_eq!(
        session.info().unwrap().annotations[0].text,
        "Café résumé — Ελληνικά — Привет"
    );
    session.redo().unwrap();
    let reopened =
        Session::from_bytes(session.bytes().to_vec(), "copy.pdf".into(), None, 2).unwrap();
    assert_eq!(reopened.info().unwrap().annotations[0].text, "Updated café");
}

#[test]
fn text_rejects_unsupported_scripts_overflow_and_nonfinite_geometry_transactionally() {
    let mut session = Session::demo(1).unwrap();
    let before = session.bytes().to_vec();
    for (rect, text) in [
        ([50.0, 500.0, 500.0, 700.0], "東京"),
        ([50.0, 500.0, 70.0, 515.0], "This text will not fit"),
        ([f32::NAN, 20.0, 100.0, 200.0], "Invalid"),
    ] {
        assert!(
            session
                .apply(Edit::Text {
                    page: 0,
                    rect,
                    text: text.into(),
                    size: 20.0,
                    color: [0.0, 0.0, 0.0]
                })
                .is_err()
        );
        assert_eq!(session.bytes(), before);
    }
    assert!(
        session
            .apply(Edit::Ink {
                page: 0,
                points: vec![[f32::INFINITY, 1.0], [10.0, 10.0]],
                width: 2.0,
                color: [0.0, 0.0, 0.0]
            })
            .is_err()
    );
    assert_eq!(session.bytes(), before);
}

#[test]
fn text_appearance_counter_rotates_on_rotated_pages() {
    let mut session = Session::demo(1).unwrap();
    session
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    session
        .apply(Edit::Text {
            page: 0,
            rect: [200.0, 100.0, 400.0, 600.0],
            text: "Upright on a rotated page".into(),
            size: 12.0,
            color: [0.1, 0.1, 0.1],
        })
        .unwrap();
    let pdf = loaded(&session);
    let annotation = pdf
        .get_page_annotations(*pdf.get_pages().values().next().unwrap())
        .unwrap()
        .into_iter()
        .find(|annotation| {
            annotation.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"FreeText")
        })
        .unwrap();
    let ap = annotation
        .get(b"AP")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"N")
        .unwrap();
    let stream = pdf.dereference(ap).unwrap().1.as_stream().unwrap();
    let matrix: Vec<_> = stream
        .dict
        .get(b"Matrix")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_float().unwrap())
        .collect();
    assert_eq!(matrix, [0.0, 1.0, -1.0, 0.0, 200.0, 0.0]);
}

#[test]
fn duplicate_annotations_are_independent_and_blank_pages_have_correct_dimensions() {
    let mut session = Session::demo(1).unwrap();
    session
        .apply(Edit::Note {
            page: 0,
            point: [100.0, 500.0],
            text: "Original".into(),
        })
        .unwrap();
    session.apply(Edit::Duplicate { page: 0 }).unwrap();
    let info = session.info().unwrap();
    assert_eq!(info.page_count, 4);
    assert_ne!(info.pages[0].key, info.pages[1].key);
    assert_ne!(info.annotations[0].id, info.annotations[1].id);
    session
        .apply(Edit::SetAnnotationText {
            id: info.annotations[1].id.clone(),
            text: "Independent copy".into(),
        })
        .unwrap();
    assert_eq!(session.info().unwrap().annotations[0].text, "Original");
    session
        .apply(Edit::BlankPage {
            at: 2,
            width: 595.0,
            height: 842.0,
        })
        .unwrap();
    assert_eq!(session.info().unwrap().page_count, 5);
    let pdf = loaded(&session);
    let page = pdf
        .get_dictionary(*pdf.get_pages().get(&3).unwrap())
        .unwrap();
    let dimensions: Vec<_> = page
        .get(b"MediaBox")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_float().unwrap())
        .collect();
    assert_eq!(dimensions, [0.0, 0.0, 595.0, 842.0]);
}

#[test]
fn import_and_extraction_preserve_page_content_annotations_and_original_state() {
    let mut donor = Session::demo(1).unwrap();
    donor
        .apply(Edit::Rotate {
            page: 1,
            degrees: 90,
        })
        .unwrap();
    donor
        .apply(Edit::Note {
            page: 1,
            point: [100.0, 500.0],
            text: "Imported note".into(),
        })
        .unwrap();
    let mut recipient = Session::blank(612.0, 792.0, 2).unwrap();
    recipient.import_pages(donor.bytes().to_vec(), 1).unwrap();
    assert_eq!(recipient.info().unwrap().page_count, 4);
    assert_eq!(recipient.info().unwrap().pages[2].rotation, 90);
    assert_eq!(recipient.info().unwrap().annotations[0].page, 2);
    let before = recipient.bytes().to_vec();
    let bytes = recipient.extract_pages(&[2, 1]).unwrap();
    assert_eq!(recipient.bytes(), before);
    let extracted = Session::from_bytes(bytes, "pages.pdf".into(), None, 3).unwrap();
    assert_eq!(extracted.info().unwrap().page_count, 2);
    assert_eq!(extracted.info().unwrap().pages[0].rotation, 90);
    assert_eq!(extracted.info().unwrap().annotations[0].page, 0);
    assert!(
        loaded(&extracted)
            .extract_text(&[2])
            .unwrap()
            .contains("Meet your new")
    );
    assert!(recipient.extract_pages(&[]).is_err());
    assert!(recipient.extract_pages(&[0, 0]).is_err());
    assert!(recipient.extract_pages(&[4]).is_err());
    recipient.undo().unwrap();
    assert_eq!(recipient.info().unwrap().page_count, 1);
    recipient.redo().unwrap();
    assert_eq!(recipient.info().unwrap().page_count, 4);
}

#[test]
fn import_resolves_named_links_without_importing_document_javascript() {
    let donor = Session::demo(1).unwrap();
    let mut pdf = loaded(&donor);
    let pages: Vec<_> = pdf.get_pages().into_values().collect();
    let target = Object::Array(vec![pages[1].into(), Object::Name(b"Fit".to_vec())]);
    let names =
        pdf.add_object(dictionary! { "Names" => vec![Object::string_literal("chapter"), target] });
    pdf.catalog_mut()
        .unwrap()
        .set("Names", dictionary! { "Dests" => names });
    let script = pdf.add_object(dictionary! { "S" => "JavaScript", "JS" => Object::string_literal("app.alert('not imported')") });
    pdf.catalog_mut().unwrap().set("OpenAction", script);
    let link = pdf.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Link",
    "Rect" => vec![10.into(), 10.into(), 100.into(), 40.into()],
    "A" => dictionary! { "S" => "GoTo", "D" => Object::string_literal("chapter") } });
    pdf.get_dictionary_mut(pages[0])
        .unwrap()
        .set("Annots", vec![Object::Reference(link)]);
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).unwrap();
    let mut recipient = Session::blank(612.0, 792.0, 2).unwrap();
    recipient.import_pages(bytes, 1).unwrap();
    let pdf = loaded(&recipient);
    assert!(!pdf.catalog().unwrap().has(b"OpenAction"));
    let pages: Vec<_> = pdf.get_pages().into_values().collect();
    let annotation = pdf.get_page_annotations(pages[1]).unwrap()[0];
    let action = pdf
        .dereference(annotation.get(b"A").unwrap())
        .unwrap()
        .1
        .as_dict()
        .unwrap();
    let destination = action.get(b"D").unwrap().as_array().unwrap();
    assert_eq!(destination[0].as_reference().unwrap(), pages[2]);
}

#[test]
fn form_and_signed_document_guards_apply_to_new_page_workflows() {
    let mut donor = Session::demo(1).unwrap();
    let mut pdf = loaded(&donor);
    let field = pdf.add_object(
        dictionary! { "FT" => "Tx", "T" => text_string("Field"), "V" => text_string("Value") },
    );
    pdf.catalog_mut().unwrap().set(
        "AcroForm",
        dictionary! { "Fields" => vec![Object::Reference(field)] },
    );
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).unwrap();
    let form = Session::from_bytes(bytes.clone(), "form.pdf".into(), None, 2).unwrap();
    assert!(form.extract_pages(&[0]).is_err());
    let before = donor.bytes().to_vec();
    assert!(donor.import_pages(bytes, 0).is_err());
    assert_eq!(donor.bytes(), before);
    let mut pdf = loaded(&donor);
    pdf.add_object(dictionary! { "Type" => "Sig", "ByteRange" => vec![0.into(), 1.into()], "Contents" => Object::string_literal("signature") });
    let mut signed = Vec::new();
    pdf.save_to(&mut signed).unwrap();
    let signed = Session::from_bytes(signed, "signed.pdf".into(), None, 3).unwrap();
    assert!(signed.extract_pages(&[0]).is_err());
}

#[test]
fn imported_pages_materialize_inherited_resources_boxes_and_rotation() {
    let donor = Session::demo(1).unwrap();
    let mut pdf = loaded(&donor);
    let page = *pdf.get_pages().values().next().unwrap();
    let parent = pdf
        .get_dictionary(page)
        .unwrap()
        .get(b"Parent")
        .unwrap()
        .as_reference()
        .unwrap();
    let resources = pdf
        .get_dictionary(page)
        .unwrap()
        .get(b"Resources")
        .unwrap()
        .clone();
    pdf.get_dictionary_mut(parent)
        .unwrap()
        .set("Resources", resources);
    pdf.get_dictionary_mut(parent).unwrap().set("Rotate", 270);
    pdf.get_dictionary_mut(parent).unwrap().set(
        "CropBox",
        vec![10.into(), 20.into(), 600.into(), 780.into()],
    );
    pdf.get_dictionary_mut(page).unwrap().remove(b"Resources");
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).unwrap();
    let mut destination = Session::blank(612.0, 792.0, 2).unwrap();
    destination.import_pages(bytes, 0).unwrap();
    let info = destination.info().unwrap();
    assert_eq!(info.pages[0].rotation, 270);
    let pdf = loaded(&destination);
    let imported = pdf
        .get_dictionary(*pdf.get_pages().values().next().unwrap())
        .unwrap();
    assert!(imported.has(b"Resources"));
    assert!(imported.has(b"MediaBox"));
    assert!(imported.has(b"CropBox"));
    assert!(pdf.extract_text(&[1]).unwrap().contains("Meet your new"));
}
