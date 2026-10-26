#[path = "../examples/output_fixtures.rs"]
mod fixtures;

use folio_core::{
    comparison::{
        MAX_PAGE_PIXELS, MAX_PAGE_TEXT_BYTES, MAX_REPORT_BYTES, MAX_SPANS, MAX_TEXT_BYTES,
        MAX_TOKENS, PageResult, PixelResult, Plan, Request, Settings, Side, SourceRef, SpanKind,
        TextResult, TextSpan, geometry_changes, page_plans,
    },
    document::{DocumentInfo, Edit, Session},
    workspace::{OpenOutcome, Workspace},
};
use lopdf::{Document, encryption::Permissions};

fn open(workspace: &mut Workspace, name: &str) -> DocumentInfo {
    workspace
        .open_bytes(
            Session::blank(72.0, 72.0, 1).unwrap().bytes().to_vec(),
            name.into(),
        )
        .unwrap()
}

fn request(before: &DocumentInfo, after: &DocumentInfo) -> Request {
    Request {
        before: SourceRef {
            document_id: before.id,
            revision: before.revision,
        },
        after: SourceRef {
            document_id: after.id,
            revision: after.revision,
        },
        settings: Settings {
            dpi: 72,
            tolerance: 0,
        },
    }
}

fn results(plan: &Plan) -> Vec<PageResult> {
    plan.pages
        .iter()
        .map(|page| PageResult {
            index: page.index,
            text: TextResult {
                before_tokens: 0,
                after_tokens: 0,
                spans: vec![],
            },
            pixels: PixelResult {
                width: page.width,
                height: page.height,
                changed: 0,
                total: u64::from(page.width) * u64::from(page.height),
                max_channel_delta: 0,
                bounds: None,
            },
        })
        .collect()
}

fn pair() -> (Workspace, DocumentInfo, DocumentInfo) {
    let mut workspace = Workspace::default();
    let before = open(&mut workspace, "Before.pdf");
    let after = open(&mut workspace, "After.pdf");
    (workspace, before, after)
}

fn unlock(workspace: &mut Workspace, bytes: Vec<u8>, password: &str) -> DocumentInfo {
    let outcome = workspace
        .request_open_bytes(bytes, "Protected.pdf".into())
        .unwrap();
    let OpenOutcome::PasswordRequired { request_id, .. } = outcome else {
        panic!("Expected a password challenge");
    };
    let OpenOutcome::Opened { document, .. } =
        workspace.unlock_document(request_id, password).unwrap()
    else {
        panic!("Expected unlocked PDF");
    };
    document
}

#[test]
fn snapshots_use_unsaved_work_and_comparison_never_changes_either_source_or_active_tab() {
    let (mut workspace, before, after) = pair();
    let changed = workspace
        .current_mut(before.id, before.revision)
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    let before_bytes = workspace.document(before.id).unwrap().bytes().to_vec();
    let after_bytes = workspace.document(after.id).unwrap().bytes().to_vec();
    let before_info =
        serde_json::to_value(workspace.document(before.id).unwrap().info().unwrap()).unwrap();
    let after_info =
        serde_json::to_value(workspace.document(after.id).unwrap().info().unwrap()).unwrap();
    let plan = workspace
        .begin_comparison(request(&changed, &after))
        .unwrap();
    assert_eq!(plan.before.pages[0].rotation, 90);
    assert_eq!(
        workspace
            .comparison_render_bytes(plan.comparison_id, Side::Before)
            .unwrap(),
        before_bytes
    );
    assert_eq!(
        workspace
            .comparison_render_bytes(plan.comparison_id, Side::After)
            .unwrap(),
        after_bytes
    );
    workspace
        .finish_comparison(plan.comparison_id, results(&plan))
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(
        &workspace
            .comparison_report(plan.comparison_id, false)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(report["schema"], "folio.local-comparison");
    assert_eq!(report["schemaVersion"], 1);
    assert_eq!(report["matching"], "page_index");
    assert_eq!(report["pages"][0]["index"], 0);
    assert_eq!(report["pages"][0]["geometryChanges"][0], "rotation");
    assert!(
        report["scope"]
            .as_str()
            .unwrap()
            .contains("signature validity")
    );
    assert!(
        report["conclusion"]
            .as_str()
            .unwrap()
            .contains("Other document content was not compared")
    );
    assert_eq!(workspace.current().unwrap().id, after.id);
    assert_eq!(workspace.document(before.id).unwrap().bytes(), before_bytes);
    assert_eq!(workspace.document(after.id).unwrap().bytes(), after_bytes);
    assert_eq!(
        serde_json::to_value(workspace.document(before.id).unwrap().info().unwrap()).unwrap(),
        before_info
    );
    assert_eq!(
        serde_json::to_value(workspace.document(after.id).unwrap().info().unwrap()).unwrap(),
        after_info
    );
}

#[test]
fn empty_page_matches_never_claim_whole_document_equivalence_and_reports_are_separate_noclobber_copies()
 {
    let (mut workspace, before, after) = pair();
    let plan = workspace
        .begin_comparison(request(&before, &after))
        .unwrap();
    assert!(
        workspace
            .comparison_report(plan.comparison_id, false)
            .is_err()
    );
    workspace
        .finish_comparison(plan.comparison_id, results(&plan))
        .unwrap();
    let bytes = workspace
        .comparison_report(plan.comparison_id, false)
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        report["conclusion"]
            .as_str()
            .unwrap()
            .contains("does not mean the PDFs are identical")
    );
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("comparison.json");
    workspace
        .write_comparison_report(plan.comparison_id, false, &path)
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(
        workspace
            .write_comparison_report(plan.comparison_id, false, &path)
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(
        workspace
            .write_comparison_report(
                plan.comparison_id,
                false,
                &temporary.path().join("comparison.pdf")
            )
            .is_err()
    );
}

#[test]
fn either_source_draft_or_close_invalidates_a_comparison_permanently() {
    for which in [0, 1] {
        let (mut workspace, before, after) = pair();
        let source = [&before, &after][which];
        workspace.set_draft(source.id, true).unwrap();
        assert!(
            workspace
                .begin_comparison(request(&before, &after))
                .is_err()
        );
        workspace.set_draft(source.id, false).unwrap();
        let plan = workspace
            .begin_comparison(request(&before, &after))
            .unwrap();
        workspace.set_draft(source.id, true).unwrap();
        workspace.set_draft(source.id, false).unwrap();
        assert!(workspace.check_comparison(plan.comparison_id).is_err());
        assert!(
            workspace
                .comparison_render_bytes(plan.comparison_id, Side::Before)
                .is_err()
        );
        assert!(
            workspace
                .finish_comparison(plan.comparison_id, results(&plan))
                .is_err()
        );
        assert!(
            workspace
                .comparison_report(plan.comparison_id, true)
                .is_err()
        );
        let new_plan = workspace
            .begin_comparison(request(&before, &after))
            .unwrap();
        workspace
            .close_document(source.id, source.revision, true)
            .unwrap();
        assert!(workspace.check_comparison(new_plan.comparison_id).is_err());
        let reopened = open(&mut workspace, &source.name);
        assert_ne!(reopened.id, source.id);
        assert!(workspace.check_comparison(new_plan.comparison_id).is_err());
    }
}

#[test]
fn mutation_then_undo_cannot_resurrect_results_even_when_the_revision_matches_again() {
    for which in [0, 1] {
        let (mut workspace, before, after) = pair();
        let source = [&before, &after][which];
        let plan = workspace
            .begin_comparison(request(&before, &after))
            .unwrap();
        workspace
            .finish_comparison(plan.comparison_id, results(&plan))
            .unwrap();
        let changed = workspace
            .current_mut(source.id, source.revision)
            .unwrap()
            .apply(Edit::Rotate {
                page: 0,
                degrees: 90,
            })
            .unwrap();
        let undone = workspace
            .current_mut(source.id, changed.revision)
            .unwrap()
            .undo()
            .unwrap();
        assert_eq!(undone.revision, source.revision);
        assert!(workspace.check_comparison(plan.comparison_id).is_err());
        assert!(
            workspace
                .comparison_report(plan.comparison_id, true)
                .is_err()
        );
        let plan = workspace
            .begin_comparison(request(&before, &after))
            .unwrap();
        workspace
            .current_mut(source.id, undone.revision)
            .unwrap()
            .redo()
            .unwrap();
        assert!(workspace.check_comparison(plan.comparison_id).is_err());
    }
}

#[test]
fn pending_backend_redaction_and_staged_authoring_images_block_both_sides_before_draft_projection()
{
    for which in [0, 1] {
        let (mut workspace, before, after) = pair();
        let source = [&before, &after][which];
        let plan = workspace
            .begin_comparison(request(&before, &after))
            .unwrap();
        let redaction = workspace
            .begin_redaction(
                source.id,
                source.revision,
                150,
                &[folio_core::redaction::Region {
                    page: 0,
                    rect: [2.0, 2.0, 12.0, 12.0],
                }],
            )
            .unwrap();
        assert!(workspace.check_comparison(plan.comparison_id).is_err());
        assert!(
            workspace
                .begin_comparison(request(&before, &after))
                .is_err()
        );
        workspace.cancel_redaction(redaction.job_id);
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[0, 0, 0, 255])
                .unwrap();
        }
        let image = workspace
            .stage_authoring_image(source.id, source.revision, &png)
            .unwrap();
        assert!(
            workspace
                .begin_comparison(request(&before, &after))
                .is_err()
        );
        workspace
            .cancel_authoring_image(source.id, image.asset_id)
            .unwrap();
        let plan = workspace
            .begin_comparison(request(&before, &after))
            .unwrap();
        let image = workspace
            .stage_authoring_image(source.id, source.revision, &png)
            .unwrap();
        workspace
            .cancel_authoring_image(source.id, image.asset_id)
            .unwrap();
        assert!(workspace.check_comparison(plan.comparison_id).is_err());
    }
}

#[test]
fn copy_permission_not_editability_controls_comparison_for_signed_and_protected_inputs() {
    let mut workspace = Workspace::default();
    let signed_bytes = fixtures::synthetic_pdf(1, true);
    let signed = workspace
        .open_bytes(signed_bytes.clone(), "Signed marker.pdf".into())
        .unwrap();
    assert!(signed.read_only_reason.is_some());
    let encrypted =
        fixtures::encrypted(&fixtures::synthetic_pdf(1, false), 4, Permissions::COPYABLE);
    let allowed = unlock(&mut workspace, encrypted.clone(), "user");
    assert!(allowed.read_only_reason.is_some() && allowed.permissions.copy_allowed);
    let plan = workspace
        .begin_comparison(request(&signed, &allowed))
        .unwrap();
    let render = workspace
        .comparison_render_bytes(plan.comparison_id, Side::After)
        .unwrap();
    assert!(!Document::load_mem(&render).unwrap().is_encrypted());
    workspace
        .finish_comparison(plan.comparison_id, results(&plan))
        .unwrap();
    assert!(
        workspace
            .comparison_report(plan.comparison_id, false)
            .is_err()
    );
    let report = workspace
        .comparison_report(plan.comparison_id, true)
        .unwrap();
    assert!(
        String::from_utf8(report)
            .unwrap()
            .contains("unencrypted extracted document content")
    );
    assert_eq!(workspace.document(signed.id).unwrap().bytes(), signed_bytes);
    assert_eq!(workspace.document(allowed.id).unwrap().bytes(), encrypted);
    workspace.cancel_comparison(plan.comparison_id);
    let restricted_bytes = fixtures::encrypted(
        &fixtures::synthetic_pdf(1, false),
        4,
        Permissions::PRINTABLE,
    );
    let restricted = unlock(&mut workspace, restricted_bytes.clone(), "user");
    assert!(
        workspace
            .begin_comparison(request(&signed, &restricted))
            .is_err()
    );
    assert!(
        workspace
            .begin_comparison(request(&restricted, &allowed))
            .is_err()
    );
    let owner = unlock(&mut workspace, restricted_bytes, "owner");
    assert!(owner.permissions.copy_allowed);
    assert!(workspace.begin_comparison(request(&signed, &owner)).is_ok());
}

#[test]
fn one_job_limit_and_explicit_cancel_do_not_touch_the_sources_or_other_output_jobs() {
    let (mut workspace, before, after) = pair();
    assert!(
        workspace
            .begin_comparison(request(&before, &before))
            .is_err()
    );
    let plan = workspace
        .begin_comparison(request(&before, &after))
        .unwrap();
    assert!(
        workspace
            .begin_comparison(request(&before, &after))
            .is_err()
    );
    assert!(!workspace.cancel_comparison(plan.comparison_id + 1));
    assert!(workspace.check_comparison(plan.comparison_id).is_ok());
    assert!(workspace.cancel_comparison(plan.comparison_id));
    assert!(workspace.check_comparison(plan.comparison_id).is_err());
    let next = workspace
        .begin_comparison(request(&before, &after))
        .unwrap();
    assert!(next.comparison_id > plan.comparison_id);
    assert!(!workspace.has_unsaved_changes());
    assert_eq!(
        workspace.document(before.id).unwrap().revision(),
        before.revision
    );
    assert_eq!(
        workspace.document(after.id).unwrap().revision(),
        after.revision
    );
}

#[test]
fn native_geometry_and_page_pixel_budgets_match_the_frontend_at_exact_thresholds() {
    let (workspace, before, _) = pair();
    let mut geometry = workspace.document(before.id).unwrap().info().unwrap().pages[0]
        .geometry
        .clone();
    geometry.width_points = 4000.0;
    geometry.height_points = 2000.0;
    let page = page_plans(&[geometry.clone()], &[geometry.clone()], 72).unwrap();
    assert_eq!(
        u64::from(page[0].width) * u64::from(page[0].height),
        MAX_PAGE_PIXELS
    );
    geometry.width_points = 4000.01;
    assert!(page_plans(&[geometry.clone()], &[geometry.clone()], 72).is_err());
    geometry.width_points = 8192.0;
    geometry.height_points = 1.0;
    assert_eq!(
        page_plans(&[geometry.clone()], &[geometry.clone()], 72).unwrap()[0].width,
        8192
    );
    geometry.width_points = 8192.01;
    assert!(page_plans(&[geometry.clone()], &[geometry.clone()], 72).is_err());
    geometry.width_points = 1000.0;
    geometry.height_points = 1000.0;
    assert_eq!(
        page_plans(&vec![geometry.clone(); 100], &[geometry.clone()], 72)
            .unwrap()
            .len(),
        100
    );
    assert!(page_plans(&vec![geometry.clone(); 101], &[geometry.clone()], 72).is_err());
    geometry.width_points = 1000.01;
    assert!(page_plans(&vec![geometry.clone(); 100], &[geometry.clone()], 72).is_err());
    let mut other = geometry.clone();
    other.rotation = 270;
    other.user_unit = 2.0;
    other.bounds[0] = 3.0;
    assert_eq!(
        geometry_changes(Some(&geometry), Some(&other)),
        ["effective_crop", "rotation", "user_unit"]
    );
}

#[test]
fn report_validation_rejects_partial_misordered_malformed_and_over_budget_results() {
    let (mut workspace, before, after) = pair();
    let plan = workspace
        .begin_comparison(request(&before, &after))
        .unwrap();
    assert!(
        workspace
            .finish_comparison(plan.comparison_id, vec![])
            .is_err()
    );
    let mut invalid = results(&plan);
    invalid[0].index = 1;
    assert!(
        workspace
            .finish_comparison(plan.comparison_id, invalid)
            .is_err()
    );
    let mut invalid = results(&plan);
    invalid[0].pixels.changed = invalid[0].pixels.total + 1;
    assert!(
        workspace
            .finish_comparison(plan.comparison_id, invalid)
            .is_err()
    );
    let mut invalid = results(&plan);
    invalid[0].pixels.max_channel_delta = 1;
    assert!(
        workspace
            .finish_comparison(plan.comparison_id, invalid)
            .is_err()
    );
    let mut invalid = results(&plan);
    invalid[0].pixels.width += 1;
    assert!(
        workspace
            .finish_comparison(plan.comparison_id, invalid)
            .is_err()
    );
    let mut invalid = results(&plan);
    invalid[0].text.spans.push(TextSpan {
        kind: SpanKind::Equal,
        text: String::new(),
    });
    assert!(
        workspace
            .finish_comparison(plan.comparison_id, invalid)
            .is_err()
    );
    let mut valid = results(&plan);
    valid[0].text.spans = vec![TextSpan {
        kind: SpanKind::Equal,
        text: "a".repeat(MAX_PAGE_TEXT_BYTES),
    }];
    valid[0].text.before_tokens = 1;
    valid[0].text.after_tokens = 1;
    workspace
        .finish_comparison(plan.comparison_id, valid.clone())
        .unwrap();
    valid[0].text.spans[0].text.push('a');
    assert!(
        workspace
            .finish_comparison(plan.comparison_id, valid)
            .is_err()
    );
    let mut valid = results(&plan);
    valid[0].text.spans = (0..MAX_SPANS)
        .map(|_| TextSpan {
            kind: SpanKind::Equal,
            text: "a".into(),
        })
        .collect();
    workspace
        .finish_comparison(plan.comparison_id, valid.clone())
        .unwrap();
    valid[0].text.spans.push(TextSpan {
        kind: SpanKind::Equal,
        text: "a".into(),
    });
    assert!(
        workspace
            .finish_comparison(plan.comparison_id, valid)
            .is_err()
    );
    let mut invalid = results(&plan);
    invalid[0].text.before_tokens = 20_001;
    assert!(
        workspace
            .finish_comparison(plan.comparison_id, invalid)
            .is_err()
    );
}

#[test]
fn report_json_encoded_bytes_have_a_hard_limit_even_when_text_is_under_its_limit() {
    let mut workspace = Workspace::default();
    let bytes = fixtures::synthetic_pdf(4, false);
    let before = workspace
        .open_bytes(bytes.clone(), "Before.pdf".into())
        .unwrap();
    let after = workspace.open_bytes(bytes, "After.pdf".into()).unwrap();
    let plan = workspace
        .begin_comparison(request(&before, &after))
        .unwrap();
    let mut pages = results(&plan);
    for page in &mut pages {
        page.text.spans.push(TextSpan {
            kind: SpanKind::Equal,
            text: "\0".repeat(MAX_PAGE_TEXT_BYTES),
        });
    }
    assert!(
        workspace
            .finish_comparison(plan.comparison_id, pages)
            .is_err_and(|error| error.to_string().contains("4 MiB"))
    );
    assert_eq!(MAX_REPORT_BYTES, 4 * 1024 * 1024);
}

#[test]
fn report_global_text_and_token_budgets_accept_exact_limits_then_reject_one_over() {
    let mut workspace = Workspace::default();
    let bytes = fixtures::synthetic_pdf(5, false);
    let before = workspace
        .open_bytes(bytes.clone(), "Before.pdf".into())
        .unwrap();
    let after = workspace.open_bytes(bytes, "After.pdf".into()).unwrap();
    let plan = workspace
        .begin_comparison(request(&before, &after))
        .unwrap();
    let mut pages = results(&plan);
    for page in &mut pages[..4] {
        page.text.spans = vec![TextSpan {
            kind: SpanKind::Equal,
            text: "a".repeat(MAX_PAGE_TEXT_BYTES),
        }];
        page.text.before_tokens = 1;
        page.text.after_tokens = 1;
    }
    assert_eq!(4 * 2 * MAX_PAGE_TEXT_BYTES, MAX_TEXT_BYTES);
    workspace
        .finish_comparison(plan.comparison_id, pages.clone())
        .unwrap();
    pages[4].text.spans.push(TextSpan {
        kind: SpanKind::Added,
        text: "a".into(),
    });
    pages[4].text.after_tokens = 1;
    assert!(
        workspace
            .finish_comparison(plan.comparison_id, pages)
            .is_err_and(|error| error.to_string().contains("2 MiB"))
    );

    let mut pages = results(&plan);
    for page in &mut pages[..2] {
        page.text.spans = vec![TextSpan {
            kind: SpanKind::Equal,
            text: "a ".repeat(10_000),
        }];
        page.text.before_tokens = 20_000;
        page.text.after_tokens = 20_000;
    }
    pages[2].text.spans = vec![TextSpan {
        kind: SpanKind::Removed,
        text: "a ".repeat(10_000),
    }];
    pages[2].text.before_tokens = 20_000;
    assert_eq!(
        pages
            .iter()
            .map(|page| page.text.before_tokens + page.text.after_tokens)
            .sum::<usize>(),
        MAX_TOKENS
    );
    workspace
        .finish_comparison(plan.comparison_id, pages.clone())
        .unwrap();
    pages[2].text.spans.push(TextSpan {
        kind: SpanKind::Added,
        text: "a".into(),
    });
    pages[2].text.after_tokens = 1;
    assert!(
        workspace
            .finish_comparison(plan.comparison_id, pages)
            .is_err_and(|error| error.to_string().contains("100,000"))
    );
}

#[test]
fn a_real_detached_cms_signed_pdf_can_be_compared_without_rewriting_its_signed_bytes() {
    use openssl::{
        asn1::{Asn1Integer, Asn1Time},
        bn::BigNum,
        ec::{EcGroup, EcKey},
        hash::MessageDigest,
        nid::Nid,
        pkcs12::Pkcs12,
        pkey::PKey,
        x509::{X509, X509NameBuilder, extension::KeyUsage},
    };
    let key = PKey::from_ec_key(
        EcKey::generate(&EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap()).unwrap(),
    )
    .unwrap();
    let mut name = X509NameBuilder::new().unwrap();
    name.append_entry_by_text("CN", "Original comparison test signer")
        .unwrap();
    let name = name.build();
    let mut certificate = X509::builder().unwrap();
    certificate.set_version(2).unwrap();
    certificate
        .set_serial_number(&Asn1Integer::from_bn(&BigNum::from_u32(42).unwrap()).unwrap())
        .unwrap();
    certificate.set_subject_name(&name).unwrap();
    certificate.set_issuer_name(&name).unwrap();
    certificate.set_pubkey(&key).unwrap();
    certificate
        .set_not_before(&Asn1Time::days_from_now(0).unwrap())
        .unwrap();
    certificate
        .set_not_after(&Asn1Time::days_from_now(1).unwrap())
        .unwrap();
    certificate
        .append_extension(KeyUsage::new().digital_signature().build().unwrap())
        .unwrap();
    certificate.sign(&key, MessageDigest::sha256()).unwrap();
    let certificate = certificate.build();
    let identity = Pkcs12::builder()
        .name("Synthetic test only")
        .pkey(&key)
        .cert(&certificate)
        .build2("synthetic")
        .unwrap()
        .to_der()
        .unwrap();
    let bytes = Session::blank(72.0, 72.0, 1)
        .unwrap()
        .signed_copy(&identity, "synthetic")
        .unwrap();
    let mut workspace = Workspace::default();
    let signed = workspace
        .open_bytes(bytes.clone(), "Actually signed.pdf".into())
        .unwrap();
    let after = open(&mut workspace, "After.pdf");
    assert!(signed.read_only_reason.is_some());
    assert!(signed.permissions.copy_allowed);
    let plan = workspace
        .begin_comparison(request(&signed, &after))
        .unwrap();
    assert_eq!(
        workspace
            .comparison_render_bytes(plan.comparison_id, Side::Before)
            .unwrap(),
        bytes
    );
    workspace
        .finish_comparison(plan.comparison_id, results(&plan))
        .unwrap();
    workspace
        .comparison_report(plan.comparison_id, false)
        .unwrap();
    assert_eq!(workspace.document(signed.id).unwrap().bytes(), bytes);
    assert!(!workspace.document(signed.id).unwrap().is_dirty());
    assert!(
        !workspace
            .document(signed.id)
            .unwrap()
            .info()
            .unwrap()
            .can_undo
    );
}
