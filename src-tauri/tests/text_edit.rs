use folio_core::{
    document::{Edit, Session},
    text_edit::{ReplacementFont, TextRun},
};
use lopdf::{Dictionary, Document, Object, Stream, content::Content, dictionary};

fn pdf_with_pages(contents: &[&[u8]]) -> Document {
    let mut pdf = Document::with_version("1.7");
    let pages = pdf.new_object_id();
    let font = pdf.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let resources = pdf.add_object(dictionary! { "Font" => dictionary! { "F1" => font } });
    let ids: Vec<_> = contents
        .iter()
        .map(|bytes| {
            let stream = pdf.add_object(Stream::new(dictionary! {}, bytes.to_vec()));
            pdf.add_object(dictionary! { "Type" => "Page", "Parent" => pages,
            "Contents" => stream })
        })
        .collect();
    pdf.objects.insert(pages, dictionary! {
        "Type" => "Pages", "Count" => ids.len() as i64,
        "Kids" => ids.into_iter().map(Object::Reference).collect::<Vec<_>>(),
        "Resources" => resources, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    }.into());
    let catalog = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
    pdf.trailer.set("Root", catalog);
    pdf
}

fn session(mut pdf: Document) -> Session {
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).unwrap();
    Session::from_bytes(bytes, "text.pdf".into(), None, 1).unwrap()
}

fn loaded(session: &Session) -> Document {
    Document::load_mem(session.bytes()).unwrap()
}

fn run(session: &Session, page: usize, text: &str) -> TextRun {
    session
        .text_runs(page)
        .unwrap()
        .into_iter()
        .find(|run| run.text.as_deref() == Some(text))
        .unwrap()
}

fn replace(
    session: &mut Session,
    page: usize,
    text: &str,
    replacement: &str,
    font: ReplacementFont,
) {
    let target = run(session, page, text);
    session
        .apply(Edit::ReplaceText {
            page,
            path: target.path,
            expected: text.into(),
            replacement: replacement.into(),
            font,
        })
        .unwrap();
}

#[test]
fn replaces_real_content_without_annotations_and_survives_save_reopen() {
    let mut session = session(pdf_with_pages(&[
        b"BT /F1 18 Tf 0.2 0.3 0.6 rg 1 0 0 1 50 700 Tm (Original sentence) Tj ET",
    ]));
    let target = run(&session, 0, "Original sentence");
    assert!(target.editable);
    assert_eq!(target.font, "Helvetica");
    assert_eq!(target.font_size, Some(18.0));
    assert_eq!(target.origin, Some([50.0, 700.0]));
    let original = session.bytes().to_vec();
    replace(
        &mut session,
        0,
        "Original sentence",
        "Edited sentence",
        ReplacementFont::Original,
    );
    assert!(session.info().unwrap().dirty);
    assert!(session.info().unwrap().annotations.is_empty());
    let pdf = loaded(&session);
    let extracted = pdf.extract_text(&[1]).unwrap();
    assert!(extracted.contains("Edited sentence"));
    assert!(!extracted.contains("Original sentence"));
    let edited = run(&session, 0, "Edited sentence");
    assert_eq!(edited.font, target.font);
    assert_eq!(edited.font_size, target.font_size);
    assert_eq!(edited.origin, target.origin);
    session.undo().unwrap();
    assert_eq!(session.bytes(), original);
    session.redo().unwrap();
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("edited.pdf");
    session.save_as(&path).unwrap();
    let reopened = Session::from_path(path, 2).unwrap();
    assert!(
        loaded(&reopened)
            .extract_text(&[1])
            .unwrap()
            .contains("Edited sentence")
    );
    assert!(reopened.info().unwrap().annotations.is_empty());
    assert!(!reopened.info().unwrap().dirty);
}

#[test]
fn repeated_text_is_targeted_by_operator_path_not_by_global_replacement() {
    let mut session = session(pdf_with_pages(&[
        b"BT /F1 12 Tf 1 0 0 1 30 700 Tm (Repeated) Tj 1 0 0 1 30 500 Tm (Repeated) Tj ET",
    ]));
    let runs = session.text_runs(0).unwrap();
    assert_eq!(runs.len(), 2);
    assert_ne!(runs[0].path, runs[1].path);
    assert_eq!(runs[1].origin, Some([30.0, 500.0]));
    session
        .apply(Edit::ReplaceText {
            page: 0,
            path: runs[1].path.clone(),
            expected: "Repeated".into(),
            replacement: "Only this one".into(),
            font: ReplacementFont::Original,
        })
        .unwrap();
    let runs = session.text_runs(0).unwrap();
    assert_eq!(runs[0].text.as_deref(), Some("Repeated"));
    assert_eq!(runs[1].text.as_deref(), Some("Only this one"));
}

#[test]
fn handles_tj_spacing_and_quote_operators_without_losing_line_movements() {
    let mut session = session(pdf_with_pages(&[
        b"BT /F1 12 Tf 20 TL 1 0 0 1 40 700 Tm [(Ker) -30 (ned)] TJ (Quoted) ' 2 1 (Double) \" ET",
    ]));
    let runs = session.text_runs(0).unwrap();
    assert_eq!(
        runs.iter()
            .map(|run| run.text.as_deref().unwrap())
            .collect::<Vec<_>>(),
        ["Kerned", "Quoted", "Double"]
    );
    assert_eq!(runs[1].origin, Some([40.0, 680.0]));
    assert_eq!(runs[2].origin, Some([40.0, 660.0]));
    replace(
        &mut session,
        0,
        "Kerned",
        "Changed",
        ReplacementFont::Original,
    );
    replace(
        &mut session,
        0,
        "Quoted",
        "Next line",
        ReplacementFont::Original,
    );
    replace(
        &mut session,
        0,
        "Double",
        "Last line",
        ReplacementFont::Original,
    );
    let runs = session.text_runs(0).unwrap();
    assert_eq!(runs[1].origin, Some([40.0, 680.0]));
    assert_eq!(runs[2].origin, Some([40.0, 660.0]));
    let pdf = loaded(&session);
    let bytes = pdf.get_page_content(*pdf.get_pages().values().next().unwrap());
    let content = Content::decode(&bytes).unwrap();
    assert!(
        content
            .operations
            .iter()
            .any(|op| op.operator == "Tw" && op.operands[0].as_i64().unwrap() == 2)
    );
    assert!(
        content
            .operations
            .iter()
            .any(|op| op.operator == "Tc" && op.operands[0].as_i64().unwrap() == 1)
    );
}

#[test]
fn shared_content_streams_and_inherited_resources_are_not_modified_on_other_pages() {
    let mut pdf = pdf_with_pages(&[b"BT /F1 14 Tf 40 700 Td (Shared original) Tj ET", b""]);
    let pages: Vec<_> = pdf.get_pages().into_values().collect();
    let stream = pdf
        .get_dictionary(pages[0])
        .unwrap()
        .get(b"Contents")
        .unwrap()
        .clone();
    pdf.get_dictionary_mut(pages[1])
        .unwrap()
        .set("Contents", stream.clone());
    let mut session = session(pdf);
    replace(
        &mut session,
        0,
        "Shared original",
        "Page one only",
        ReplacementFont::Original,
    );
    assert_eq!(
        run(&session, 1, "Shared original").text.as_deref(),
        Some("Shared original")
    );
    let pdf = loaded(&session);
    assert_ne!(
        pdf.get_dictionary(pages[0])
            .unwrap()
            .get(b"Contents")
            .unwrap(),
        &stream
    );
    assert_eq!(
        pdf.get_dictionary(pages[1])
            .unwrap()
            .get(b"Contents")
            .unwrap(),
        &stream
    );
}

#[test]
fn isolates_one_form_invocation_including_nested_forms_and_transforms() {
    let mut pdf =
        pdf_with_pages(&[b"q 1 0 0 1 50 500 cm /Outer Do Q q 1 0 0 1 50 300 cm /Outer Do Q"]);
    let page = *pdf.get_pages().values().next().unwrap();
    let inner = pdf.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 200.into(), 100.into()],
            "Matrix" => vec![1.into(), 0.into(), 0.into(), 1.into(), 10.into(), 20.into()],
        },
        b"BT /F1 12 Tf 1 0 0 1 5 10 Tm (Form text) Tj ET".to_vec(),
    ));
    let parent = pdf
        .get_dictionary(page)
        .unwrap()
        .get(b"Parent")
        .unwrap()
        .as_reference()
        .unwrap();
    let resources = pdf
        .get_dictionary(parent)
        .unwrap()
        .get(b"Resources")
        .unwrap();
    let mut resources = pdf
        .dereference(resources)
        .unwrap()
        .1
        .as_dict()
        .unwrap()
        .clone();
    resources.set("XObject", dictionary! { "Inner" => inner });
    let outer = pdf.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 300.into(), 200.into()],
            "Resources" => resources,
        },
        b"/Inner Do".to_vec(),
    ));
    let mut resources = Dictionary::new();
    resources.set("XObject", dictionary! { "Outer" => outer });
    pdf.get_dictionary_mut(page)
        .unwrap()
        .set("Resources", resources);
    let mut session = session(pdf);
    let runs = session.text_runs(0).unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].path.len(), 3);
    assert_eq!(runs[0].origin, Some([65.0, 530.0]));
    assert_eq!(runs[1].origin, Some([65.0, 330.0]));
    let original_inner = loaded(&session).get_object(inner).unwrap().clone();
    session
        .apply(Edit::ReplaceText {
            page: 0,
            path: runs[0].path.clone(),
            expected: "Form text".into(),
            replacement: "Isolated edit".into(),
            font: ReplacementFont::Original,
        })
        .unwrap();
    let runs = session.text_runs(0).unwrap();
    assert_eq!(runs[0].text.as_deref(), Some("Isolated edit"));
    assert_eq!(runs[1].text.as_deref(), Some("Form text"));
    assert_eq!(loaded(&session).get_object(inner).unwrap(), &original_inner);
}

#[test]
fn explicit_embedded_font_supports_new_characters_and_restores_surrounding_font_state() {
    let mut session = session(pdf_with_pages(&[
        b"BT /F1 15 Tf 50 700 Td (Old) Tj 0 -30 Td (Surrounding) Tj ET",
    ]));
    let before = session.bytes().to_vec();
    let target = run(&session, 0, "Old");
    assert!(
        session
            .apply(Edit::ReplaceText {
                page: 0,
                path: target.path,
                expected: "Old".into(),
                replacement: "Καλημέρα café".into(),
                font: ReplacementFont::Original
            })
            .is_err()
    );
    assert_eq!(session.bytes(), before);
    replace(
        &mut session,
        0,
        "Old",
        "Καλημέρα café",
        ReplacementFont::Noto,
    );
    let edited = run(&session, 0, "Καλημέρα café");
    assert!(edited.editable, "{:?}", edited.reason);
    assert_eq!(edited.font, "NotoSans");
    assert_eq!(edited.font_size, Some(15.0));
    assert_eq!(edited.origin, Some([50.0, 700.0]));
    assert_eq!(run(&session, 0, "Surrounding").font, "Helvetica");
    let pdf = loaded(&session);
    assert!(pdf.extract_text(&[1]).unwrap().contains("Καλημέρα café"));
    assert!(session.info().unwrap().annotations.is_empty());
}

#[test]
fn subset_unicode_fonts_allow_existing_glyphs_and_report_missing_ones() {
    let mut source = Session::blank(612.0, 792.0, 1).unwrap();
    source
        .apply(Edit::Text {
            page: 0,
            rect: [50.0, 500.0, 500.0, 700.0],
            text: "Hello café".into(),
            size: 14.0,
            color: [0.0, 0.0, 0.0],
        })
        .unwrap();
    let mut pdf = loaded(&source);
    let page = *pdf.get_pages().values().next().unwrap();
    let annotation = pdf.get_page_annotations(page).unwrap()[0];
    let appearance = annotation
        .get(b"AP")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"N")
        .unwrap();
    let appearance = pdf.dereference(appearance).unwrap().1.as_stream().unwrap();
    let resources = appearance.dict.get(b"Resources").unwrap().clone();
    let content = Content::decode(&appearance.get_plain_content().unwrap()).unwrap();
    let encoded = content
        .operations
        .iter()
        .find(|op| op.operator == "Tj")
        .unwrap()
        .operands[0]
        .clone();
    let content = Content {
        operations: vec![
            lopdf::content::Operation::new("BT", vec![]),
            lopdf::content::Operation::new("Tf", vec!["FolioFont".into(), 14.into()]),
            lopdf::content::Operation::new("Td", vec![50.into(), 700.into()]),
            lopdf::content::Operation::new("Tj", vec![encoded]),
            lopdf::content::Operation::new("ET", vec![]),
        ],
    }
    .encode()
    .unwrap();
    let contents = pdf.add_object(Stream::new(dictionary! {}, content));
    let page = pdf.get_dictionary_mut(page).unwrap();
    page.set("Resources", resources);
    page.set("Contents", contents);
    page.remove(b"Annots");
    pdf.prune_objects();
    let mut session = session(pdf);
    replace(
        &mut session,
        0,
        "Hello café",
        "café café",
        ReplacementFont::Original,
    );
    let target = run(&session, 0, "café café");
    let before = session.bytes().to_vec();
    assert!(
        session
            .apply(Edit::ReplaceText {
                page: 0,
                path: target.path,
                expected: "café café".into(),
                replacement: "New glyph".into(),
                font: ReplacementFont::Original
            })
            .is_err()
    );
    assert_eq!(session.bytes(), before);
}

#[test]
fn rejects_stale_targets_signed_files_hidden_text_and_actualtext_metadata() {
    let source = b"BT /F1 12 Tf 50 700 Td (Visible) Tj ET /Span << /ActualText (Semantic) >> BDC BT /F1 12 Tf 50 650 Td (Tagged) Tj ET EMC BT /F1 12 Tf 3 Tr 50 600 Td (OCR) Tj ET";
    let mut session = session(pdf_with_pages(&[source]));
    let runs = session.text_runs(0).unwrap();
    assert!(runs[0].editable);
    assert!(!runs[1].editable);
    assert!(runs[1].reason.as_deref().unwrap().contains("ActualText"));
    assert!(!runs[2].editable);
    let original = session.bytes().to_vec();
    for (path, expected, replacement) in [
        (runs[0].path.clone(), "Wrong", "Changed"),
        (vec![usize::MAX], "Visible", "Changed"),
        (runs[1].path.clone(), "Tagged", "Changed"),
        (runs[2].path.clone(), "OCR", "Changed"),
        (runs[0].path.clone(), "Visible", "Two\nlines"),
    ] {
        assert!(
            session
                .apply(Edit::ReplaceText {
                    page: 0,
                    path,
                    expected: expected.into(),
                    replacement: replacement.into(),
                    font: ReplacementFont::Original
                })
                .is_err()
        );
        assert_eq!(session.bytes(), original);
    }
    let mut pdf = loaded(&session);
    pdf.add_object(
        dictionary! { "Type" => "Sig", "ByteRange" => vec![0.into(), 1.into()],
        "Contents" => Object::string_literal("signature") },
    );
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).unwrap();
    let mut signed = Session::from_bytes(bytes.clone(), "signed.pdf".into(), None, 2).unwrap();
    let target = run(&signed, 0, "Visible");
    assert!(
        signed
            .apply(Edit::ReplaceText {
                page: 0,
                path: target.path,
                expected: "Visible".into(),
                replacement: "Changed".into(),
                font: ReplacementFont::Original
            })
            .is_err()
    );
    assert_eq!(signed.bytes(), bytes);
}

#[test]
fn deletion_is_real_content_removal_and_does_not_remove_neighboring_text() {
    let mut session = session(pdf_with_pages(&[
        b"BT /F1 12 Tf 50 700 Td (Remove me) Tj 0 -25 Td (Keep me) Tj ET",
    ]));
    replace(&mut session, 0, "Remove me", "", ReplacementFont::Original);
    let pdf = loaded(&session);
    let text = pdf.extract_text(&[1]).unwrap();
    assert!(!text.contains("Remove me"));
    assert!(text.contains("Keep me"));
    assert!(session.info().unwrap().annotations.is_empty());
}

#[test]
fn invalid_and_excessive_unicode_maps_are_explicitly_unsupported() {
    let mut pdf = pdf_with_pages(&[b"BT /F1 12 Tf 50 700 Td (Old) Tj ET"]);
    let cmap = pdf.add_object(Stream::new(
        dictionary! {},
        b"1 beginbfrange <00000000> <FFFFFFFF> <0041> endbfrange".to_vec(),
    ));
    let page = *pdf.get_pages().values().next().unwrap();
    let mut font = {
        let fonts = pdf.get_page_fonts(page).unwrap();
        (*fonts.get(b"F1".as_slice()).unwrap()).clone()
    };
    font.set("ToUnicode", cmap);
    pdf.get_dictionary_mut(page).unwrap().set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F1" => font } },
    );
    let session = session(pdf);
    let runs = session.text_runs(0).unwrap();
    assert!(!runs[0].editable);
    assert!(
        runs[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("complex Unicode")
    );
}

#[test]
fn differences_encodings_without_optional_type_are_decoded_strictly() {
    let mut pdf = pdf_with_pages(&[b"BT /F1 12 Tf 50 700 Td (ABC) Tj ET"]);
    let page = *pdf.get_pages().values().next().unwrap();
    let font = pdf.add_object(
        dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => dictionary! { "BaseEncoding" => "WinAnsiEncoding",
            "Differences" => vec![65.into(), "X".into(), "Y".into(), "Z".into()] } },
    );
    pdf.get_dictionary_mut(page).unwrap().set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F1" => font } },
    );
    let mut session = session(pdf);
    assert!(run(&session, 0, "XYZ").editable);
    replace(&mut session, 0, "XYZ", "ZYX", ReplacementFont::Original);
    assert_eq!(run(&session, 0, "ZYX").text.as_deref(), Some("ZYX"));
}

#[test]
fn out_of_range_differences_do_not_wrap_and_guess_glyphs() {
    let mut pdf = pdf_with_pages(&[b"BT /F1 12 Tf 50 700 Td (A) Tj ET"]);
    let page = *pdf.get_pages().values().next().unwrap();
    let font = pdf.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => dictionary! { "BaseEncoding" => "WinAnsiEncoding",
            "Differences" => vec![255.into(), "A".into(), "B".into()] },
    });
    pdf.get_dictionary_mut(page).unwrap().set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F1" => font } },
    );
    let session = session(pdf);
    let runs = session.text_runs(0).unwrap();
    assert!(!runs[0].editable);
    assert!(
        runs[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("invalid source codes")
    );
}

#[test]
fn distinct_source_fonts_have_a_bounded_editing_budget() {
    let mut pdf = pdf_with_pages(&[b""]);
    let page = *pdf.get_pages().values().next().unwrap();
    let mut fonts = Dictionary::new();
    let mut content = String::from("BT 50 700 Td ");
    for index in 0..129 {
        let font = pdf.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
        });
        fonts.set(format!("F{index}"), font);
        content.push_str(&format!("/F{index} 12 Tf (A) Tj "));
    }

    content.push_str("ET");
    let contents = pdf.add_object(Stream::new(dictionary! {}, content.into_bytes()));
    let page = pdf.get_dictionary_mut(page).unwrap();
    page.set("Resources", dictionary! { "Font" => fonts });
    page.set("Contents", contents);
    let session = session(pdf);
    let runs = session.text_runs(0).unwrap();
    assert_eq!(runs.len(), 129);
    assert!(runs[127].editable);
    assert!(!runs[128].editable);
    assert!(
        runs[128]
            .reason
            .as_deref()
            .unwrap()
            .contains("128-font limit")
    );
}

#[test]
fn repeated_inline_font_use_does_not_consume_the_distinct_font_budget() {
    let mut pdf = pdf_with_pages(&[b""]);
    let page = *pdf.get_pages().values().next().unwrap();
    let font = dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    };
    let content = format!("BT 50 700 Td {} ET", "/F1 12 Tf (A) Tj ".repeat(1000));
    let contents = pdf.add_object(Stream::new(dictionary! {}, content.into_bytes()));
    let page = pdf.get_dictionary_mut(page).unwrap();
    page.set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F1" => font } },
    );
    page.set("Contents", contents);
    let session = session(pdf);
    let runs = session.text_runs(0).unwrap();
    assert_eq!(runs.len(), 1000);
    assert!(runs.iter().all(|run| run.editable));
}

#[test]
fn unsupported_shaping_does_not_create_unshaped_text_or_drop_the_original() {
    let mut session = session(pdf_with_pages(&[
        b"BT /F1 12 Tf 50 700 Td (Original) Tj ET",
    ]));
    let before = session.bytes().to_vec();
    let target = run(&session, 0, "Original");
    for replacement in ["مرحبا", "देवनागरी", "e\u{0301}"] {
        let result = session.apply(Edit::ReplaceText {
            page: 0,
            path: target.path.clone(),
            expected: "Original".into(),
            replacement: replacement.into(),
            font: ReplacementFont::Original,
        });
        assert!(result.unwrap_err().to_string().contains("shaping"));
        assert_eq!(session.bytes(), before);
    }
    replace(
        &mut session,
        0,
        "Original",
        "e\u{0301}",
        ReplacementFont::Noto,
    );
    assert_eq!(run(&session, 0, "é").text.as_deref(), Some("é"));
}
