#[path = "../examples/output_fixtures.rs"]
mod fixtures;

use folio_core::{
    document::Edit,
    output::{
        MAX_PAGE_PIXELS, MAX_PRINT_PIXELS, MAX_TEXT_BYTES, OutputKind, OutputPlan, OutputRequest,
        output_path, safe_stem,
    },
    workspace::{OpenOutcome, Workspace},
};
use lopdf::{Document, LoadOptions, Object, encryption::Permissions};

fn workspace(bytes: Vec<u8>, password: Option<&str>) -> Workspace {
    let mut workspace = Workspace::default();
    let outcome = workspace
        .request_open_bytes(bytes, "Synthetic.pdf".into())
        .unwrap();
    if let OpenOutcome::PasswordRequired { request_id, .. } = outcome {
        assert!(matches!(
            workspace
                .unlock_document(request_id, password.unwrap())
                .unwrap(),
            OpenOutcome::Opened { .. }
        ));
    }
    workspace
}

fn request(workspace: &Workspace, kind: OutputKind) -> OutputRequest {
    let info = workspace.current().unwrap().info().unwrap();
    OutputRequest {
        document_id: info.id,
        revision: info.revision,
        kind,
        pages: if kind == OutputKind::Optimize {
            vec![]
        } else {
            vec![0]
        },
        dpi: if matches!(kind, OutputKind::Print | OutputKind::Png | OutputKind::Jpeg) {
            Some(150)
        } else {
            None
        },
        quality: if kind == OutputKind::Jpeg {
            Some(90)
        } else {
            None
        },
        allow_unencrypted: true,
    }
}

fn begin(workspace: &mut Workspace, kind: OutputKind) -> OutputPlan {
    workspace.begin_output(request(workspace, kind)).unwrap()
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::One);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&vec![0; width.div_ceil(8) as usize * height as usize])
            .unwrap();
    }
    bytes
}

fn optimized(workspace: &mut Workspace) -> (Vec<u8>, folio_core::output::OptimizationReport) {
    let plan = begin(workspace, OutputKind::Optimize);
    let report = workspace.prepare_optimized_output(plan.job_id).unwrap();
    let bytes = workspace.output_bytes(plan.job_id).unwrap();
    workspace.cancel_output(plan.job_id);
    (bytes, report)
}

#[test]
fn optimization_really_reduces_bytes_and_preserves_reachable_content_geometry_and_metadata() {
    let source = fixtures::synthetic_pdf(24, false);
    let original = Document::load_mem(&source).unwrap();
    let mut workspace = workspace(source.clone(), None);
    let original_info = workspace.current().unwrap().info().unwrap();
    let (bytes, report) = optimized(&mut workspace);
    assert!(report.reduced);
    assert!(report.after_bytes < report.before_bytes / 8, "{report:?}");
    assert!(report.compressed_streams >= 24);
    assert!(report.removed_objects >= 1);
    assert_eq!(report.before_bytes, source.len());
    assert_eq!(report.after_bytes, bytes.len());
    let copy = Document::load_mem(&bytes).unwrap();
    assert_eq!(copy.get_pages(), original.get_pages());
    for id in copy.get_pages().values() {
        assert_eq!(copy.get_page_content(*id), original.get_page_content(*id));
        assert_eq!(
            copy.get_dictionary(*id).unwrap(),
            original.get_dictionary(*id).unwrap()
        );
    }
    for key in [b"Metadata".as_slice(), b"Names", b"Outlines"] {
        assert_eq!(
            copy.catalog().unwrap().get(key).unwrap(),
            original.catalog().unwrap().get(key).unwrap()
        );
    }
    let metadata = copy
        .catalog()
        .unwrap()
        .get(b"Metadata")
        .unwrap()
        .as_reference()
        .unwrap();
    assert_eq!(
        copy.get_object(metadata).unwrap(),
        original.get_object(metadata).unwrap()
    );
    assert_eq!(
        copy.trailer.get(b"Info").unwrap(),
        original.trailer.get(b"Info").unwrap()
    );
    assert_eq!(
        copy.trailer.get(b"ID").unwrap(),
        original.trailer.get(b"ID").unwrap()
    );
    let text = copy.extract_text(&[24]).unwrap();
    assert!(text.contains("SYNTHETIC PAGE 024"), "{text:?}");
    assert_eq!(workspace.current().unwrap().bytes(), source);
    let info = workspace.current().unwrap().info().unwrap();
    assert_eq!(info.revision, original_info.revision);
    assert!(!info.dirty && !info.can_undo && !info.can_redo);
}

#[test]
fn no_reduction_offers_an_exact_unchanged_copy_not_a_larger_rewrite() {
    let mut first = workspace(fixtures::synthetic_pdf(1, false), None);
    let (small, _) = optimized(&mut first);
    let mut workspace = workspace(small.clone(), None);
    let (copy, report) = optimized(&mut workspace);
    assert!(!report.reduced, "{report:?}");
    assert!(report.candidate_bytes >= small.len());
    assert_eq!(report.after_bytes, report.before_bytes);
    assert_eq!(report.compressed_streams, 0);
    assert_eq!(report.removed_objects, 0);
    assert_eq!(copy, small);
}

#[test]
fn optimization_uses_unsaved_work_without_clearing_dirty_state_saved_revision_or_undo() {
    let original = fixtures::synthetic_pdf(3, false);
    let mut workspace = workspace(original.clone(), None);
    let initial = workspace.current().unwrap().info().unwrap();
    let changed = workspace
        .current_mut(initial.id, initial.revision)
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    let working = workspace.current().unwrap().bytes().to_vec();
    let (copy, _) = optimized(&mut workspace);
    let reopened =
        folio_core::document::Session::from_bytes(copy, "Copy.pdf".into(), None, 100).unwrap();
    assert_eq!(reopened.info().unwrap().pages[0].rotation, 90);
    let after = workspace.current().unwrap().info().unwrap();
    assert_eq!(after.revision, changed.revision);
    assert!(after.dirty && after.can_undo && !after.can_redo);
    assert_eq!(workspace.current().unwrap().bytes(), working);
    workspace
        .current_mut(changed.id, changed.revision)
        .unwrap()
        .undo()
        .unwrap();
    assert_eq!(workspace.current().unwrap().bytes(), original);
    assert!(!workspace.current().unwrap().is_dirty());
}

#[test]
fn optimized_copies_keep_rc4_aes_passwords_keys_permissions_and_never_save_render_bytes() {
    for version in [1, 2, 4, 5] {
        let original = fixtures::encrypted(
            &fixtures::synthetic_pdf(3, false),
            version,
            Permissions::all(),
        );
        let mut workspace = workspace(original.clone(), Some("user"));
        let plan = begin(&mut workspace, OutputKind::Optimize);
        assert!(workspace.output_render_bytes(plan.job_id).is_err());
        let report = workspace.prepare_optimized_output(plan.job_id).unwrap();
        assert!(report.reduced);
        let copy = workspace.output_bytes(plan.job_id).unwrap();
        assert!(Document::load_mem(&copy).unwrap().is_encrypted());
        for password in ["user", "owner"] {
            let output =
                Document::load_mem_with_options(&copy, LoadOptions::with_password(password))
                    .unwrap();
            let before =
                Document::load_mem_with_options(&original, LoadOptions::with_password(password))
                    .unwrap();
            let state = output.encryption_state.as_ref().unwrap();
            let old_state = before.encryption_state.as_ref().unwrap();
            assert_eq!(state.version(), old_state.version());
            assert_eq!(state.permissions(), old_state.permissions());
            assert_eq!(state.file_encryption_key(), old_state.file_encryption_key());
            assert_eq!(
                output.trailer.get(b"ID").unwrap(),
                before.trailer.get(b"ID").unwrap()
            );
            let text = output.extract_text(&[1]).unwrap();
            assert!(text.contains("SYNTHETIC PAGE 001"), "{text:?}");
        }
        assert_eq!(workspace.current().unwrap().bytes(), original);
    }
}

#[test]
fn print_copy_and_edit_authorities_are_independent_and_owner_access_is_honored() {
    let bytes = fixtures::encrypted(
        &fixtures::synthetic_pdf(1, false),
        4,
        Permissions::PRINTABLE,
    );
    let mut restricted = workspace(bytes.clone(), Some("user"));
    let info = restricted.current().unwrap().info().unwrap();
    assert!(info.read_only_reason.is_some());
    assert!(info.permissions.print_allowed);
    assert!(!info.permissions.high_quality_print_allowed);
    assert!(!info.copy_allowed);
    let print = begin(&mut restricted, OutputKind::Print);
    restricted.cancel_output(print.job_id);
    let mut high = request(&restricted, OutputKind::Print);
    high.dpi = Some(300);
    assert!(restricted.begin_output(high).is_err());
    for kind in [
        OutputKind::Png,
        OutputKind::Jpeg,
        OutputKind::Text,
        OutputKind::Optimize,
    ] {
        assert!(restricted.begin_output(request(&restricted, kind)).is_err());
    }
    assert!(
        restricted
            .current_mut(info.id, info.revision)
            .unwrap()
            .apply(Edit::Rotate {
                page: 0,
                degrees: 90
            })
            .is_err()
    );
    let mut owner = workspace(bytes, Some("owner"));
    let owner_info = owner.current().unwrap().info().unwrap();
    assert!(
        owner_info.permissions.owner_authenticated
            && owner_info.permissions.high_quality_print_allowed
    );
    let mut high = request(&owner, OutputKind::Print);
    high.dpi = Some(300);
    let plan = owner.begin_output(high).unwrap();
    owner.cancel_output(plan.job_id);
    let plan = begin(&mut owner, OutputKind::Text);
    owner.cancel_output(plan.job_id);

    let mut copy_only = workspace(
        fixtures::encrypted(&fixtures::synthetic_pdf(1, false), 5, Permissions::COPYABLE),
        Some("user"),
    );
    assert!(
        !copy_only
            .current()
            .unwrap()
            .info()
            .unwrap()
            .permissions
            .print_allowed
    );
    assert!(
        copy_only
            .begin_output(request(&copy_only, OutputKind::Print))
            .is_err()
    );
    let plan = begin(&mut copy_only, OutputKind::Text);
    copy_only.cancel_output(plan.job_id);
    assert!(
        copy_only
            .begin_output(request(&copy_only, OutputKind::Optimize))
            .is_err()
    );
    let mut rc4 = workspace(
        fixtures::encrypted(
            &fixtures::synthetic_pdf(1, false),
            1,
            Permissions::PRINTABLE,
        ),
        Some("user"),
    );
    let mut high = request(&rc4, OutputKind::Print);
    high.dpi = Some(300);
    assert!(rc4.begin_output(high).is_ok());
}

#[test]
fn signed_sources_allow_permitted_read_only_output_but_never_optimization_even_with_owner_access() {
    for password in ["user", "owner"] {
        let bytes = fixtures::encrypted(&fixtures::synthetic_pdf(3, true), 4, Permissions::all());
        let mut workspace = workspace(bytes.clone(), Some(password));
        assert!(
            workspace
                .current()
                .unwrap()
                .info()
                .unwrap()
                .read_only_reason
                .unwrap()
                .contains("signed")
        );
        assert!(
            workspace
                .begin_output(request(&workspace, OutputKind::Optimize))
                .is_err()
        );
        for kind in [
            OutputKind::Print,
            OutputKind::Png,
            OutputKind::Jpeg,
            OutputKind::Text,
        ] {
            let plan = begin(&mut workspace, kind);
            assert!(workspace.output_render_bytes(plan.job_id).is_ok());
            workspace.cancel_output(plan.job_id);
        }
        assert_eq!(workspace.current().unwrap().bytes(), bytes);
    }
}

#[test]
fn protected_content_exports_require_explicit_unencrypted_acknowledgment() {
    let mut workspace = workspace(
        fixtures::encrypted(&fixtures::synthetic_pdf(1, false), 4, Permissions::all()),
        Some("user"),
    );
    for kind in [OutputKind::Png, OutputKind::Jpeg, OutputKind::Text] {
        let mut input = request(&workspace, kind);
        input.allow_unencrypted = false;
        assert!(
            workspace
                .begin_output(input)
                .unwrap_err()
                .to_string()
                .contains("unencrypted")
        );
        let plan = begin(&mut workspace, kind);
        let rendered = workspace.output_render_bytes(plan.job_id).unwrap();
        assert!(!Document::load_mem(&rendered).unwrap().is_encrypted());
        workspace.cancel_output(plan.job_id);
    }
}

#[test]
fn staged_text_is_exact_utf8_in_selected_order_with_only_form_feed_page_separators() {
    let mut workspace = workspace(fixtures::synthetic_pdf(24, false), None);
    let mut input = request(&workspace, OutputKind::Text);
    input.pages = vec![23, 1, 0];
    let plan = workspace.begin_output(input).unwrap();
    for (index, text) in ["Offscreen page 24\n", "Καλημέρα\n", "First page"]
        .into_iter()
        .enumerate()
    {
        workspace
            .stage_output_page(plan.job_id, index, text.as_bytes().to_vec())
            .unwrap();
    }
    assert_eq!(
        workspace.output_bytes(plan.job_id).unwrap(),
        "Offscreen page 24\n\n\u{000c}\nΚαλημέρα\n\n\u{000c}\nFirst page".as_bytes()
    );
    workspace.cancel_output(plan.job_id);
    let empty = begin(&mut workspace, OutputKind::Text);
    workspace
        .stage_output_page(empty.job_id, 0, b"\n ".to_vec())
        .unwrap();
    assert!(
        workspace
            .check_output_ready(empty.job_id)
            .unwrap_err()
            .to_string()
            .contains("OCR")
    );
}

fn zip_entries(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut index = 0;
    let mut entries = Vec::new();
    while bytes[index..].starts_with(&0x04034b50_u32.to_le_bytes()) {
        let size = u32::from_le_bytes(bytes[index + 18..index + 22].try_into().unwrap()) as usize;
        let length = u16::from_le_bytes(bytes[index + 26..index + 28].try_into().unwrap()) as usize;
        let crc = u32::from_le_bytes(bytes[index + 14..index + 18].try_into().unwrap());
        let name = String::from_utf8(bytes[index + 30..index + 30 + length].to_vec()).unwrap();
        let content = bytes[index + 30 + length..index + 30 + length + size].to_vec();
        assert_eq!(crc, crc32fast::hash(&content));
        entries.push((name, content));
        index += 30 + length + size;
    }
    assert!(bytes[index..].starts_with(&0x02014b50_u32.to_le_bytes()));
    assert!(bytes[bytes.len() - 22..].starts_with(&0x06054b50_u32.to_le_bytes()));
    assert_eq!(
        u16::from_le_bytes(
            bytes[bytes.len() - 12..bytes.len() - 10]
                .try_into()
                .unwrap()
        ) as usize,
        entries.len()
    );
    entries
}

#[test]
fn single_png_is_exact_and_multiple_images_form_one_crc_valid_ordered_zip_with_safe_entry_names() {
    let mut workspace = workspace(fixtures::synthetic_pdf(24, false), None);
    let page = png(13, 17);
    let single = begin(&mut workspace, OutputKind::Png);
    workspace
        .stage_output_page(single.job_id, 0, page.clone())
        .unwrap();
    assert_eq!(workspace.output_bytes(single.job_id).unwrap(), page);
    workspace.cancel_output(single.job_id);
    let mut input = request(&workspace, OutputKind::Png);
    input.pages = vec![23, 0];
    let plan = workspace.begin_output(input).unwrap();
    workspace
        .stage_output_page(plan.job_id, 0, page.clone())
        .unwrap();
    workspace
        .stage_output_page(plan.job_id, 1, png(21, 19))
        .unwrap();
    assert_eq!(plan.suggested_name, "Synthetic-pages.zip");
    let zip = zip_entries(&workspace.output_bytes(plan.job_id).unwrap());
    assert_eq!(zip[0], ("page-0024.png".into(), page));
    assert_eq!(zip[1], ("page-0001.png".into(), png(21, 19)));
}

#[test]
fn outputs_are_incomplete_until_every_page_and_drafts_stale_revisions_cancel_and_close_are_guarded()
{
    let mut workspace = workspace(fixtures::synthetic_pdf(3, false), None);
    let first = workspace.current().unwrap().info().unwrap();
    workspace.set_draft(first.id, true).unwrap();
    assert!(
        workspace
            .begin_output(request(&workspace, OutputKind::Text))
            .is_err()
    );
    workspace.set_draft(first.id, false).unwrap();
    let plan = begin(&mut workspace, OutputKind::Text);
    assert!(workspace.has_unsaved_changes());
    assert!(
        workspace
            .close_document(first.id, first.revision, false)
            .is_err()
    );
    assert!(workspace.output_bytes(plan.job_id).is_err());
    assert!(
        workspace
            .stage_output_page(plan.job_id, 1, b"wrong order".to_vec())
            .is_err()
    );
    workspace.set_draft(first.id, true).unwrap();
    assert!(
        workspace
            .stage_output_page(plan.job_id, 0, b"unapplied".to_vec())
            .is_err()
    );
    workspace.set_draft(first.id, false).unwrap();
    workspace
        .stage_output_page(plan.job_id, 0, b"Selected text".to_vec())
        .unwrap();
    assert!(
        workspace
            .stage_output_page(plan.job_id, 0, b"repeated".to_vec())
            .is_err()
    );
    let second = workspace.open_demo().unwrap();
    assert!(workspace.check_output_ready(plan.job_id).is_ok());
    workspace
        .current_mut(first.id, first.revision)
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    assert!(workspace.output_bytes(plan.job_id).is_err());
    assert!(workspace.cancel_output(plan.job_id));
    assert!(!workspace.cancel_output(plan.job_id));
    assert!(workspace.output_plan(plan.job_id).is_err());
    workspace.activate(second.id).unwrap();
    let next = begin(&mut workspace, OutputKind::Print);
    assert!(next.job_id > plan.job_id);
    assert!(
        workspace
            .begin_output(request(&workspace, OutputKind::Text))
            .is_err()
    );
    workspace
        .close_document(second.id, second.revision, true)
        .unwrap();
    assert!(workspace.check_output(next.job_id).is_err());
    assert!(workspace.document(first.id).unwrap().is_dirty());
}

#[test]
fn native_copy_writes_are_no_clobber_and_recheck_stale_draft_and_all_tab_paths() {
    let directory = tempfile::tempdir().unwrap();
    let original = directory.path().join("original.pdf");
    std::fs::write(&original, fixtures::synthetic_pdf(1, false)).unwrap();
    let mut workspace = Workspace::default();
    let first = workspace.open_path(original.clone()).unwrap();
    let saved = directory.path().join("saved.pdf");
    workspace
        .current_mut(first.id, first.revision)
        .unwrap()
        .save_as(&saved)
        .unwrap();
    let other = workspace.open_demo().unwrap();
    let plan = begin(&mut workspace, OutputKind::Optimize);
    workspace.prepare_optimized_output(plan.job_id).unwrap();
    assert!(workspace.write_output_copy(plan.job_id, &original).is_err());
    assert!(workspace.write_output_copy(plan.job_id, &saved).is_err());
    let folder = directory.path().join("folder.pdf");
    std::fs::create_dir(&folder).unwrap();
    assert!(workspace.write_output_copy(plan.job_id, &folder).is_err());
    #[cfg(unix)]
    {
        let alias = directory.path().join("alias.pdf");
        std::os::unix::fs::symlink(&original, &alias).unwrap();
        assert!(workspace.write_output_copy(plan.job_id, &alias).is_err());
    }
    let target = directory.path().join("output.pdf");
    workspace.set_draft(other.id, true).unwrap();
    assert!(workspace.write_output_copy(plan.job_id, &target).is_err());
    assert!(!target.exists());
    workspace.set_draft(other.id, false).unwrap();
    let count = workspace.write_output_copy(plan.job_id, &target).unwrap();
    assert_eq!(std::fs::read(&target).unwrap().len(), count);
    let before = std::fs::read(&target).unwrap();
    assert!(workspace.write_output_copy(plan.job_id, &target).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), before);
    workspace
        .current_mut(other.id, other.revision)
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    assert!(
        workspace
            .write_output_copy(plan.job_id, &directory.path().join("stale.pdf"))
            .is_err()
    );
    assert!(workspace.document(first.id).is_ok());
    assert_eq!(
        std::fs::read(&original).unwrap(),
        std::fs::read(&saved).unwrap()
    );
}

#[test]
fn exact_text_page_pixel_and_cumulative_print_limits_are_enforced_without_partial_output() {
    let mut workspace = workspace(fixtures::synthetic_pdf(6, false), None);
    let text = begin(&mut workspace, OutputKind::Text);
    workspace
        .stage_output_page(text.job_id, 0, vec![b'a'; MAX_TEXT_BYTES])
        .unwrap();
    assert_eq!(
        workspace.output_bytes(text.job_id).unwrap().len(),
        MAX_TEXT_BYTES
    );
    workspace.cancel_output(text.job_id);
    let text = begin(&mut workspace, OutputKind::Text);
    assert!(
        workspace
            .stage_output_page(text.job_id, 0, vec![b'a'; MAX_TEXT_BYTES + 1])
            .is_err()
    );
    assert!(workspace.output_bytes(text.job_id).is_err());
    workspace.cancel_output(text.job_id);
    let image = begin(&mut workspace, OutputKind::Png);
    assert_eq!(6000 * 4000, MAX_PAGE_PIXELS);
    workspace
        .stage_output_page(image.job_id, 0, png(6000, 4000))
        .unwrap();
    workspace.cancel_output(image.job_id);
    let image = begin(&mut workspace, OutputKind::Png);
    assert!(
        workspace
            .stage_output_page(image.job_id, 0, png(6001, 4000))
            .is_err()
    );
    assert!(
        workspace
            .stage_output_page(image.job_id, 0, png(16385, 1))
            .is_err()
    );
    workspace.cancel_output(image.job_id);
    let mut input = request(&workspace, OutputKind::Print);
    input.pages = vec![0, 1, 2, 3, 4, 5];
    let print = workspace.begin_output(input).unwrap();
    assert_eq!(5 * 5000 * 4000, MAX_PRINT_PIXELS);
    let image = png(5000, 4000);
    for index in 0..5 {
        workspace
            .stage_output_page(print.job_id, index, image.clone())
            .unwrap();
    }
    assert!(
        workspace
            .stage_output_page(print.job_id, 5, png(1, 1))
            .is_err()
    );
    assert!(workspace.check_output_ready(print.job_id).is_err());
}

#[test]
fn page_range_settings_filename_and_format_inputs_are_bounded_and_untrusted() {
    assert_eq!(safe_stem("../../dangerous\\CON.pdf"), "Folio");
    assert_eq!(safe_stem("a:<bad>?.PDF"), "a--bad");
    assert_eq!(safe_stem("..\\folder/private.pdf"), "private");
    assert!(safe_stem(&"a".repeat(500)).len() <= 80);
    let mut workspace = workspace(fixtures::synthetic_pdf(3, false), None);
    let mut invalids = Vec::new();
    for pages in [vec![], vec![0, 0], vec![3], vec![0; 501]] {
        let mut input = request(&workspace, OutputKind::Png);
        input.pages = pages;
        invalids.push(input);
    }
    let mut input = request(&workspace, OutputKind::Print);
    input.dpi = Some(600);
    invalids.push(input);
    let mut input = request(&workspace, OutputKind::Jpeg);
    input.quality = Some(59);
    invalids.push(input);
    let mut input = request(&workspace, OutputKind::Text);
    input.dpi = Some(150);
    invalids.push(input);
    for input in invalids {
        assert!(workspace.begin_output(input).is_err());
    }
    let plan = begin(&mut workspace, OutputKind::Png);
    assert_eq!(
        output_path("out".into(), &plan).unwrap(),
        std::path::PathBuf::from("out.png")
    );
    assert!(output_path("out.pdf".into(), &plan).is_err());
    assert!(
        workspace
            .stage_output_page(plan.job_id, 0, b"not a PNG".to_vec())
            .is_err()
    );
    assert!(workspace.output_bytes(plan.job_id).is_err());
    let info = workspace.current().unwrap().info().unwrap();
    assert!(!info.dirty && !info.can_undo);
}

#[test]
fn malformed_reference_optimization_fails_explicitly_without_mutating_the_source() {
    let mut document = Document::load_mem(&fixtures::synthetic_pdf(1, false)).unwrap();
    document
        .catalog_mut()
        .unwrap()
        .set("BrokenReference", Object::Reference((999_999, 0)));
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    let mut workspace = workspace(bytes.clone(), None);
    let plan = begin(&mut workspace, OutputKind::Optimize);
    assert!(
        workspace
            .prepare_optimized_output(plan.job_id)
            .unwrap_err()
            .to_string()
            .contains("unresolved")
    );
    assert!(workspace.output_bytes(plan.job_id).is_err());
    assert_eq!(workspace.current().unwrap().bytes(), bytes);
    assert!(!workspace.current().unwrap().info().unwrap().dirty);
}
