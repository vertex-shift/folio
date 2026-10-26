#[path = "../examples/output_fixtures.rs"]
mod fixtures;

use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use folio_core::{
    conversion::{
        CreationKind, CreationSettings, CreationStatus, MAX_DOCX_PAGE_TEXT_BYTES,
        MAX_TEXT_FILE_BYTES,
    },
    document::Edit,
    output::{OutputKind, OutputRequest},
    workspace::{OpenOutcome, Workspace},
};
use lopdf::{Document, encryption::Permissions};

fn settings() -> CreationSettings {
    CreationSettings {
        width: 612.0,
        height: 792.0,
        margin: 36.0,
        font_size: 12.0,
    }
}

fn wait(workspace: &mut Workspace, job: u64) -> Result<CreationStatus, String> {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        let state = workspace
            .creation_status(job)
            .map_err(|error| error.to_string())?;
        if state.phase == "ready" {
            return Ok(state);
        }
        assert!(Instant::now() < until, "PDF preparation timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn png() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[255, 0, 0, 128, 0, 255, 0, 255])
            .unwrap();
    }
    bytes
}

fn docx_request(workspace: &Workspace, pages: Vec<usize>) -> OutputRequest {
    let info = workspace.current().unwrap().info().unwrap();
    OutputRequest {
        document_id: info.id,
        revision: info.revision,
        kind: OutputKind::Docx,
        pages,
        dpi: None,
        quality: None,
        allow_unencrypted: true,
    }
}

fn source(bytes: Vec<u8>, password: Option<&str>) -> Workspace {
    let mut workspace = Workspace::default();
    if let OpenOutcome::PasswordRequired { request_id, .. } = workspace
        .request_open_bytes(bytes, "Synthetic.pdf".into())
        .unwrap()
    {
        assert!(matches!(
            workspace
                .unlock_document(request_id, password.unwrap())
                .unwrap(),
            OpenOutcome::Opened { .. }
        ));
    }
    workspace
}

fn parts(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut result = BTreeMap::new();
    let mut offset = 0;
    let u16_at = |index| u16::from_le_bytes(bytes[index..index + 2].try_into().unwrap()) as usize;
    let u32_at = |index| u32::from_le_bytes(bytes[index..index + 4].try_into().unwrap());
    while u32_at(offset) == 0x04034b50 {
        assert_eq!(u16_at(offset + 8), 0);
        let length = u32_at(offset + 18) as usize;
        let name_length = u16_at(offset + 26);
        let extra = u16_at(offset + 28);
        let name = std::str::from_utf8(&bytes[offset + 30..offset + 30 + name_length]).unwrap();
        let start = offset + 30 + name_length + extra;
        let content = &bytes[start..start + length];
        assert_eq!(crc32fast::hash(content), u32_at(offset + 14));
        let mut reader = quick_xml::Reader::from_reader(content);
        loop {
            if reader.read_event().unwrap() == quick_xml::events::Event::Eof {
                break;
            }
        }
        result.insert(name.into(), content.to_vec());
        offset = start + length;
    }
    assert_eq!(u32_at(offset), 0x02014b50);
    result
}

#[test]
fn creation_is_a_new_dirty_tab_with_no_source_or_history_changes_and_real_unicode_pages() {
    let mut workspace = source(fixtures::synthetic_pdf(2, false), None);
    let original = workspace.current().unwrap().info().unwrap();
    let changed = workspace
        .current_mut(original.id, original.revision)
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    let original_bytes = workspace.document(original.id).unwrap().bytes().to_vec();
    workspace.set_draft(original.id, true).unwrap();
    let plan = workspace.begin_creation(CreationKind::Text).unwrap();
    let first = format!(
        "\u{feff}Zoë café Ελληνικά Привет <w:t>&quotes\r\n{}\rEnd\tmarker",
        "Wrapped text 87492 ".repeat(2500)
    );
    workspace
        .stage_creation_file(plan.job_id, "First.txt".into(), first.into_bytes())
        .unwrap();
    workspace
        .stage_creation_file(
            plan.job_id,
            "Second.txt".into(),
            b"SECOND FILE 29407".to_vec(),
        )
        .unwrap();
    workspace
        .prepare_creation(plan.job_id, settings(), vec![1, 0])
        .unwrap();
    let ready = wait(&mut workspace, plan.job_id).unwrap();
    assert!(ready.page_count.unwrap() > 3);
    assert_eq!(workspace.snapshot().unwrap().documents.len(), 1);
    let created = workspace.commit_creation(plan.job_id).unwrap();
    assert_ne!(created.id, original.id);
    assert!(created.dirty && !created.can_undo && !created.can_redo);
    assert!(
        workspace
            .document(created.id)
            .unwrap()
            .source_path
            .is_none()
    );
    assert!(workspace.document(created.id).unwrap().saved_path.is_none());
    let pdf = Document::load_mem(workspace.document(created.id).unwrap().bytes()).unwrap();
    assert_eq!(pdf.get_pages().len(), created.page_count);
    assert!(
        pdf.extract_text(&[1])
            .unwrap()
            .contains("SECOND FILE 29407")
    );
    let text = pdf
        .extract_text(&(2..=created.page_count as u32).collect::<Vec<_>>())
        .unwrap();
    for expected in [
        "Zoë café Ελληνικά Привет <w:t>&quotes",
        "Wrapped text 87492",
        "End    marker",
    ] {
        assert!(text.contains(expected), "{expected}: {text}");
    }
    assert!(!text.contains('\u{feff}') && !text.contains('\r'));
    assert_eq!(
        workspace.document(original.id).unwrap().bytes(),
        original_bytes
    );
    assert_eq!(
        workspace.document(original.id).unwrap().revision(),
        changed.revision
    );
    assert!(
        workspace
            .document(original.id)
            .unwrap()
            .info()
            .unwrap()
            .can_undo
    );
    assert!(
        workspace
            .ensure_export_ready(original.id, changed.revision)
            .is_err()
    );
}

#[test]
fn image_creation_has_real_page_content_rgb_soft_mask_and_requested_layout() {
    let mut workspace = Workspace::default();
    let plan = workspace.begin_creation(CreationKind::Images).unwrap();
    workspace
        .stage_creation_file(plan.job_id, "Alpha.png".into(), png())
        .unwrap();
    let settings = CreationSettings {
        width: 792.0,
        height: 612.0,
        margin: 72.0,
        ..settings()
    };
    workspace
        .prepare_creation(plan.job_id, settings, vec![0])
        .unwrap();
    wait(&mut workspace, plan.job_id).unwrap();
    let info = workspace.commit_creation(plan.job_id).unwrap();
    let pdf = Document::load_mem(workspace.document(info.id).unwrap().bytes()).unwrap();
    let page = *pdf.get_pages().values().next().unwrap();
    assert!(!pdf.get_dictionary(page).unwrap().has(b"Annots"));
    let content = lopdf::content::Content::decode(&pdf.get_page_content(page)).unwrap();
    let matrix = &content
        .operations
        .iter()
        .find(|op| op.operator == "cm")
        .unwrap()
        .operands;
    assert_eq!(
        matrix
            .iter()
            .map(|value| value.as_float().unwrap())
            .collect::<Vec<_>>(),
        vec![648.0, 0.0, 0.0, 324.0, 72.0, 144.0]
    );
    let resources = pdf.get_page_resources(page).unwrap().0.unwrap();
    let images = resources.get(b"XObject").unwrap().as_dict().unwrap();
    let image = pdf
        .get_object(images.get(b"Image0").unwrap().as_reference().unwrap())
        .unwrap()
        .as_stream()
        .unwrap();
    assert_eq!(
        image.dict.get(b"ColorSpace").unwrap().as_name().unwrap(),
        b"DeviceRGB"
    );
    assert_eq!(image.get_plain_content().unwrap(), [255, 0, 0, 0, 255, 0]);
    let alpha = pdf
        .get_object(image.dict.get(b"SMask").unwrap().as_reference().unwrap())
        .unwrap()
        .as_stream()
        .unwrap();
    assert_eq!(alpha.get_plain_content().unwrap(), [128, 255]);
}

#[test]
fn creation_refuses_invalid_utf8_scripts_controls_images_and_limits_without_partial_tabs() {
    for (kind, name, bytes, message) in [
        (CreationKind::Text, "bad.txt", vec![0xff], "strict UTF-8"),
        (
            CreationKind::Text,
            "Arabic.txt",
            "مرحبا".as_bytes().to_vec(),
            "unsupported",
        ),
        (
            CreationKind::Text,
            "Control.txt",
            b"Control\0text".to_vec(),
            "unsupported",
        ),
        (
            CreationKind::Text,
            "Empty.txt",
            b"\xef\xbb\xbf\r\n \t".to_vec(),
            "no readable text",
        ),
        (
            CreationKind::Images,
            "fake.png",
            b"not an image".to_vec(),
            "Only still PNG",
        ),
    ] {
        let mut workspace = source(fixtures::synthetic_pdf(1, false), None);
        let original = workspace.current().unwrap().bytes().to_vec();
        let plan = workspace.begin_creation(kind).unwrap();
        workspace
            .stage_creation_file(
                plan.job_id,
                "Valid".into(),
                if kind == CreationKind::Text {
                    b"Valid".to_vec()
                } else {
                    png()
                },
            )
            .unwrap();
        workspace
            .stage_creation_file(plan.job_id, name.into(), bytes)
            .unwrap();
        workspace
            .prepare_creation(plan.job_id, settings(), vec![0, 1])
            .unwrap();
        assert!(
            wait(&mut workspace, plan.job_id)
                .unwrap_err()
                .contains(message)
        );
        assert!(workspace.commit_creation(plan.job_id).is_err());
        assert_eq!(workspace.snapshot().unwrap().documents.len(), 1);
        assert_eq!(workspace.current().unwrap().bytes(), original);
        assert!(!workspace.has_unsaved_changes());
    }
    let mut workspace = Workspace::default();
    let plan = workspace.begin_creation(CreationKind::Text).unwrap();
    assert!(
        workspace
            .stage_creation_file(
                plan.job_id,
                "large.txt".into(),
                vec![b'a'; MAX_TEXT_FILE_BYTES + 1]
            )
            .is_err()
    );
    assert!(workspace.creation_plan(plan.job_id).is_err());
    for setting in [
        CreationSettings {
            width: f32::NAN,
            ..settings()
        },
        CreationSettings {
            width: 143.9,
            ..settings()
        },
        CreationSettings {
            height: 1440.1,
            ..settings()
        },
        CreationSettings {
            margin: 145.0,
            ..settings()
        },
        CreationSettings {
            width: 144.0,
            margin: 37.0,
            ..settings()
        },
        CreationSettings {
            font_size: 7.9,
            ..settings()
        },
    ] {
        assert!(setting.validate().is_err());
    }
    assert!(
        CreationSettings {
            width: 144.0,
            height: 1440.0,
            margin: 36.0,
            font_size: 36.0
        }
        .validate()
        .is_ok()
    );
}

#[test]
fn creation_cancellation_order_and_page_boundaries_do_not_leave_jobs_or_partial_documents() {
    let mut workspace = Workspace::default();
    let plan = workspace.begin_creation(CreationKind::Text).unwrap();
    assert!(workspace.has_unsaved_changes());
    assert!(workspace.begin_creation(CreationKind::Images).is_err());
    workspace
        .stage_creation_file(plan.job_id, "Cancel.txt".into(), b"Cancel".to_vec())
        .unwrap();
    workspace
        .prepare_creation(plan.job_id, settings(), vec![0])
        .unwrap();
    assert!(workspace.cancel_creation(plan.job_id).unwrap());
    assert!(!workspace.cancel_creation(plan.job_id).unwrap());
    assert!(workspace.commit_creation(plan.job_id).is_err());
    assert!(!workspace.has_unsaved_changes());
    let plan = workspace.begin_creation(CreationKind::Text).unwrap();
    workspace
        .stage_creation_file(plan.job_id, "Order.txt".into(), b"Order".to_vec())
        .unwrap();
    assert!(
        workspace
            .prepare_creation(plan.job_id, settings(), vec![0, 0])
            .is_err()
    );
    assert!(!workspace.has_unsaved_changes());
    let plan = workspace.begin_creation(CreationKind::Text).unwrap();
    workspace
        .stage_creation_file(
            plan.job_id,
            "Pages.txt".into(),
            format!("Text\n{}", "\n".repeat(1000)).into_bytes(),
        )
        .unwrap();
    workspace
        .prepare_creation(
            plan.job_id,
            CreationSettings {
                width: 144.0,
                height: 144.0,
                margin: 36.0,
                font_size: 36.0,
            },
            vec![0],
        )
        .unwrap();
    assert!(
        wait(&mut workspace, plan.job_id)
            .unwrap_err()
            .contains("500 pages")
    );
    assert!(workspace.commit_creation(plan.job_id).is_err());
}

#[test]
fn docx_is_real_crc_checked_opc_xml_with_editable_unicode_and_requested_page_order() {
    let mut workspace = source(fixtures::synthetic_pdf(4, false), None);
    let before = workspace.current().unwrap().bytes().to_vec();
    let plan = workspace
        .begin_output(docx_request(&workspace, vec![2, 0, 1]))
        .unwrap();
    for (index, text) in [
        "Page 3 Ελληνικά Привет",
        "Page 1 <w:t>&\"' café\nSecond\tparagraph",
        "Page 2 87492",
    ]
    .iter()
    .enumerate()
    {
        workspace
            .stage_output_page(plan.job_id, index, text.as_bytes().to_vec())
            .unwrap();
    }
    let bytes = workspace.output_bytes(plan.job_id).unwrap();
    assert!(bytes.starts_with(b"PK\x03\x04"));
    let parts = parts(&bytes);
    assert_eq!(parts.len(), 3);
    let content = std::str::from_utf8(&parts["[Content_Types].xml"]).unwrap();
    assert!(content.contains("wordprocessingml.document.main+xml"));
    assert!(
        std::str::from_utf8(&parts["_rels/.rels"])
            .unwrap()
            .contains("Target=\"word/document.xml\"")
    );
    let xml = std::str::from_utf8(&parts["word/document.xml"]).unwrap();
    assert!(xml.contains("Ελληνικά Привет"));
    assert!(xml.contains("&lt;w:t&gt;&amp;&quot;&apos; café"));
    assert!(xml.contains("<w:tab/>"));
    assert_eq!(xml.matches("w:type=\"page\"").count(), 2);
    assert!(xml.find("Page 3").unwrap() < xml.find("Page 1").unwrap());
    assert!(xml.find("Page 1").unwrap() < xml.find("Page 2").unwrap());
    assert_eq!(workspace.current().unwrap().bytes(), before);
    assert_eq!(workspace.current().unwrap().revision(), 0);
    assert!(!workspace.current().unwrap().is_dirty());
}

#[test]
fn docx_requires_copy_authority_including_owner_access_and_protected_output_acknowledgment() {
    for version in [1, 2, 4, 5] {
        let encrypted = fixtures::encrypted(
            &fixtures::synthetic_pdf(1, false),
            version,
            Permissions::PRINTABLE,
        );
        let mut restricted = source(encrypted.clone(), Some("user"));
        assert!(
            restricted
                .begin_output(docx_request(&restricted, vec![0]))
                .is_err()
        );
        let mut owner = source(encrypted, Some("owner"));
        assert!(owner.current().unwrap().permissions().owner_authenticated);
        let mut request = docx_request(&owner, vec![0]);
        request.allow_unencrypted = false;
        assert!(
            owner
                .begin_output(request)
                .unwrap_err()
                .to_string()
                .contains("unencrypted")
        );
        let plan = owner.begin_output(docx_request(&owner, vec![0])).unwrap();
        let render = owner.output_render_bytes(plan.job_id).unwrap();
        assert!(!Document::load_mem(&render).unwrap().is_encrypted());
        owner
            .stage_output_page(plan.job_id, 0, b"Authorized copy".to_vec())
            .unwrap();
        assert!(owner.output_bytes(plan.job_id).unwrap().starts_with(b"PK"));
        assert!(
            Document::load_mem(owner.current().unwrap().bytes())
                .unwrap()
                .is_encrypted()
        );
    }
    let mut signed = source(fixtures::synthetic_pdf(1, true), None);
    assert!(
        signed
            .current()
            .unwrap()
            .info()
            .unwrap()
            .read_only_reason
            .is_some()
    );
    let before = signed.current().unwrap().bytes().to_vec();
    let plan = signed.begin_output(docx_request(&signed, vec![0])).unwrap();
    signed
        .stage_output_page(plan.job_id, 0, b"Signed text copy".to_vec())
        .unwrap();
    assert!(signed.output_bytes(plan.job_id).is_ok());
    assert_eq!(signed.current().unwrap().bytes(), before);
}

#[test]
fn docx_empty_invalid_xml_text_limits_stale_drafts_undo_and_no_clobber_are_guarded() {
    let mut workspace = source(fixtures::synthetic_pdf(2, false), None);
    let plan = workspace
        .begin_output(docx_request(&workspace, vec![0]))
        .unwrap();
    assert!(workspace.output_bytes(plan.job_id).is_err());
    assert!(
        workspace
            .stage_output_page(plan.job_id, 0, vec![b'a'; MAX_DOCX_PAGE_TEXT_BYTES + 1])
            .is_err()
    );
    assert!(
        workspace
            .stage_output_page(plan.job_id, 0, b"bad\0text".to_vec())
            .is_err()
    );
    assert!(
        workspace
            .stage_output_page(plan.job_id, 0, vec![0xff])
            .is_err()
    );
    workspace
        .stage_output_page(plan.job_id, 0, b" \n\t".to_vec())
        .unwrap();
    assert!(
        workspace
            .output_bytes(plan.job_id)
            .unwrap_err()
            .to_string()
            .contains("No editable text")
    );
    workspace.cancel_output(plan.job_id);
    let plan = workspace
        .begin_output(docx_request(&workspace, vec![0]))
        .unwrap();
    workspace
        .stage_output_page(plan.job_id, 0, b"Fresh".to_vec())
        .unwrap();
    let before = workspace.current().unwrap().info().unwrap();
    workspace.set_draft(before.id, true).unwrap();
    workspace.set_draft(before.id, false).unwrap();
    assert!(workspace.output_bytes(plan.job_id).is_err());
    let plan = workspace
        .begin_output(docx_request(&workspace, vec![0]))
        .unwrap();
    workspace
        .stage_output_page(plan.job_id, 0, b"Fresh".to_vec())
        .unwrap();
    let changed = workspace
        .current_mut(before.id, before.revision)
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    workspace
        .current_mut(before.id, changed.revision)
        .unwrap()
        .undo()
        .unwrap();
    assert_eq!(workspace.current().unwrap().revision(), before.revision);
    assert!(workspace.output_bytes(plan.job_id).is_err());
    let plan = workspace
        .begin_output(docx_request(&workspace, vec![0]))
        .unwrap();
    workspace
        .stage_output_page(plan.job_id, 0, b"Save copy".to_vec())
        .unwrap();
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("Text.docx");
    workspace.write_output_copy(plan.job_id, &path).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(workspace.write_output_copy(plan.job_id, &path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(
        workspace
            .write_output_copy(plan.job_id, &folder.path().join("Wrong.pdf"))
            .is_err()
    );
    assert!(
        workspace
            .close_document(before.id, before.revision, false)
            .is_err()
    );
    workspace
        .close_document(before.id, before.revision, true)
        .unwrap();
    assert!(workspace.output_bytes(plan.job_id).is_err());
}

#[test]
fn exact_creation_file_and_text_batch_limits_are_inclusive_and_never_truncate() {
    let mut workspace = Workspace::default();
    let plan = workspace.begin_creation(CreationKind::Text).unwrap();
    for index in 0..50 {
        workspace
            .stage_creation_file(plan.job_id, format!("File-{index}.txt"), b"x".to_vec())
            .unwrap();
    }
    assert_eq!(
        workspace.creation_plan(plan.job_id).unwrap().inputs.len(),
        50
    );
    assert!(
        workspace
            .stage_creation_file(plan.job_id, "Extra.txt".into(), b"x".to_vec())
            .is_err()
    );
    assert!(workspace.creation_plan(plan.job_id).is_err());
    let plan = workspace.begin_creation(CreationKind::Text).unwrap();
    for index in 0..4 {
        workspace
            .stage_creation_file(
                plan.job_id,
                format!("File-{index}.txt"),
                vec![b'x'; MAX_TEXT_FILE_BYTES],
            )
            .unwrap();
    }
    assert_eq!(
        workspace.creation_plan(plan.job_id).unwrap().inputs.len(),
        4
    );
    assert!(
        workspace
            .stage_creation_file(plan.job_id, "Extra.txt".into(), b"x".to_vec())
            .is_err()
    );
    assert!(!workspace.has_unsaved_changes());
}

#[test]
fn exact_docx_page_and_utf8_text_budgets_are_inclusive_and_do_not_accept_extra_bytes() {
    let mut workspace = source(fixtures::synthetic_pdf(201, false), None);
    let request = docx_request(&workspace, (0..200).collect());
    let plan = workspace.begin_output(request).unwrap();
    workspace.cancel_output(plan.job_id);
    assert!(
        workspace
            .begin_output(docx_request(&workspace, (0..201).collect()))
            .is_err()
    );
    for pages in [vec![0, 0], vec![201], vec![]] {
        assert!(
            workspace
                .begin_output(docx_request(&workspace, pages))
                .is_err()
        );
    }
    let page = vec![b'x'; MAX_DOCX_PAGE_TEXT_BYTES];
    let plan = workspace
        .begin_output(docx_request(&workspace, (0..32).collect()))
        .unwrap();
    for index in 0..32 {
        workspace
            .stage_output_page(plan.job_id, index, page.clone())
            .unwrap();
    }
    let docx = workspace.output_bytes(plan.job_id).unwrap();
    assert!(docx.len() > folio_core::conversion::MAX_DOCX_TEXT_BYTES);
    assert_eq!(parts(&docx).len(), 3);
    workspace.cancel_output(plan.job_id);
    let plan = workspace
        .begin_output(docx_request(&workspace, (0..33).collect()))
        .unwrap();
    for index in 0..32 {
        workspace
            .stage_output_page(plan.job_id, index, page.clone())
            .unwrap();
    }
    assert!(
        workspace
            .stage_output_page(plan.job_id, 32, b"x".to_vec())
            .is_err()
    );
    assert!(workspace.output_bytes(plan.job_id).is_err());
}

#[test]
fn source_image_staging_redaction_and_explicit_cancel_permanently_invalidate_docx_jobs() {
    let mut workspace = source(fixtures::synthetic_pdf(1, false), None);
    let info = workspace.current().unwrap().info().unwrap();
    let plan = workspace
        .begin_output(docx_request(&workspace, vec![0]))
        .unwrap();
    workspace
        .stage_output_page(plan.job_id, 0, b"Text".to_vec())
        .unwrap();
    let image = workspace
        .stage_authoring_image(info.id, info.revision, &png())
        .unwrap();
    assert!(workspace.output_bytes(plan.job_id).is_err());
    assert!(
        workspace
            .begin_output(docx_request(&workspace, vec![0]))
            .is_err()
    );
    workspace
        .cancel_authoring_image(info.id, image.asset_id)
        .unwrap();
    assert!(workspace.output_bytes(plan.job_id).is_err());
    let plan = workspace
        .begin_output(docx_request(&workspace, vec![0]))
        .unwrap();
    workspace
        .stage_output_page(plan.job_id, 0, b"Text".to_vec())
        .unwrap();
    let redaction = workspace
        .begin_redaction(
            info.id,
            info.revision,
            150,
            &[folio_core::redaction::Region {
                page: 0,
                rect: [40.0, 40.0, 80.0, 80.0],
            }],
        )
        .unwrap();
    assert!(workspace.output_bytes(plan.job_id).is_err());
    assert!(
        workspace
            .begin_output(docx_request(&workspace, vec![0]))
            .is_err()
    );
    workspace.cancel_redaction(redaction.job_id);
    assert!(workspace.output_bytes(plan.job_id).is_err());
    let plan = workspace
        .begin_output(docx_request(&workspace, vec![0]))
        .unwrap();
    assert!(workspace.cancel_output(plan.job_id));
    assert!(workspace.output_render_bytes(plan.job_id).is_err());
    assert!(!workspace.has_unsaved_changes());
}

#[test]
fn docx_native_post_dialog_checks_all_newly_opened_source_and_saved_paths_even_when_missing() {
    let directory = tempfile::tempdir().unwrap();
    let mut workspace = source(fixtures::synthetic_pdf(1, false), None);
    let plan = workspace
        .begin_output(docx_request(&workspace, vec![0]))
        .unwrap();
    workspace
        .stage_output_page(plan.job_id, 0, b"Copied text".to_vec())
        .unwrap();
    let source_path = directory.path().join("Opened.docx");
    std::fs::write(&source_path, fixtures::synthetic_pdf(1, false)).unwrap();
    let other = workspace.open_path(source_path.clone()).unwrap();
    let saved_path = directory.path().join("Saved.docx");
    workspace
        .current_mut(other.id, other.revision)
        .unwrap()
        .save_as(&saved_path)
        .unwrap();
    std::fs::remove_file(&source_path).unwrap();
    std::fs::remove_file(&saved_path).unwrap();
    assert!(
        workspace
            .write_output_copy(plan.job_id, &source_path)
            .is_err()
    );
    assert!(
        workspace
            .write_output_copy(plan.job_id, &saved_path)
            .is_err()
    );
    assert!(!source_path.exists() && !saved_path.exists());
    let output = directory.path().join("Separate.docx");
    workspace.write_output_copy(plan.job_id, &output).unwrap();
    assert!(std::fs::read(&output).unwrap().starts_with(b"PK"));
    assert!(
        workspace
            .document(plan.document_id)
            .unwrap()
            .saved_path
            .is_none()
    );
    assert_eq!(
        workspace.document(plan.document_id).unwrap().revision(),
        plan.revision
    );
}
