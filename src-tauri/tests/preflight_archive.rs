#[path = "../examples/output_fixtures.rs"]
mod fixtures;

use folio_core::{
    archive::{self, Request, SourceRef},
    document::{DocumentInfo, Edit, Session},
    preflight::State,
    protection::CopyProtection,
    workspace::{OpenOutcome, Workspace},
};
use lopdf::{Document, Object, Stream, dictionary, encryption::Permissions};

fn open(workspace: &mut Workspace) -> DocumentInfo {
    workspace
        .open_bytes(
            Session::blank(72.0, 96.0, 1).unwrap().bytes().to_vec(),
            "Original.pdf".into(),
        )
        .unwrap()
}

fn request(info: &DocumentInfo) -> Request {
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

fn png(width: u32, height: u32, rgba: bool, alpha: u8) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(if rgba {
            png::ColorType::Rgba
        } else {
            png::ColorType::Rgb
        });
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        let pixel = if rgba {
            vec![20, 80, 160, alpha]
        } else {
            vec![20, 80, 160]
        };
        writer
            .write_image_data(&pixel.repeat(width as usize * height as usize))
            .unwrap();
        writer.finish().unwrap();
    }
    bytes
}

fn complete(workspace: &mut Workspace, plan: &archive::Plan) -> Vec<u8> {
    for (index, page) in plan.pages.iter().enumerate() {
        workspace
            .stage_archive_page(
                plan.archive_id,
                index,
                &png(page.pixel_width, page.pixel_height, false, 255),
            )
            .unwrap();
    }
    workspace.archive_bytes(plan.archive_id).unwrap()
}

#[test]
fn native_and_preview_request_shape_preserves_source_identity_and_rejects_unknown_fields() {
    let input = serde_json::json!({
        "documentId": 3, "revision": 7, "pages": [0, 2], "dpi": 300,
        "acknowledgeLoss": true, "allowUnencrypted": true, "acknowledgeUnsigned": true
    });
    let request: Request = serde_json::from_value(input.clone()).unwrap();
    assert_eq!(request.source.document_id, 3);
    assert_eq!(request.source.revision, 7);
    let mut wrong = input;
    wrong["unexpected"] = true.into();
    assert!(serde_json::from_value::<Request>(wrong).is_err());
}

#[test]
fn fresh_archive_has_only_new_raster_objects_and_does_not_touch_source_history_paths_or_tabs() {
    let mut workspace = Workspace::default();
    let original = workspace
        .open_bytes(fixtures::synthetic_pdf(3, false), "Source.pdf".into())
        .unwrap();
    let other = open(&mut workspace);
    let before = workspace.document(original.id).unwrap().bytes().to_vec();
    let state =
        serde_json::to_value(workspace.document(original.id).unwrap().info().unwrap()).unwrap();
    let mut input = request(&original);
    input.pages = vec![0, 1, 2];
    let plan = workspace.begin_archive(input).unwrap();
    assert!(workspace.has_unsaved_changes());
    assert!(
        workspace
            .close_document(original.id, original.revision, false)
            .is_err()
    );
    assert!(workspace.archive_bytes(plan.archive_id).is_err());
    let bytes = complete(&mut workspace, &plan);
    let pdf = Document::load_mem(&bytes).unwrap();
    assert_eq!(pdf.version, "1.7");
    assert!(!pdf.is_encrypted());
    let catalog = pdf.catalog().unwrap();
    for key in [
        b"AcroForm".as_slice(),
        b"StructTreeRoot",
        b"MarkInfo",
        b"Names",
        b"OpenAction",
        b"Outlines",
        b"Perms",
    ] {
        assert!(!catalog.has(key));
    }
    assert!(!pdf.trailer.has(b"Info"));
    assert!(catalog.has(b"Metadata") && catalog.has(b"OutputIntents"));
    assert_eq!(pdf.get_pages().len(), 3);
    for object in pdf.objects.values() {
        let dictionary = match object {
            Object::Dictionary(value) => Some(value),
            Object::Stream(value) => Some(&value.dict),
            _ => None,
        };
        if let Some(dictionary) = dictionary {
            assert!(!dictionary.has(b"Font") && !dictionary.has(b"FontDescriptor"));
            assert!(!dictionary.has(b"ByteRange") && !dictionary.has(b"Annots"));
        }
    }
    for (index, id) in pdf.get_pages().values().enumerate() {
        let reopened = Session::from_bytes(bytes.clone(), "Archive.pdf".into(), None, 99)
            .unwrap()
            .info()
            .unwrap();
        assert_eq!(reopened.pages[index].geometry.rotation, 0);
        assert_eq!(reopened.pages[index].geometry.user_unit, 1.0);
        assert_eq!(
            reopened.pages[index].geometry.width_points,
            plan.pages[index].geometry.width_points
        );
        assert_eq!(
            reopened.pages[index].geometry.height_points,
            plan.pages[index].geometry.height_points
        );
        assert!(!pdf.get_dictionary(*id).unwrap().has(b"Annots"));
    }
    assert_eq!(workspace.current().unwrap().id, other.id);
    assert_eq!(workspace.document(original.id).unwrap().bytes(), before);
    assert_eq!(
        serde_json::to_value(workspace.document(original.id).unwrap().info().unwrap()).unwrap(),
        state
    );
    assert!(
        workspace
            .document(original.id)
            .unwrap()
            .source_path
            .is_none()
    );
    assert!(
        workspace
            .document(original.id)
            .unwrap()
            .saved_path
            .is_none()
    );
    assert!(workspace.cancel_archive(plan.archive_id));
    assert!(!workspace.cancel_archive(plan.archive_id));
    assert!(!workspace.has_unsaved_changes());
}

#[test]
fn every_folio_rewrite_of_its_own_archive_is_read_only_but_exact_saves_and_raster_rearchives_are_allowed()
 {
    let mut workspace = Workspace::default();
    let original = open(&mut workspace);
    let plan = workspace.begin_archive(request(&original)).unwrap();
    let bytes = complete(&mut workspace, &plan);
    workspace.cancel_archive(plan.archive_id);
    let mut archived = Session::from_bytes(bytes.clone(), "Archive.pdf".into(), None, 50).unwrap();
    assert!(
        archived
            .info()
            .unwrap()
            .read_only_reason
            .unwrap()
            .contains("archival")
    );
    assert!(
        archived
            .apply(Edit::Rotate {
                page: 0,
                degrees: 90
            })
            .is_err()
    );
    assert!(archived.fill_form_fields(vec![]).is_err());
    assert!(archived.extract_pages(&[0]).is_err());
    assert!(
        archived
            .import_pages(workspace.document(original.id).unwrap().bytes().to_vec(), 0)
            .is_err()
    );
    assert!(archived.flattened_form_copy().is_err());
    assert!(
        archived
            .protection_copy(&CopyProtection::Unprotect, None)
            .is_err()
    );
    assert!(archived.signed_copy(&[], "not-a-key").is_err());
    assert!(archived.ensure_signable().is_err());
    let directory = tempfile::tempdir().unwrap();
    let exact = directory.path().join("exact.pdf");
    archived.save_as(&exact).unwrap();
    assert_eq!(std::fs::read(exact).unwrap(), bytes);
    let info = workspace.open_bytes(bytes, "Archive.pdf".into()).unwrap();
    let second = workspace.begin_archive(request(&info)).unwrap();
    workspace.cancel_archive(second.archive_id);
    let output: folio_core::output::OutputRequest = serde_json::from_value(serde_json::json!({
        "documentId":info.id,"revision":info.revision,"kind":"optimize","pages":[],"dpi":null,"quality":null,"allowUnencrypted":false
    })).unwrap();
    assert!(workspace.begin_output(output).is_err());
}

#[test]
fn edits_undo_drafts_staging_close_and_cancel_invalidate_jobs_without_resurrection() {
    for invalidator in ["edit-undo", "draft", "image", "close", "cancel"] {
        let mut workspace = Workspace::default();
        let info = open(&mut workspace);
        let report = workspace.inspect_technical(request(&info).source).unwrap();
        let plan = workspace.begin_archive(request(&info)).unwrap();
        match invalidator {
            "edit-undo" => {
                let changed = workspace
                    .current_mut(info.id, info.revision)
                    .unwrap()
                    .apply(Edit::Rotate {
                        page: 0,
                        degrees: 90,
                    })
                    .unwrap();
                workspace
                    .current_mut(info.id, changed.revision)
                    .unwrap()
                    .undo()
                    .unwrap();
                assert_eq!(
                    workspace.document(info.id).unwrap().revision(),
                    info.revision
                );
            }
            "draft" => {
                workspace.set_draft(info.id, true).unwrap();
                workspace.set_draft(info.id, false).unwrap();
            }
            "image" => {
                let asset = workspace
                    .stage_authoring_image(info.id, info.revision, &png(1, 1, false, 255))
                    .unwrap();
                assert!(
                    workspace
                        .ensure_technical_ready(info.id, info.revision)
                        .is_err()
                );
                workspace
                    .cancel_authoring_image(info.id, asset.asset_id)
                    .unwrap();
            }
            "close" => {
                workspace
                    .close_document(info.id, info.revision, true)
                    .unwrap();
            }
            "cancel" => {
                workspace.cancel_archive(plan.archive_id);
                workspace.cancel_inspection(report.inspection_id);
            }
            _ => unreachable!(),
        }
        assert!(
            workspace.check_archive(plan.archive_id).is_err(),
            "{invalidator}"
        );
        assert!(workspace.archive_render_bytes(plan.archive_id).is_err());
        assert!(workspace.check_inspection(report.inspection_id).is_err());
        if invalidator != "close" {
            let fresh = workspace.begin_archive(request(&info)).unwrap();
            if invalidator != "cancel" {
                assert!(fresh.epoch > plan.epoch);
            }
        }
    }
}

fn unlock(workspace: &mut Workspace, bytes: Vec<u8>, password: &str) -> DocumentInfo {
    let OpenOutcome::PasswordRequired { request_id, .. } = workspace
        .request_open_bytes(bytes, "Protected.pdf".into())
        .unwrap()
    else {
        panic!("Expected password");
    };
    let OpenOutcome::Opened { document, .. } =
        workspace.unlock_document(request_id, password).unwrap()
    else {
        panic!("Expected successful unlock");
    };
    document
}

#[test]
fn protection_requires_actual_owner_and_unencrypted_acknowledgment_not_permissive_user_access() {
    let source = fixtures::synthetic_pdf(1, false);
    for version in [1, 2, 4, 5] {
        let encrypted = fixtures::encrypted(&source, version, Permissions::all());
        let mut workspace = Workspace::default();
        let user = unlock(&mut workspace, encrypted.clone(), "user");
        assert!(!user.permissions.owner_authenticated && user.permissions.copy_allowed);
        let mut input = request(&user);
        input.allow_unencrypted = true;
        assert!(workspace.begin_archive(input).is_err());
        let report = workspace.inspect_technical(request(&user).source).unwrap();
        assert!(
            workspace
                .inspection_report(report.inspection_id, false)
                .is_err()
        );
        assert!(
            workspace
                .inspection_report(report.inspection_id, true)
                .is_ok()
        );
        let owner = unlock(&mut workspace, encrypted, "owner");
        assert!(workspace.begin_archive(request(&owner)).is_err());
        let before = workspace.document(owner.id).unwrap().bytes().to_vec();
        let mut input = request(&owner);
        input.allow_unencrypted = true;
        let plan = workspace.begin_archive(input).unwrap();
        let archive = complete(&mut workspace, &plan);
        assert!(!Document::load_mem(&archive).unwrap().is_encrypted());
        assert_eq!(workspace.document(owner.id).unwrap().bytes(), before);
    }
    let mut restricted = Workspace::default();
    let user = unlock(
        &mut restricted,
        fixtures::encrypted(&source, 4, Permissions::PRINTABLE),
        "user",
    );
    assert!(restricted.inspect_technical(request(&user).source).is_err());
    let mut input = request(&user);
    input.allow_unencrypted = true;
    assert!(restricted.begin_archive(input).is_err());
    let mut session = Session::from_bytes_with_password(
        fixtures::encrypted(&source, 4, Permissions::all()),
        "Wrong.pdf".into(),
        None,
        44,
        Some("wrong"),
    );
    assert!(session.is_err());
    session = Session::from_bytes(
        fixtures::encrypted(&source, 4, Permissions::all()),
        "No-owner.pdf".into(),
        None,
        45,
    );
    assert!(session.is_err());
}

#[test]
fn signed_source_needs_explicit_unsigned_ack_and_ordinary_editing_stays_locked() {
    let mut workspace = Workspace::default();
    let signed = workspace
        .open_bytes(fixtures::synthetic_pdf(1, true), "Signed.pdf".into())
        .unwrap();
    assert!(workspace.begin_archive(request(&signed)).is_err());
    let mut input = request(&signed);
    input.acknowledge_unsigned = true;
    let plan = workspace.begin_archive(input).unwrap();
    assert!(plan.signed_source);
    complete(&mut workspace, &plan);
    assert!(
        workspace
            .current_mut(signed.id, signed.revision)
            .unwrap()
            .apply(Edit::Rotate {
                page: 0,
                degrees: 90
            })
            .is_err()
    );
}

#[test]
fn sequence_exact_pixels_alpha_pages_and_noclobber_are_enforced() {
    let mut workspace = Workspace::default();
    let info = open(&mut workspace);
    let mut input = request(&info);
    input.acknowledge_loss = false;
    assert!(workspace.begin_archive(input).is_err());
    let plan = workspace.begin_archive(request(&info)).unwrap();
    assert!(
        workspace
            .stage_archive_page(plan.archive_id, 1, &png(1, 1, false, 255))
            .is_err()
    );
    assert!(
        workspace
            .stage_archive_page(plan.archive_id, 0, &png(1, 1, false, 255))
            .is_err()
    );
    let page = &plan.pages[0];
    assert!(
        workspace
            .stage_archive_page(
                plan.archive_id,
                0,
                &png(page.pixel_width, page.pixel_height, true, 254)
            )
            .is_err()
    );
    let bytes = complete(&mut workspace, &plan);
    assert!(
        workspace
            .stage_archive_page(plan.archive_id, 0, &png(1, 1, false, 255))
            .is_err()
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("archive.pdf");
    workspace
        .write_archive_copy(plan.archive_id, &path)
        .unwrap();
    let saved = std::fs::read(&path).unwrap();
    assert_eq!(saved, bytes);
    assert!(
        workspace
            .write_archive_copy(plan.archive_id, &path)
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), saved);
    assert!(
        workspace
            .write_archive_copy(plan.archive_id, &directory.path().join("wrong.txt"))
            .is_err()
    );
    workspace.set_draft(info.id, true).unwrap();
    assert!(
        workspace
            .write_archive_copy(
                plan.archive_id,
                &directory.path().join("stale-after-dialog.pdf")
            )
            .is_err()
    );
    assert!(!directory.path().join("stale-after-dialog.pdf").exists());
}

#[test]
fn exact_pixel_and_page_limits_never_fall_back_to_lower_quality_or_partial_selection() {
    let mut geometry = Session::blank(72.0, 72.0, 1).unwrap().info().unwrap().pages[0]
        .geometry
        .clone();
    geometry.width_points = 2880.0;
    geometry.height_points = 1920.0;
    assert_eq!(
        archive::page_plans(&[geometry.clone()], &[0], 150).unwrap()[0].pixel_width,
        6000
    );
    assert!(archive::page_plans(&[geometry.clone()], &[0], 300).is_err());
    assert!(archive::page_plans(&[geometry.clone()], &[0], 96).is_err());
    geometry.width_points += 0.01;
    assert!(archive::page_plans(&[geometry.clone()], &[0], 150).is_err());
    geometry.width_points = 7864.319;
    geometry.height_points = 0.48;
    assert_eq!(
        archive::page_plans(&[geometry.clone()], &[0], 150).unwrap()[0].pixel_width,
        16_384
    );
    geometry.width_points += 0.01;
    assert!(archive::page_plans(&[geometry.clone()], &[0], 150).is_err());
    geometry.width_points = 2400.0;
    geometry.height_points = 1920.0; // 20 MP
    assert!(archive::page_plans(&vec![geometry.clone(); 5], &[0, 1, 2, 3, 4], 150).is_ok());
    assert!(archive::page_plans(&vec![geometry.clone(); 6], &[0, 1, 2, 3, 4, 5], 150).is_err());
    geometry.width_points = 1.0;
    geometry.height_points = 1.0;
    assert!(
        archive::page_plans(
            &vec![geometry.clone(); 100],
            &(0..100).collect::<Vec<_>>(),
            150
        )
        .is_ok()
    );
    assert!(
        archive::page_plans(
            &vec![geometry.clone(); 101],
            &(0..101).collect::<Vec<_>>(),
            150
        )
        .is_err()
    );
    for pages in [vec![], vec![0, 0], vec![1, 0], vec![200]] {
        assert!(archive::page_plans(&vec![geometry.clone(); 100], &pages, 150).is_err());
    }
}

#[test]
fn false_conformance_declarations_and_tag_flags_are_only_observations_and_bad_xmp_is_explicit() {
    let mut pdf = Document::load_mem(&fixtures::synthetic_pdf(1, false)).unwrap();
    let metadata = pdf.add_object(Stream::new(dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
        br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><r:RDF xmlns:r="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><r:Description r:about="" xmlns:a="http://www.aiim.org/pdfa/ns/id/" xmlns:u="http://www.aiim.org/pdfua/ns/id/" a:part="2" a:conformance="B"><u:part>1</u:part></r:Description></r:RDF></x:xmpmeta>"#.to_vec()));
    let catalog = pdf.catalog_mut().unwrap();
    catalog.set("Metadata", metadata);
    catalog.set("MarkInfo", dictionary! { "Marked" => true });
    catalog.set(
        "StructTreeRoot",
        dictionary! { "Type" => "StructTreeRoot", "K" => Vec::<Object>::new() },
    );
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).unwrap();
    let mut workspace = Workspace::default();
    let info = workspace
        .open_bytes(bytes.clone(), "False claims.pdf".into())
        .unwrap();
    let report = workspace.inspect_technical(request(&info).source).unwrap();
    assert!(report.input_conformance.contains("NOT VALIDATED"));
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.property == "pdfaid:part"
                && finding.detail.contains("UNVERIFIED"))
    );
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.property == "pdfuaid:part")
    );
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.category == "fonts"
                && finding.state == State::Absent
                && finding.detail.contains("Helvetica"))
    );
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.state == State::Unchecked
                && finding.property.contains("accessibility"))
    );
    assert_eq!(workspace.document(info.id).unwrap().bytes(), bytes);
    let bad = pdf.add_object(Stream::new(
        dictionary! {},
        b"<!DOCTYPE x [<!ENTITY e 'bad'>]><x/>".to_vec(),
    ));
    pdf.catalog_mut().unwrap().set("Metadata", bad);
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).unwrap();
    let info = workspace.open_bytes(bytes, "Bad XMP.pdf".into()).unwrap();
    let report = workspace.inspect_technical(request(&info).source).unwrap();
    assert!(
        report.findings.iter().any(
            |finding| finding.category == "declarations" && finding.state == State::Unsupported
        )
    );
}

#[test]
fn malformed_reference_graphs_and_unsupported_widget_appearances_do_not_produce_success_shaped_results()
 {
    let mut pdf = Document::load_mem(Session::blank(72.0, 72.0, 1).unwrap().bytes()).unwrap();
    let bad = pdf.new_object_id();
    pdf.objects.insert(bad, Object::Reference(bad));
    pdf.catalog_mut().unwrap().set("Bad", bad);
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).unwrap();
    let mut workspace = Workspace::default();
    let info = workspace.open_bytes(bytes, "Cyclic.pdf".into()).unwrap();
    assert!(workspace.inspect_technical(request(&info).source).is_err());
    assert!(workspace.begin_archive(request(&info)).is_err());
    let mut pdf = Document::load_mem(Session::blank(72.0, 72.0, 1).unwrap().bytes()).unwrap();
    let widget = pdf.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Widget", "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()], "FT" => "Tx", "T" => Object::string_literal("field"), "V" => Object::string_literal("Visible value") });
    let page = *pdf.get_pages().values().next().unwrap();
    pdf.get_dictionary_mut(page)
        .unwrap()
        .set("Annots", vec![Object::Reference(widget)]);
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).unwrap();
    let info = workspace
        .open_bytes(bytes, "Missing appearance.pdf".into())
        .unwrap();
    assert!(workspace.begin_archive(request(&info)).is_err());
}
