use folio_core::{
    archive::{Request, SourceRef},
    document::{DocumentInfo, Session},
    output::{OutputKind, OutputRequest},
    workspace::Workspace,
};

fn source(workspace: &mut Workspace) -> DocumentInfo {
    workspace
        .open_bytes(
            Session::blank(72.0, 96.0, 1).unwrap().bytes().to_vec(),
            "Original roadmap integration.pdf".into(),
        )
        .unwrap()
}

fn replacement_png() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[220, 22, 129]).unwrap();
        writer.finish().unwrap();
    }
    bytes
}

fn archive_request(info: &DocumentInfo) -> Request {
    Request {
        source: SourceRef {
            document_id: info.id,
            revision: info.revision,
        },
        pages: vec![0],
        dpi: 150,
        acknowledge_loss: true,
        allow_unencrypted: false,
        acknowledge_unsigned: false,
    }
}

fn docx_request(info: &DocumentInfo) -> OutputRequest {
    OutputRequest {
        document_id: info.id,
        revision: info.revision,
        kind: OutputKind::Docx,
        pages: vec![0],
        dpi: None,
        quality: None,
        allow_unencrypted: false,
    }
}

#[test]
fn replacement_staging_invalidates_technical_and_docx_jobs_even_after_cancel_restores_readiness() {
    let mut workspace = Workspace::default();
    let info = source(&mut workspace);
    let before = workspace.document(info.id).unwrap().bytes().to_vec();
    let docx = workspace.begin_output(docx_request(&info)).unwrap();
    let inspection = workspace
        .inspect_technical(SourceRef {
            document_id: info.id,
            revision: info.revision,
        })
        .unwrap();
    let archive = workspace.begin_archive(archive_request(&info)).unwrap();
    let replacement = workspace
        .stage_image_replacement(info.id, info.revision, &replacement_png())
        .unwrap();
    assert!(workspace.check_inspection(inspection.inspection_id).is_err());
    assert!(workspace.check_archive(archive.archive_id).is_err());
    assert!(workspace.check_output(docx.job_id).is_err());
    assert!(workspace.has_unsaved_changes());
    assert!(workspace.ensure_export_ready(info.id, info.revision).is_err());
    assert!(
        workspace
            .cancel_image_replacement(info.id, replacement.asset_id)
            .unwrap()
    );
    assert!(workspace.ensure_export_ready(info.id, info.revision).is_ok());
    assert!(workspace.check_inspection(inspection.inspection_id).is_err());
    assert!(workspace.check_archive(archive.archive_id).is_err());
    assert!(workspace.check_output(docx.job_id).is_err());
    assert_eq!(workspace.document(info.id).unwrap().bytes(), before);
    assert!(!workspace.has_unsaved_changes());
}

#[test]
fn rejected_replacement_bytes_do_not_create_a_draft_or_cancel_valid_technical_jobs() {
    let mut workspace = Workspace::default();
    let info = source(&mut workspace);
    let docx = workspace.begin_output(docx_request(&info)).unwrap();
    let inspection = workspace
        .inspect_technical(SourceRef {
            document_id: info.id,
            revision: info.revision,
        })
        .unwrap();
    let archive = workspace.begin_archive(archive_request(&info)).unwrap();
    assert!(
        workspace
            .stage_image_replacement(info.id, info.revision, b"Not a PNG or JPEG")
            .is_err()
    );
    assert!(workspace.check_inspection(inspection.inspection_id).is_ok());
    assert!(workspace.check_archive(archive.archive_id).is_ok());
    assert!(workspace.check_output(docx.job_id).is_ok());
    assert!(workspace.ensure_export_ready(info.id, info.revision).is_ok());
    assert!(workspace.cancel_archive(archive.archive_id));
    assert!(workspace.cancel_output(docx.job_id));
    assert!(!workspace.has_unsaved_changes());
}

#[test]
fn staged_source_replacement_blocks_scoped_close_signing_and_every_common_export_until_discard() {
    let mut workspace = Workspace::default();
    let info = source(&mut workspace);
    let replacement = workspace
        .stage_image_replacement(info.id, info.revision, &replacement_png())
        .unwrap();
    assert!(workspace.check_signing(info.id, info.revision).is_err());
    assert!(workspace.check_form_flattening(info.id, info.revision).is_err());
    assert!(workspace.ensure_technical_ready(info.id, info.revision).is_err());
    assert!(
        workspace
            .close_document(info.id, info.revision, false)
            .is_err()
    );
    workspace
        .cancel_image_replacement(info.id, replacement.asset_id)
        .unwrap();
    assert!(workspace.check_signing(info.id, info.revision).is_ok());
    assert!(
        workspace
            .close_document(info.id, info.revision, false)
            .is_ok()
    );
}
