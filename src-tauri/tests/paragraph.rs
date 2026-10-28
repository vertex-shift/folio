use folio_core::{
    document::{Edit, Session},
    paragraph::{Alignment, ParagraphBlock, ParagraphChange},
    text_edit::ReplacementFont,
    workspace::Workspace,
};
use lopdf::{
    Document, LoadOptions, Object, Stream,
    content::Content,
    dictionary,
    encryption::{EncryptionState, EncryptionVersion, Permissions},
};

fn numbers(values: &[f32]) -> Object {
    Object::Array(values.iter().map(|value| Object::Real(*value)).collect())
}

fn bytes(mut pdf: Document) -> Vec<u8> {
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).unwrap();
    bytes
}

fn fixture(content: &[u8]) -> Document {
    let mut pdf = Document::with_version("1.7");
    let pages = pdf.new_object_id();
    let font = pdf.add_object(dictionary! {
        "Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica","Encoding"=>"WinAnsiEncoding",
    });
    let stream = pdf.add_object(Stream::new(dictionary! {}, content.to_vec()));
    let page = pdf.add_object(dictionary! {"Type"=>"Page","Parent"=>pages,"Contents"=>stream});
    pdf.objects.insert(
        pages,
        dictionary! {
            "Type"=>"Pages","Kids"=>vec![Object::Reference(page)],"Count"=>1,
            "MediaBox"=>numbers(&[-20.0,-30.0,612.0,792.0]),
            "CropBox"=>numbers(&[0.0,10.0,600.0,780.0]),
            "Resources"=>dictionary! {"Font"=>dictionary! {"F1"=>font}},
        }
        .into(),
    );
    let root = pdf.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages});
    pdf.trailer.set("Root", root);
    pdf
}

fn open(pdf: Document) -> Session {
    Session::from_bytes(bytes(pdf), "paragraph.pdf".into(), None, 1).unwrap()
}
fn first(session: &Session) -> ParagraphBlock {
    session.paragraph_blocks(0).unwrap().blocks.remove(0)
}
fn change(session: &Session, block: &ParagraphBlock, replacement: &str) -> ParagraphChange {
    ParagraphChange {
        epoch: session.paragraph_blocks(0).unwrap().epoch,
        page: 0,
        path: block.path.clone(),
        block_id: block.id.clone(),
        expected: block.text.clone().unwrap(),
        text: replacement.into(),
        placement: [40.0, 60.0, 230.0, 130.0],
        size: 12.0,
        leading: 17.0,
        alignment: Alignment::Left,
        font: ReplacementFont::Noto,
    }
}

const SOURCE: &[u8]=b"0.15 0.3 0.5 rg BT /F1 12 Tf 16 TL 1 0 0 1 40 700 Tm (Original first line) Tj T* (with a second line) Tj T* (and the final line.) Tj ET BT 1 0 0 1 330 700 Tm (Independent right column) Tj ET 0.1 0.6 0.2 rg 330 400 80 40 re f";

#[test]
fn replaces_a_multiline_source_object_with_searchable_wrapped_text_and_atomic_history() {
    let mut session = open(fixture(SOURCE));
    let original = session.bytes().to_vec();
    let blocks = session.paragraph_blocks(0).unwrap();
    assert_eq!(blocks.blocks.len(), 2);
    let block = &blocks.blocks[0];
    assert!(block.editable, "{:?}", block.reason);
    assert_eq!(block.source_lines.len(), 3);
    assert_eq!(
        block.text.as_deref(),
        Some("Original first line with a second line and the final line.")
    );
    assert!(!block.original_font_available);
    let request = change(
        &session,
        block,
        "A longer replacement paragraph with real word wrapping. Café, Ελληνικά and Кириллица stay searchable.\nAn explicit final line.",
    );
    let preview = session.paragraph_preview(&request).unwrap();
    assert!(preview.layout.fits, "{:?}", preview.layout.overflow);
    assert!(preview.layout.lines.len() >= 4);
    assert!(preview.layout.lines.iter().any(|line| line.hard_break));
    assert_eq!(session.bytes(), original, "preview is read-only");
    let rendered =
        Session::from_bytes(preview.render_pdf.unwrap(), "preview.pdf".into(), None, 3).unwrap();
    assert!(rendered.text_runs(0).unwrap().iter().any(|run| {
        run.text
            .as_deref()
            .is_some_and(|text| text.contains("replacement"))
    }));
    let after = session.edit_paragraph(&request).unwrap();
    assert!(after.can_undo);
    assert!(after.annotations.is_empty());
    let pdf = Document::load_mem(session.bytes()).unwrap();
    let text = session
        .text_runs(0)
        .unwrap()
        .iter()
        .filter_map(|run| run.text.as_deref())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!text.contains("Original first line"));
    assert!(!text.contains("with a second line"));
    assert!(text.contains("replacement"));
    assert!(text.contains("Independent right column"));
    assert!(
        !pdf.get_page_content(pdf.get_pages()[&1])
            .windows(b"Original first line".len())
            .any(|bytes| bytes == b"Original first line")
    );
    let current = first(&session);
    assert!(current.editable, "{:?}", current.reason);
    assert!(current.id.starts_with("folio-paragraph-"));
    let edited = session.bytes().to_vec();
    session.undo().unwrap();
    assert_eq!(session.bytes(), original);
    session.redo().unwrap();
    assert_eq!(session.bytes(), edited);
    assert!(
        session
            .edit_paragraph(&request)
            .unwrap_err()
            .to_string()
            .contains("epoch")
    );
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("reflow.pdf");
    session.save_as(&path).unwrap();
    let mut reopened = Session::from_path(path, 2).unwrap();
    let block = first(&reopened);
    assert_eq!(block.id, current.id);
    let mut request = change(
        &reopened,
        &block,
        "Shorter text.\n\nA preserved paragraph break.",
    );
    request.font = ReplacementFont::Original;
    // The original embedded subset has no new glyphs; explicit Noto conversion remains available.
    if reopened.paragraph_preview(&request).is_err() {
        request.font = ReplacementFont::Noto;
    }
    reopened.edit_paragraph(&request).unwrap();
    assert_eq!(first(&reopened).id, block.id);
    assert_eq!(
        first(&reopened).text.as_deref(),
        Some("Shorter text.\n\nA preserved paragraph break.")
    );
}

#[test]
fn overflow_and_invalid_glyphs_preserve_exact_bytes_revision_and_history() {
    let mut session = open(fixture(SOURCE));
    let block = first(&session);
    let before = session.bytes().to_vec();
    let mut request = change(
        &session,
        &block,
        "A long sentence with many words that must span multiple lines and never disappear.",
    );
    request.placement = [40.0, 60.0, 65.0, 14.0];
    let preview = session.paragraph_preview(&request).unwrap();
    assert!(!preview.layout.fits);
    assert!(preview.layout.overflow.is_some());
    assert!(
        session
            .edit_paragraph(&request)
            .unwrap_err()
            .to_string()
            .contains("overflow")
    );
    for text in ["unsupported \u{301}", "مرحبا", "bad\ttext", "漢字"] {
        request.text = text.into();
        assert!(session.edit_paragraph(&request).is_err());
        assert_eq!(session.bytes(), before);
        assert_eq!(session.revision(), 0);
        assert!(!session.info().unwrap().can_undo);
    }
    request.text = "Supercalifragilisticexpialidocious".into();
    request.placement = [40.0, 60.0, 20.0, 200.0];
    let preview = session.paragraph_preview(&request).unwrap();
    assert_eq!(
        preview.layout.lines.len(),
        1,
        "unbreakable words are not silently split"
    );
    assert!(!preview.layout.fits);
}

#[test]
fn preserves_quote_side_effects_and_text_state_inherited_by_the_next_object() {
    let mut session=open(fixture(b"0.2 0.4 0.6 rg BT /F1 12 Tf 17 TL 1 0 0 1 40 700 Tm [(First ) 20 (line)] TJ (Second line) ' 2 0.2 (Third line) \" ET BT 1 0 0 1 330 700 Tm (Following inherited text) Tj ET"));
    let target = first(&session);
    assert!(target.editable, "{:?}", target.reason);
    assert_eq!(target.source_lines.len(), 3);
    let following = session.text_runs(0).unwrap().pop().unwrap();
    let request = change(
        &session,
        &target,
        "Reflow preserves inherited state for the next object.",
    );
    session.edit_paragraph(&request).unwrap();
    let next = session
        .text_runs(0)
        .unwrap()
        .into_iter()
        .find(|run| run.text.as_deref() == Some("Following inherited text"))
        .unwrap();
    assert_eq!(next.font, following.font);
    assert_eq!(next.font_size, following.font_size);
    assert_eq!(next.origin, following.origin);
    let pdf = Document::load_mem(session.bytes()).unwrap();
    let content = Content::decode(&pdf.get_page_content(pdf.get_pages()[&1])).unwrap();
    let operators: Vec<_> = content
        .operations
        .iter()
        .map(|op| op.operator.as_str())
        .collect();
    assert!(operators.windows(3).any(|ops| ops == ["Tw", "Tc", "T*"]));
    let blocks = session.paragraph_blocks(0).unwrap();
    assert!(
        blocks.blocks.iter().any(
            |block| block.text.as_deref() == Some("Following inherited text") && block.editable
        )
    );
}

#[test]
fn detects_ambiguous_columns_tags_actualtext_clips_and_mixed_styles_without_mutating() {
    for (content, expected) in [
        (
            "BT /F1 12 Tf 40 700 Td (Left column) Tj 300 0 Td (Right column) Tj ET",
            "gaps",
        ),
        (
            "BT /F1 12 Tf 40 700 Td (First) Tj 0 -90 Td (Another paragraph) Tj ET",
            "baselines",
        ),
        (
            "/Span << /ActualText (Logical text) >> BDC BT /F1 12 Tf 40 700 Td (Visible source) Tj ET EMC",
            "ActualText",
        ),
        (
            "q 30 600 200 140 re W n BT /F1 12 Tf 40 700 Td (Clipped source) Tj ET Q",
            "clipping",
        ),
        (
            "BT /F1 12 Tf 40 700 Td (First) Tj 0 -16 Td /F1 14 Tf (Larger) Tj ET",
            "mixes",
        ),
        (
            "BT /F1 12 Tf 40 700 Td (First) Tj 0 -16 Td 0.8 g (Color change) Tj ET",
            "mixes",
        ),
    ] {
        let session = open(fixture(content.as_bytes()));
        let block = first(&session);
        assert!(!block.editable, "{content}");
        assert!(block.reason.unwrap().contains(expected), "{content}");
    }
    let mut pdf = fixture(SOURCE);
    pdf.catalog_mut()
        .unwrap()
        .set("StructTreeRoot", dictionary! {"Type"=>"StructTreeRoot"});
    let session = open(pdf);
    assert!(first(&session).reason.unwrap().contains("Tagged"));
}

#[test]
fn empty_text_is_a_real_persistent_edit_and_keeps_a_selectable_logical_source_box() {
    let mut session = open(fixture(SOURCE));
    let block = first(&session);
    let request = change(&session, &block, "");
    session.edit_paragraph(&request).unwrap();
    let target = first(&session);
    assert_eq!(target.text.as_deref(), Some(""));
    assert!(target.editable);
    let mut reopened =
        Session::from_bytes(session.bytes().to_vec(), "empty.pdf".into(), None, 2).unwrap();
    let block = first(&reopened);
    let request = change(&reopened, &block, "Text restored into the same box.");
    reopened.edit_paragraph(&request).unwrap();
    assert_eq!(first(&reopened).id, target.id);
}

#[test]
fn whitespace_and_utf8_ranges_reconstruct_the_exact_draft() {
    let session = open(fixture(SOURCE));
    let target = first(&session);
    let mut request = change(&session, &target, "  Café  beta gamma  \n\nlast line ");
    request.placement = [40.0, 60.0, 85.0, 210.0];
    let preview = session.paragraph_preview(&request).unwrap();
    let mut reconstructed = String::new();
    for line in &preview.layout.lines {
        assert_eq!(&request.text[line.start..line.end], line.text);
        reconstructed.push_str(&line.text);
        if line.hard_break {
            reconstructed.push('\n');
        }
        if let Some([x, y, w, h]) = line.ink_bounds {
            assert!(
                x >= -0.001
                    && y >= -0.001
                    && x + w <= preview.layout.width + 0.001
                    && y + h <= preview.layout.height + 0.001
            );
        }
    }
    assert_eq!(reconstructed, request.text);
}

#[test]
fn wrapping_whitespace_never_replaces_a_legal_boundary_with_an_overwide_space() {
    let session = open(fixture(SOURCE));
    let target = first(&session);
    for text in [
        "This real replacement uses a different font and size, but following source text must retain the inherited original font. STATE-NEW-94831.",
        "alpha                              beta gamma delta ",
        "One small word then a-hyphenated-word and more words.  ",
    ] {
        let mut request = change(&session, &target, text);
        request.placement = [40.0, 60.0, 250.0, 140.0];
        request.size = 14.0;
        request.leading = 19.0;
        let preview = session.paragraph_preview(&request).unwrap();
        assert!(preview.layout.fits, "{text}: {:?}", preview.layout.overflow);
        assert!(
            preview
                .layout
                .lines
                .iter()
                .all(|line| line.advance <= 250.001)
        );
        assert_eq!(
            preview
                .layout
                .lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<String>(),
            text
        );
    }
}

#[test]
fn workspace_draft_and_epoch_guards_prevent_stale_export_and_undo_resurrection() {
    let mut workspace = Workspace::default();
    let info = workspace
        .open_bytes(bytes(fixture(SOURCE)), "source.pdf".into())
        .unwrap();
    let source = workspace.document(info.id).unwrap();
    let request = change(source, &first(source), "Changed paragraph.");
    workspace.set_draft(info.id, true).unwrap();
    assert!(
        workspace
            .ensure_export_ready(info.id, info.revision)
            .is_err()
    );
    assert!(
        workspace
            .close_document(info.id, info.revision, false)
            .is_err()
    );
    assert!(workspace.has_unsaved_changes());
    let next = workspace
        .edit_paragraph(info.id, info.revision, request.clone())
        .unwrap();
    workspace.set_draft(info.id, false).unwrap();
    workspace
        .current_mut(next.id, next.revision)
        .unwrap()
        .undo()
        .unwrap();
    let original = workspace.document(info.id).unwrap();
    assert_eq!(original.revision(), info.revision);
    let before = original.bytes().to_vec();
    assert!(
        workspace
            .edit_paragraph(info.id, info.revision, request)
            .unwrap_err()
            .to_string()
            .contains("epoch")
    );
    assert_eq!(workspace.document(info.id).unwrap().bytes(), before);
    let info = workspace.document(info.id).unwrap().info().unwrap();
    workspace
        .current_mut(info.id, info.revision)
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
}

#[test]
fn original_embedded_font_is_reused_and_measured_with_negative_bearings_and_descenders() {
    let mut session = open(fixture(SOURCE));
    let block = first(&session);
    let mut request = change(&session, &block, "jgyJ Café Ελληνικά Кириллица");
    request.placement = [40.0, 60.0, 300.0, 150.0];
    request.size = 24.0;
    request.leading = 35.0;
    session.edit_paragraph(&request).unwrap();
    let block = first(&session);
    assert!(block.original_font_available);
    let before = Document::load_mem(session.bytes()).unwrap();
    let before_fonts: Vec<_> = before
        .objects
        .iter()
        .filter_map(|(id, value)| {
            value
                .as_dict()
                .ok()
                .filter(|value| {
                    value.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Type0")
                })
                .map(|_| *id)
        })
        .collect();
    let mut request = change(&session, &block, "gyjJ Café\nКириллица Ελληνικά");
    request.font = ReplacementFont::Original;
    request.size = 24.0;
    request.leading = 35.0;
    request.placement = [40.0, 60.0, 300.0, 160.0];
    let preview = session.paragraph_preview(&request).unwrap();
    assert!(preview.layout.fits, "{:?}", preview.layout.overflow);
    for line in &preview.layout.lines {
        let [x, y, width, height] = line.ink_bounds.unwrap();
        assert!(x >= 0.0 && y >= 0.0 && x + width <= 300.001 && y + height <= 160.001);
    }
    session.edit_paragraph(&request).unwrap();
    let after = Document::load_mem(session.bytes()).unwrap();
    let after_fonts: Vec<_> = after
        .objects
        .iter()
        .filter_map(|(id, value)| {
            value
                .as_dict()
                .ok()
                .filter(|value| {
                    value.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Type0")
                })
                .map(|_| *id)
        })
        .collect();
    assert_eq!(
        after_fonts, before_fonts,
        "Original mode must not re-embed or substitute the font program"
    );
    assert_eq!(first(&session).id, block.id);
    assert_eq!(first(&session).text, Some(request.text));
}

#[test]
fn crop_rotation_and_page_local_userunit_preserve_orientation_and_physical_boxes() {
    for rotation in [0, 90, 180, 270] {
        for unit in [0.75, 1.0, 1.5, 2.0] {
            let mut pdf = fixture(SOURCE);
            let page = pdf.get_pages()[&1];
            pdf.get_dictionary_mut(page)
                .unwrap()
                .set("Rotate", rotation);
            pdf.get_dictionary_mut(page).unwrap().set("UserUnit", unit);
            let mut session = open(pdf);
            let block = first(&session);
            assert!(
                block.editable,
                "rotation={rotation}, unit={unit}: {:?}",
                block.reason
            );
            assert_eq!(block.rotation, rotation);
            let mut request = change(
                &session,
                &block,
                "A short reflowed line.\nAnother actual line.",
            );
            let [x, y, w, h] = block.placement.unwrap();
            request.placement = if rotation % 180 == 0 {
                [x, y, w.max(240.0), h.max(100.0)]
            } else {
                [x, y, w.max(100.0), h.max(240.0)]
            };
            // Choose an explicit physical box safely inside every rotated visible page.
            request.placement[0] = 20.0;
            request.placement[1] = 20.0;
            session.edit_paragraph(&request).unwrap();
            let actual = first(&session);
            assert_eq!(actual.rotation, rotation);
            assert!(actual.editable, "{:?}", actual.reason);
            for (actual, expected) in actual.placement.unwrap().iter().zip(request.placement) {
                assert!((*actual - expected).abs() < 0.01, "{actual} vs {expected}");
            }
        }
    }
}

#[test]
fn isolates_one_nested_repeated_form_and_shared_page_stream_with_inherited_resources() {
    let mut pdf = fixture(b"");
    let first_page = pdf.get_pages()[&1];
    let parent = pdf
        .get_dictionary(first_page)
        .unwrap()
        .get(b"Parent")
        .unwrap()
        .as_reference()
        .unwrap();
    let resources = pdf
        .get_dictionary(parent)
        .unwrap()
        .get(b"Resources")
        .unwrap()
        .clone();
    let inner=pdf.add_object(Stream::new(dictionary! {
        "Type"=>"XObject","Subtype"=>"Form","BBox"=>numbers(&[0.0,0.0,260.0,170.0]),
    },b"BT /F1 12 Tf 16 TL 1 0 0 1 20 130 Tm (Nested first line) Tj T* (Nested second line) Tj ET".to_vec()));
    let outer = pdf.add_object(Stream::new(
        dictionary! {
            "Type"=>"XObject","Subtype"=>"Form","BBox"=>numbers(&[0.0,0.0,280.0,180.0]),
            "Matrix"=>numbers(&[1.0,0.0,0.0,1.0,5.0,5.0]),
            "Resources"=>dictionary! {"XObject"=>dictionary! {"Inner"=>inner},
                "Font"=>resources.as_dict().unwrap().get(b"Font").unwrap().clone()},
        },
        b"/Inner Do".to_vec(),
    ));
    let content = pdf.add_object(Stream::new(
        dictionary! {},
        b"q 1 0 0 1 10 400 cm /Outer Do Q q 1 0 0 1 300 400 cm /Outer Do Q".to_vec(),
    ));
    pdf.get_dictionary_mut(parent).unwrap().set(
        "Resources",
        dictionary! {"XObject"=>dictionary! {"Outer"=>outer}},
    );
    pdf.get_dictionary_mut(first_page)
        .unwrap()
        .set("Contents", vec![Object::Reference(content)]);
    let second = pdf.add_object(dictionary! {"Type"=>"Page","Parent"=>parent,"Contents"=>content});
    pdf.get_dictionary_mut(parent).unwrap().set(
        "Kids",
        vec![Object::Reference(first_page), Object::Reference(second)],
    );
    pdf.get_dictionary_mut(parent).unwrap().set("Count", 2);
    let mut session = open(pdf);
    let baseline = Document::load_mem(session.bytes()).unwrap();
    let original_inner = baseline.get_object(inner).unwrap().clone();
    let original_outer = baseline.get_object(outer).unwrap().clone();
    let blocks = session.paragraph_blocks(0).unwrap();
    assert_eq!(blocks.blocks.len(), 2);
    let target = &blocks.blocks[1];
    assert_eq!(target.path.len(), 3);
    assert!(target.editable, "{:?}", target.reason);
    let mut request = change(
        &session,
        target,
        "Only this invocation changes.\nThe other instances stay.",
    );
    request.placement = target.placement.unwrap();
    request.placement[2] = 230.0;
    request.placement[3] = 100.0;
    let before = session.bytes().to_vec();
    let mut clipped = request.clone();
    clipped.placement[0] += 30.0;
    assert!(
        session
            .edit_paragraph(&clipped)
            .unwrap_err()
            .to_string()
            .contains("Form BBoxes")
    );
    assert_eq!(session.bytes(), before);
    assert!(!session.info().unwrap().can_undo);
    session.edit_paragraph(&request).unwrap();
    let blocks = session.paragraph_blocks(0).unwrap();
    assert!(
        blocks.blocks[0]
            .text
            .as_deref()
            .unwrap()
            .contains("Nested first line")
    );
    assert_eq!(
        blocks.blocks[1].text.as_deref(),
        Some(request.text.as_str())
    );
    assert!(
        session
            .paragraph_blocks(1)
            .unwrap()
            .blocks
            .iter()
            .all(|block| block.text.as_deref().unwrap().contains("Nested first line"))
    );
    let pdf = Document::load_mem(session.bytes()).unwrap();
    assert_eq!(pdf.get_object(inner).unwrap(), &original_inner);
    assert_eq!(pdf.get_object(outer).unwrap(), &original_outer);
    assert_eq!(
        pdf.get_dictionary(second)
            .unwrap()
            .get(b"Contents")
            .unwrap(),
        &Object::Reference(content)
    );
}

fn encrypted(mut pdf: Document, version: u8, permissions: Permissions) -> Vec<u8> {
    pdf.trailer.set(
        "ID",
        vec![
            Object::string_literal("paragraph-synthetic-id"),
            Object::string_literal("paragraph-synthetic-id"),
        ],
    );
    let version = if version == 1 {
        EncryptionVersion::V1 {
            document: &pdf,
            owner_password: "owner",
            user_password: "reader",
            permissions,
        }
    } else {
        EncryptionVersion::V2 {
            document: &pdf,
            owner_password: "owner",
            user_password: "reader",
            permissions,
            key_length: 128,
        }
    };
    pdf.encrypt(&EncryptionState::try_from(version).unwrap())
        .unwrap();
    bytes(pdf)
}

#[test]
fn legacy_protected_paragraphs_preserve_exact_history_passwords_and_file_identifier() {
    for version in [1, 2] {
        let original = encrypted(fixture(SOURCE), version, Permissions::all());
        let mut session = Session::from_bytes_with_password(
            original.clone(),
            "legacy.pdf".into(),
            None,
            1,
            Some("reader"),
        )
        .unwrap();
        let request = change(
            &session,
            &first(&session),
            "A protected paragraph is real wrapped PDF text. Café.",
        );
        let preview = session.paragraph_preview(&request).unwrap();
        assert!(preview.layout.fits);
        assert_eq!(
            session.bytes(),
            original,
            "renderer-only preview cannot change protected bytes"
        );
        session.edit_paragraph(&request).unwrap();
        let edited = session.bytes().to_vec();
        assert!(Document::load_mem(&edited).unwrap().is_encrypted());
        for password in ["reader", "owner"] {
            let before =
                Document::load_mem_with_options(&original, LoadOptions::with_password(password))
                    .unwrap();
            let after =
                Document::load_mem_with_options(&edited, LoadOptions::with_password(password))
                    .unwrap();
            assert_eq!(
                before.trailer.get(b"ID").unwrap(),
                after.trailer.get(b"ID").unwrap()
            );
            assert!(after.was_encrypted());
            let reopened = Session::from_bytes_with_password(
                edited.clone(),
                "legacy.pdf".into(),
                None,
                2,
                Some(password),
            )
            .unwrap();
            assert_eq!(
                first(&reopened).text.as_deref(),
                Some(request.text.as_str())
            );
        }
        session.undo().unwrap();
        assert_eq!(session.bytes(), original);
        session.redo().unwrap();
        assert_eq!(session.bytes(), edited);
        let mut bad = change(
            &session,
            &first(&session),
            "Overflow must not replace protected state.",
        );
        bad.placement = [40.0, 60.0, 30.0, 4.0];
        assert!(session.edit_paragraph(&bad).is_err());
        assert_eq!(session.bytes(), edited);
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("protected-reflow.pdf");
        session.save_as(&path).unwrap();
        let mut reopened = Session::from_bytes_with_password(
            std::fs::read(path).unwrap(),
            "protected-reflow.pdf".into(),
            None,
            3,
            Some("owner"),
        )
        .unwrap();
        let request = change(&reopened, &first(&reopened), "A second protected edit.");
        reopened.edit_paragraph(&request).unwrap();
        assert!(Document::load_mem(reopened.bytes()).unwrap().is_encrypted());
    }
}

#[test]
fn general_edit_authority_signed_and_archival_restrictions_cannot_be_bypassed_by_paragraph_apis() {
    let restricted = encrypted(
        fixture(SOURCE),
        2,
        Permissions::COPYABLE | Permissions::FILLABLE,
    );
    let mut session = Session::from_bytes_with_password(
        restricted.clone(),
        "fill-only.pdf".into(),
        None,
        1,
        Some("reader"),
    )
    .unwrap();
    assert!(session.info().unwrap().permissions.fill_forms_allowed);
    let block = first(&session);
    assert!(!block.editable);
    let request = change(&session, &block, "Not authorized for source editing.");
    assert!(session.paragraph_preview(&request).is_err());
    assert!(session.edit_paragraph(&request).is_err());
    assert_eq!(session.bytes(), restricted);
    for marker in ["signed", "certified", "archival"] {
        let mut pdf = fixture(SOURCE);
        if marker == "archival" {
            pdf.catalog_mut().unwrap().set("FolioRasterArchive", true);
        } else if marker == "certified" {
            pdf.catalog_mut().unwrap().set("Perms", dictionary! {});
        } else {
            let signature = pdf.add_object(dictionary! {"Type"=>"Sig","ByteRange"=>vec![0.into(),1.into(),2.into(),3.into()],
                "Contents"=>Object::string_literal("synthetic read-only marker; not a valid signature")});
            pdf.catalog_mut()
                .unwrap()
                .set("SyntheticSignature", signature);
        }
        let mut session = open(pdf);
        let before = session.bytes().to_vec();
        let block = first(&session);
        assert!(!block.editable, "{marker}");
        let request = change(&session, &block, "A forbidden edit.");
        assert!(session.paragraph_preview(&request).is_err(), "{marker}");
        assert!(session.edit_paragraph(&request).is_err(), "{marker}");
        assert_eq!(session.bytes(), before);
        assert!(!session.info().unwrap().can_undo);
    }
}

#[test]
fn managed_logical_metadata_is_checked_against_real_glyph_operations_before_reediting() {
    let mut session = open(fixture(SOURCE));
    let request = change(&session, &first(&session), "Managed actual source text.");
    session.edit_paragraph(&request).unwrap();
    let mut pdf = Document::load_mem(session.bytes()).unwrap();
    let form = pdf
        .objects
        .values_mut()
        .find_map(|value| {
            value
                .as_stream_mut()
                .ok()
                .filter(|stream| stream.dict.has(b"FolioParagraph"))
        })
        .unwrap();
    form.dict
        .get_mut(b"FolioParagraph")
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set(
            "Text",
            lopdf::text_string("Tampered metadata, not visible source."),
        );
    let session = open(pdf);
    let error = session.paragraph_blocks(0).unwrap_err().to_string();
    assert!(
        error.contains("metadata") || error.contains("encode"),
        "{error}"
    );
}

#[test]
fn enclosing_form_bounds_and_unstable_absolute_origins_fail_without_partial_source_changes() {
    let mut pdf = fixture(SOURCE);
    let page = pdf.get_pages()[&1];
    pdf.get_dictionary_mut(page).unwrap().set(
        "MediaBox",
        numbers(&[100000000.0, 100000000.0, 100001024.0, 100001024.0]),
    );
    pdf.get_dictionary_mut(page).unwrap().set(
        "CropBox",
        numbers(&[100000000.0, 100000000.0, 100001024.0, 100001024.0]),
    );
    let stream = pdf.add_object(Stream::new(dictionary! {},b"BT /F1 12 Tf 16 TL 1 0 0 1 100000081 100000700 Tm (Finite source line) Tj T* (Another source line) Tj ET".to_vec()));
    pdf.get_dictionary_mut(page)
        .unwrap()
        .set("Contents", stream);
    let mut session = open(pdf);
    let before = session.bytes().to_vec();
    let block = first(&session);
    assert!(block.editable, "{:?}", block.reason);
    let mut request = change(
        &session,
        &block,
        "A safe refusal rather than rounded placement.",
    );
    request.placement = [85.0, 60.0, 240.0, 110.0];
    assert!(
        session
            .edit_paragraph(&request)
            .unwrap_err()
            .to_string()
            .contains("precision")
    );
    assert_eq!(session.bytes(), before);
}

#[test]
fn write_original_synthetic_paragraph_fixtures_when_requested() {
    let Ok(folder) = std::env::var("FOLIO_PARAGRAPH_EVIDENCE_DIR") else {
        return;
    };
    let folder = std::path::Path::new(&folder);
    std::fs::create_dir_all(folder).unwrap();
    let mut pdf = fixture(SOURCE);
    let first = pdf.get_pages()[&1];
    let parent = pdf
        .get_dictionary(first)
        .unwrap()
        .get(b"Parent")
        .unwrap()
        .as_reference()
        .unwrap();
    let image = pdf.add_object(Stream::new(
        dictionary! {
            "Type"=>"XObject","Subtype"=>"Image","Width"=>2,"Height"=>1,
            "ColorSpace"=>"DeviceRGB","BitsPerComponent"=>8,
        },
        vec![240, 30, 40, 20, 60, 230],
    ));
    pdf.get_dictionary_mut(parent)
        .unwrap()
        .get_mut(b"Resources")
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("XObject", dictionary! {"KeepImage"=>image});
    let mut source = SOURCE.to_vec();
    source.extend_from_slice(b" q 100 0 0 50 330 200 cm /KeepImage Do Q BT /F1 10 Tf 1 0 0 1 40 60 Tm (KEEP-FOOTER-90451) Tj ET");
    let content = pdf.add_object(Stream::new(dictionary! {}, source));
    pdf.get_dictionary_mut(first)
        .unwrap()
        .set("Contents", content);
    let mut pages = vec![first];
    for (rotation, unit) in [(90, 1.5_f32), (180, 0.75), (270, 2.0)] {
        pages.push(pdf.add_object(dictionary! {"Type"=>"Page","Parent"=>parent,"Contents"=>content,"Rotate"=>rotation,"UserUnit"=>unit}));
    }
    pdf.get_dictionary_mut(parent).unwrap().set(
        "Kids",
        pages
            .iter()
            .map(|id| Object::Reference(*id))
            .collect::<Vec<_>>(),
    );
    pdf.get_dictionary_mut(parent)
        .unwrap()
        .set("Count", pages.len() as i64);
    let appearance = pdf.add_object(Stream::new(
        dictionary! {
            "Type"=>"XObject","Subtype"=>"Form","BBox"=>numbers(&[0.0,0.0,60.0,40.0]),
        },
        b"0.9 0.6 0.1 rg 0 0 60 40 re f".to_vec(),
    ));
    let annotation = pdf.add_object(dictionary! {
        "Type"=>"Annot","Subtype"=>"Stamp","P"=>first,"Rect"=>numbers(&[400.0,100.0,460.0,140.0]),
        "F"=>4,"AP"=>dictionary! {"N"=>appearance},
    });
    pdf.get_dictionary_mut(first)
        .unwrap()
        .set("Annots", vec![Object::Reference(annotation)]);
    let payload = pdf.add_object(Stream::new(
        dictionary! {"Type"=>"EmbeddedFile"},
        b"Original synthetic attachment remains byte-identical.".to_vec(),
    ));
    let attachment = pdf.add_object(
        dictionary! {"Type"=>"Filespec","F"=>Object::string_literal("keep.txt"),
        "UF"=>lopdf::text_string("keep.txt"),"EF"=>dictionary! {"F"=>payload}},
    );
    pdf.catalog_mut().unwrap().set(
        "Names",
        dictionary! {"EmbeddedFiles"=>dictionary! {
            "Names"=>vec![Object::string_literal("keep.txt"),Object::Reference(attachment)],
        }},
    );
    std::fs::write(folder.join("source.pdf"), bytes(pdf)).unwrap();
    let mut embedded = open(fixture(SOURCE));
    let request = change(
        &embedded,
        &first_block(&embedded),
        "Original embedded paragraph. Café Ω Привет.\nA separate logical line.",
    );
    embedded.edit_paragraph(&request).unwrap();
    std::fs::write(folder.join("embedded.pdf"), embedded.bytes()).unwrap();
    fn first_block(session: &Session) -> ParagraphBlock {
        session.paragraph_blocks(0).unwrap().blocks.remove(0)
    }
}
