use folio_core::{
    document::{DocumentInfo, Edit, Session},
    workspace::Workspace,
};
use lopdf::{Document, Object, dictionary};

fn open_blank(workspace: &mut Workspace, name: &str) -> DocumentInfo {
    let bytes = Session::blank(72.0, 72.0, 1).unwrap().bytes().to_vec();
    workspace.open_bytes(bytes, name.into()).unwrap()
}

#[test]
fn pending_drafts_block_only_their_own_documents_exports_without_changing_bytes() {
    let mut workspace = Workspace::default();
    let first = open_blank(&mut workspace, "First.pdf");
    let second = open_blank(&mut workspace, "Second.pdf");
    let first_bytes = workspace.document(first.id).unwrap().bytes().to_vec();
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
    assert_eq!(workspace.document(first.id).unwrap().bytes(), first_bytes);
    assert_eq!(workspace.document(second.id).unwrap().bytes(), second_bytes);
    assert!(!workspace.document(first.id).unwrap().is_dirty());
    assert!(workspace.has_unsaved_changes());

    workspace.set_draft(first.id, false).unwrap();
    assert!(
        workspace
            .ensure_export_ready(first.id, first.revision)
            .is_ok()
    );
    assert!(!workspace.has_unsaved_changes());
}

#[test]
fn clearing_a_draft_does_not_authorize_an_old_revision() {
    let mut workspace = Workspace::default();
    let first = open_blank(&mut workspace, "First.pdf");
    let changed = workspace
        .current_mut(first.id, first.revision)
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    workspace.set_draft(first.id, true).unwrap();
    assert!(
        workspace
            .ensure_export_ready(first.id, changed.revision)
            .is_err()
    );
    workspace.set_draft(first.id, false).unwrap();
    assert!(
        workspace
            .ensure_export_ready(first.id, first.revision)
            .is_err()
    );
    assert!(
        workspace
            .ensure_export_ready(first.id, changed.revision)
            .is_ok()
    );
}

#[test]
fn draft_guard_does_not_prevent_rendering_the_current_working_document() {
    let mut workspace = Workspace::default();
    let first = open_blank(&mut workspace, "First.pdf");
    workspace.set_draft(first.id, true).unwrap();
    let rendered = workspace
        .check_revision(first.id, first.revision)
        .unwrap()
        .render_bytes()
        .unwrap();
    assert!(rendered.starts_with(b"%PDF-"));
    assert!(
        workspace
            .ensure_export_ready(first.id, first.revision)
            .is_err()
    );
}

#[test]
fn clearing_or_closing_one_tab_does_not_clear_another_tabs_draft() {
    let mut workspace = Workspace::default();
    let first = open_blank(&mut workspace, "First.pdf");
    let second = open_blank(&mut workspace, "Second.pdf");
    workspace.set_draft(first.id, true).unwrap();
    workspace.set_draft(second.id, true).unwrap();
    workspace.set_draft(first.id, false).unwrap();
    workspace
        .close_document(first.id, first.revision, false)
        .unwrap();

    assert!(
        workspace
            .ensure_export_ready(first.id, first.revision)
            .is_err()
    );
    assert!(
        workspace
            .ensure_export_ready(second.id, second.revision)
            .is_err()
    );
    assert!(workspace.has_unsaved_changes());
    workspace
        .close_document(second.id, second.revision, true)
        .unwrap();
    assert!(!workspace.has_unsaved_changes());
}

#[test]
fn unchanged_signed_copies_are_not_confused_with_pdf_mutations() {
    let mut pdf = Document::load_mem(Session::blank(72.0, 72.0, 1).unwrap().bytes()).unwrap();
    let signature = pdf.add_object(dictionary! {
        "Type" => "Sig",
        "Filter" => "Adobe.PPKLite",
        "SubFilter" => "adbe.pkcs7.detached",
        "ByteRange" => vec![Object::Integer(0); 4],
        "Contents" => Object::string_literal("Synthetic unsupported signature"),
    });
    pdf.catalog_mut()
        .unwrap()
        .set("SyntheticSignature", signature);
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).unwrap();

    let mut workspace = Workspace::default();
    let signed = workspace
        .open_bytes(bytes.clone(), "Signed.pdf".into())
        .unwrap();
    assert!(signed.read_only_reason.is_some());
    assert_eq!(
        workspace
            .ensure_export_ready(signed.id, signed.revision)
            .unwrap()
            .bytes(),
        bytes
    );
    workspace.set_draft(signed.id, true).unwrap();
    assert!(
        workspace
            .ensure_export_ready(signed.id, signed.revision)
            .is_err()
    );
}

#[test]
fn staged_images_block_exports_and_signing_before_a_frontend_draft_update() {
    let mut workspace = Workspace::default();
    let document = open_blank(&mut workspace, "Image draft.pdf");
    assert!(
        workspace
            .check_signing(document.id, document.revision)
            .is_ok()
    );
    let original = workspace.document(document.id).unwrap().bytes().to_vec();
    let mut image = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut image, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[0, 128, 255, 128]).unwrap();
        writer.finish().unwrap();
    }
    let asset = workspace
        .stage_authoring_image(document.id, document.revision, &image)
        .unwrap();
    assert!(
        workspace
            .ensure_export_ready(document.id, document.revision)
            .is_err()
    );
    assert!(
        workspace
            .check_signing(document.id, document.revision)
            .is_err()
    );
    assert!(
        workspace
            .begin_ocr(
                document.id,
                document.revision,
                folio_core::ocr::Options {
                    pages: vec![0],
                    dpi: 150,
                    language: "eng".into(),
                    existing_text: folio_core::ocr::ExistingTextPolicy::Force,
                },
            )
            .is_err()
    );
    assert_eq!(workspace.document(document.id).unwrap().bytes(), original);
    workspace
        .cancel_authoring_image(document.id, asset.asset_id)
        .unwrap();
    assert!(
        workspace
            .ensure_export_ready(document.id, document.revision)
            .is_ok()
    );
    assert!(
        workspace
            .check_signing(document.id, document.revision)
            .is_ok()
    );
}
