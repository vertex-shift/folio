use folio_core::{
    accessibility::{Assignment, CandidateKind, CopyRequest, Inspection, Role, SemanticNode},
    archive::SourceRef,
    document::{DocumentInfo, Edit, Session},
    paragraph::{Alignment, ParagraphChange},
    protection::CopyProtection,
    text_edit::ReplacementFont,
    workspace::{OpenOutcome, Workspace},
};
use lopdf::{Document, Object, Stream, dictionary};

fn bytes(mut document: Document) -> Vec<u8> {
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

fn fixture() -> Document {
    let mut document = Document::with_version("1.7");
    let pages = document.new_object_id();
    let font = document.add_object(dictionary! { "Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica","Encoding"=>"WinAnsiEncoding" });
    let image = document.add_object(Stream::new(dictionary! {
        "Type"=>"XObject","Subtype"=>"Image","Width"=>1,"Height"=>1,"ColorSpace"=>"DeviceRGB","BitsPerComponent"=>8,
    }, vec![200, 30, 60]));
    let content = document.add_object(Stream::new(
        dictionary! {},
        b"q\nBT /F1 20 Tf 1 0 0 1 32 720 Tm (Original semantic heading) Tj ET\nQ\n\
BT /F1 11 Tf 1 0 0 1 320 650 Tm (Right drawn first, read second.) Tj ET\n\
BT /F1 11 Tf 1 0 0 1 32 650 Tm (Left must be read first.) Tj ET\n\
q 60 0 0 60 32 520 cm /Im Do Q\n\
0.123456789 0.4 0.7 rg 31.123456789 400 80 20 re f\n\
BT /F1 11 Tf 1 0 0 1 32 350 Tm (End of original searchable source.) Tj ET\n"
            .to_vec(),
    ));
    let page = document.add_object(dictionary! { "Type"=>"Page","Parent"=>pages,"MediaBox"=>vec![0.into(),0.into(),612.into(),792.into()],
        "Resources"=>dictionary! { "Font"=>dictionary! { "F1"=>font },"XObject"=>dictionary! { "Im"=>image } },"Contents"=>content });
    document.objects.insert(
        pages,
        dictionary! { "Type"=>"Pages","Kids"=>vec![Object::Reference(page)],"Count"=>1 }.into(),
    );
    let root = document.add_object(dictionary! { "Type"=>"Catalog","Pages"=>pages });
    document.trailer.set("Root", root);
    document
}

fn open(workspace: &mut Workspace) -> DocumentInfo {
    workspace
        .open_bytes(bytes(fixture()), "Semantics.pdf".into())
        .unwrap()
}
fn inspect(workspace: &mut Workspace, info: &DocumentInfo) -> Inspection {
    workspace
        .inspect_accessibility(SourceRef {
            document_id: info.id,
            revision: info.revision,
        })
        .unwrap()
}
fn request(report: &Inspection) -> CopyRequest {
    let mut entries: Vec<_> = report
        .candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| Assignment {
            candidate_id: candidate.id.clone(),
            role: if index == 0 {
                Role::H1
            } else {
                match candidate.kind {
                    CandidateKind::Text => Role::P,
                    CandidateKind::Figure => Role::Figure,
                    CandidateKind::Graphic => Role::Artifact,
                }
            },
            alt: (candidate.kind == CandidateKind::Figure)
                .then(|| "A crimson square, supplied by the human author.".into()),
            language: (index == 2).then(|| "fr-FR".into()),
        })
        .collect();
    if entries.len() >= 3 {
        entries.swap(1, 2);
    }
    CopyRequest {
        source: report.source.clone(),
        source_epoch: report.source_epoch,
        title: "Reviewed original document".into(),
        language: "en-US".into(),
        entries,
        reviewed: true,
    }
}
fn leaves<'a>(nodes: &'a [SemanticNode], result: &mut Vec<&'a SemanticNode>) {
    for node in nodes {
        if node.children.is_empty() {
            result.push(node);
        } else {
            leaves(&node.children, result);
        }
    }
}
fn tagged() -> Vec<u8> {
    let mut workspace = Workspace::default();
    let info = open(&mut workspace);
    let report = inspect(&mut workspace, &info);
    let plan = workspace
        .begin_accessibility_copy(request(&report))
        .unwrap();
    workspace.accessibility_copy_bytes(plan.job_id).unwrap()
}
fn reopened(bytes: Vec<u8>) -> Inspection {
    let mut workspace = Workspace::default();
    let info = workspace.open_bytes(bytes, "Tagged.pdf".into()).unwrap();
    inspect(&mut workspace, &info)
}

#[test]
fn genuine_tagged_copy_preserves_source_history_pixels_operands_and_bound_logical_order() {
    let mut workspace = Workspace::default();
    let info = open(&mut workspace);
    let original = workspace.document(info.id).unwrap().bytes().to_vec();
    let state = serde_json::to_value(&info).unwrap();
    let report = inspect(&mut workspace, &info);
    assert_eq!(report.status, "untagged");
    assert!(report.authoring_allowed);
    assert_eq!(report.candidates.len(), 6);
    let plan = workspace
        .begin_accessibility_copy(request(&report))
        .unwrap();
    assert!(
        workspace
            .close_document(info.id, info.revision, false)
            .is_err()
    );
    let output = workspace.accessibility_copy_bytes(plan.job_id).unwrap();
    assert_eq!(workspace.document(info.id).unwrap().bytes(), original);
    assert_eq!(
        serde_json::to_value(workspace.document(info.id).unwrap().info().unwrap()).unwrap(),
        state
    );
    assert!(workspace.document(info.id).unwrap().source_path.is_none());
    assert!(workspace.document(info.id).unwrap().saved_path.is_none());
    let pdf = Document::load_mem(&output).unwrap();
    let page = pdf.get_pages()[&1];
    let content = pdf.get_page_content(page);
    assert!(
        content
            .windows(b"31.123456789".len())
            .any(|part| part == b"31.123456789"),
        "drawing numbers stay byte-exact"
    );
    let result = reopened(output);
    assert_eq!(result.status, "tagged");
    assert!(result.complete, "{:?}", result.issues);
    assert!(!result.authoring_allowed);
    assert_eq!(result.language.as_deref(), Some("en-US"));
    assert_eq!(result.title, "Reviewed original document");
    let mut nodes = Vec::new();
    leaves(&result.structure, &mut nodes);
    assert_eq!(
        nodes
            .iter()
            .map(|node| node.role.as_str())
            .collect::<Vec<_>>(),
        ["H1", "P", "P", "Figure", "P"]
    );
    assert_eq!(nodes[1].text.as_deref(), Some("Left must be read first."));
    assert_eq!(nodes[1].language.as_deref(), Some("fr-FR"));
    assert_eq!(
        nodes[2].text.as_deref(),
        Some("Right drawn first, read second.")
    );
    assert_eq!(
        nodes[3].alt.as_deref(),
        Some("A crimson square, supplied by the human author.")
    );
    assert!(nodes.iter().all(|node| node.bindings.len() == 1));
    assert!(workspace.cancel_accessibility_copy(plan.job_id));
    assert!(workspace.accessibility_copy_bytes(plan.job_id).is_err());
}

#[test]
fn no_candidate_is_silently_omitted_or_given_automatic_semantics() {
    let mut workspace = Workspace::default();
    let info = open(&mut workspace);
    let report = inspect(&mut workspace, &info);
    for variant in 0..7 {
        let mut input = request(&report);
        match variant {
            0 => {
                input.entries.pop();
            }
            1 => {
                input.entries[1] = input.entries[0].clone();
            }
            2 => {
                input
                    .entries
                    .iter_mut()
                    .find(|entry| entry.role == Role::Figure)
                    .unwrap()
                    .alt = Some("  ".into());
            }
            3 => {
                input.reviewed = false;
            }
            4 => {
                input.language = "not_a_language".into();
            }
            5 => {
                input.entries.iter_mut().for_each(|entry| {
                    entry.role = Role::Artifact;
                    entry.alt = None;
                });
            }
            _ => {
                input.entries[0].role = Role::Figure;
                input.entries[0].alt =
                    Some("Do not turn searchable text into an image description.".into());
            }
        }
        assert!(
            workspace.begin_accessibility_copy(input).is_err(),
            "variant {variant}"
        );
    }
}

#[test]
fn actual_native_preview_request_shape_rejects_unknown_fields() {
    let mut workspace = Workspace::default();
    let info = open(&mut workspace);
    let report = inspect(&mut workspace, &info);
    let input = request(&report);
    let mut json = serde_json::json!({ "documentId":info.id,"revision":0,"sourceEpoch":report.source_epoch,
        "title":input.title,"language":input.language,"entries":input.entries,"reviewed":true });
    assert!(serde_json::from_value::<CopyRequest>(json.clone()).is_ok());
    json["allowUnencrypted"] = true.into();
    assert!(serde_json::from_value::<CopyRequest>(json).is_err());
}

#[test]
fn reflowed_logical_paragraphs_get_unique_form_scopes_and_real_stm_associations() {
    let mut workspace = Workspace::default();
    let info = open(&mut workspace);
    let block = workspace
        .document(info.id)
        .unwrap()
        .paragraph_blocks(0)
        .unwrap()
        .blocks
        .remove(0);
    let text = "An original paragraph with genuine wrapping that stays logically whole.\n\nAn explicit paragraph break remains.";
    let changed = workspace
        .edit_paragraph(
            info.id,
            0,
            ParagraphChange {
                epoch: 0,
                page: 0,
                path: block.path,
                block_id: block.id,
                expected: block.text.unwrap(),
                text: text.into(),
                placement: [32.0, 40.0, 260.0, 150.0],
                size: 12.0,
                leading: 18.0,
                alignment: Alignment::Left,
                font: ReplacementFont::Noto,
            },
        )
        .unwrap();
    let report = inspect(&mut workspace, &changed);
    assert!(report.authoring_allowed, "{:?}", report.issues);
    assert!(
        report
            .candidates
            .iter()
            .any(|candidate| candidate.text.as_deref() == Some(text))
    );
    let plan = workspace
        .begin_accessibility_copy(request(&report))
        .unwrap();
    let output = workspace.accessibility_copy_bytes(plan.job_id).unwrap();
    let result = reopened(output.clone());
    assert!(result.complete, "{:?}", result.issues);
    let mut nodes = Vec::new();
    leaves(&result.structure, &mut nodes);
    assert_eq!(nodes[0].text.as_deref(), Some(text));
    assert!(nodes[0].bindings[0].stream.is_some());
    let pdf = Document::load_mem(&output).unwrap();
    let form = pdf
        .objects
        .values()
        .filter_map(|value| value.as_stream().ok())
        .find(|value| value.dict.has(b"StructParents"))
        .unwrap();
    assert!(form.dict.has(b"FolioParagraph"));
    assert_eq!(
        workspace.document(info.id).unwrap().revision(),
        changed.revision
    );
}

#[test]
fn malformed_cycles_shared_nodes_parent_mappings_empty_alts_and_role_maps_are_refused() {
    for variant in 0..6 {
        let mut pdf = Document::load_mem(&tagged()).unwrap();
        let root = pdf
            .catalog()
            .unwrap()
            .get(b"StructTreeRoot")
            .unwrap()
            .as_reference()
            .unwrap();
        let document = pdf
            .get_dictionary(root)
            .unwrap()
            .get(b"K")
            .unwrap()
            .as_array()
            .unwrap()[0]
            .as_reference()
            .unwrap();
        let kids = pdf
            .get_dictionary(document)
            .unwrap()
            .get(b"K")
            .unwrap()
            .as_array()
            .unwrap()
            .clone();
        let first = kids[0].as_reference().unwrap();
        match variant {
            0 => {
                pdf.get_dictionary_mut(first)
                    .unwrap()
                    .set("K", vec![Object::Reference(document)]);
            }
            1 => {
                let mut shared = kids;
                shared.push(shared[0].clone());
                pdf.get_dictionary_mut(document).unwrap().set("K", shared);
            }
            2 => {
                let parent = pdf
                    .get_dictionary(root)
                    .unwrap()
                    .get(b"ParentTree")
                    .unwrap()
                    .as_reference()
                    .unwrap();
                pdf.get_dictionary_mut(parent).unwrap().set(
                    "Nums",
                    vec![0.into(), Object::Array(vec![Object::Reference(document)])],
                );
            }
            3 => {
                let figure = kids
                    .iter()
                    .map(|value| value.as_reference().unwrap())
                    .find(|id| {
                        pdf.get_dictionary(*id)
                            .unwrap()
                            .get(b"S")
                            .unwrap()
                            .as_name()
                            .unwrap()
                            == b"Figure"
                    })
                    .unwrap();
                pdf.get_dictionary_mut(figure)
                    .unwrap()
                    .set("Alt", Object::string_literal(""));
            }
            4 => {
                pdf.get_dictionary_mut(root)
                    .unwrap()
                    .set("RoleMap", dictionary! { "One"=>"Two","Two"=>"One" });
            }
            _ => {
                pdf.get_dictionary_mut(first)
                    .unwrap()
                    .set("ID", Object::string_literal("no-matching-id-tree"));
            }
        }
        let report = reopened(bytes(pdf));
        assert_eq!(
            report.status, "malformed_or_unsupported",
            "variant {variant}: {:?}",
            report.issues
        );
        assert!(!report.complete && report.structure.is_empty());
    }
}

#[test]
fn draft_mutations_undo_resurrection_and_cancel_invalidate_bound_jobs_and_inspections() {
    let mut workspace = Workspace::default();
    let info = open(&mut workspace);
    let report = inspect(&mut workspace, &info);
    workspace.set_draft(info.id, true).unwrap();
    assert!(
        workspace
            .check_accessibility_inspection(report.inspection_id)
            .is_err()
    );
    assert!(
        workspace
            .begin_accessibility_copy(request(&report))
            .is_err()
    );
    workspace.set_draft(info.id, false).unwrap();
    let plan = workspace
        .begin_accessibility_copy(request(&report))
        .unwrap();
    let changed = workspace
        .current_mut(info.id, 0)
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    let restored = workspace
        .current_mut(info.id, changed.revision)
        .unwrap()
        .undo()
        .unwrap();
    assert_eq!(restored.revision, 0);
    assert!(workspace.accessibility_copy_bytes(plan.job_id).is_err());
    assert!(
        workspace
            .begin_accessibility_copy(request(&report))
            .is_err(),
        "undo cannot resurrect source epoch"
    );
    let report = inspect(&mut workspace, &restored);
    let plan = workspace
        .begin_accessibility_copy(request(&report))
        .unwrap();
    workspace.set_draft(info.id, true).unwrap();
    workspace.set_draft(info.id, false).unwrap();
    assert!(workspace.check_accessibility_copy(plan.job_id).is_err());
    let report = inspect(&mut workspace, &restored);
    assert!(workspace.cancel_accessibility_inspection(report.inspection_id));
    assert!(
        workspace
            .check_accessibility_inspection(report.inspection_id)
            .is_err()
    );
}

#[test]
fn tagged_copies_reject_all_structure_staling_mutation_bypasses_but_allow_exact_byte_saves() {
    let original = tagged();
    let mut session = Session::from_bytes(original.clone(), "Tagged.pdf".into(), None, 1).unwrap();
    assert!(
        session
            .info()
            .unwrap()
            .read_only_reason
            .unwrap()
            .contains("tagged copy")
    );
    assert!(
        session
            .apply(Edit::Rotate {
                page: 0,
                degrees: 90
            })
            .is_err()
    );
    assert!(session.fill_form_fields(vec![]).is_err());
    assert!(session.design_form_fields(vec![]).is_err());
    assert!(session.extract_pages(&[0]).is_err());
    assert!(session.import_pages(bytes(fixture()), 0).is_err());
    assert!(session.ensure_signable().is_err());
    assert!(session.signed_copy(b"", "").is_err());
    assert!(
        session
            .protection_copy(&CopyProtection::Unprotect, None)
            .is_err()
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("exact.pdf");
    session.save_as(&path).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), original);
    assert_eq!(session.render_bytes().unwrap(), original);
}

#[test]
fn native_output_is_atomic_separate_no_clobber_and_rechecks_all_tab_source_paths() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.pdf");
    std::fs::write(&source, bytes(fixture())).unwrap();
    let mut workspace = Workspace::default();
    let info = match workspace.request_open_path(source.clone()).unwrap() {
        OpenOutcome::Opened { document, .. } => document,
        _ => panic!(),
    };
    let report = inspect(&mut workspace, &info);
    let plan = workspace
        .begin_accessibility_copy(request(&report))
        .unwrap();
    assert!(
        workspace
            .write_accessibility_copy(plan.job_id, &source)
            .is_err()
    );
    let other_path = directory.path().join("other.pdf");
    std::fs::write(&other_path, bytes(fixture())).unwrap();
    workspace.request_open_path(other_path.clone()).unwrap();
    assert!(
        workspace
            .write_accessibility_copy(plan.job_id, &other_path)
            .is_err()
    );
    let exists = directory.path().join("exists.pdf");
    std::fs::write(&exists, b"sentinel").unwrap();
    assert!(
        workspace
            .write_accessibility_copy(plan.job_id, &exists)
            .is_err()
    );
    assert_eq!(std::fs::read(exists).unwrap(), b"sentinel");
    let output = directory.path().join("reviewed.pdf");
    workspace
        .write_accessibility_copy(plan.job_id, &output)
        .unwrap();
    assert!(workspace.check_accessibility_copy(plan.job_id).is_err());
    assert!(reopened(std::fs::read(output).unwrap()).complete);
    assert_eq!(
        workspace.document(info.id).unwrap().source_path.as_ref(),
        Some(&source)
    );
    assert!(workspace.document(info.id).unwrap().saved_path.is_none());
}

#[test]
fn encrypted_copies_retain_protection_and_copy_or_fill_permission_is_not_structural_editing() {
    for editable in [false, true] {
        let original = Session::from_bytes(bytes(fixture()), "source.pdf".into(), None, 1).unwrap();
        let protection: CopyProtection = serde_json::from_value(serde_json::json!({
            "mode":"protect","openingPassword":"reader","ownerPassword":"owner",
            "permissions":{"print":"high","copy":true,"edit":editable,"annotate":editable,"fillForms":true,"assemble":editable}
        })).unwrap();
        let encrypted = original.protection_copy(&protection, None).unwrap();
        let mut workspace = Workspace::default();
        let challenge = workspace
            .request_open_bytes(encrypted, "protected.pdf".into())
            .unwrap();
        let request_id = match challenge {
            OpenOutcome::PasswordRequired { request_id, .. } => request_id,
            _ => panic!(),
        };
        let opened = workspace.unlock_document(request_id, "reader").unwrap();
        let info = match opened {
            OpenOutcome::Opened { document, .. } => document,
            _ => panic!(),
        };
        let report = inspect(&mut workspace, &info);
        assert_eq!(report.authoring_allowed, editable);
        if !editable {
            assert!(
                workspace
                    .begin_accessibility_copy(request(&report))
                    .is_err()
            );
            continue;
        }
        let plan = workspace
            .begin_accessibility_copy(request(&report))
            .unwrap();
        assert!(plan.password_protected);
        let output = workspace.accessibility_copy_bytes(plan.job_id).unwrap();
        assert!(Session::from_bytes(output.clone(), "copy.pdf".into(), None, 3).is_err());
        let protected = Session::from_bytes_with_password(
            output.clone(),
            "copy.pdf".into(),
            None,
            3,
            Some("reader"),
        )
        .unwrap();
        assert!(protected.info().unwrap().password_protected);
        assert!(!protected.permissions().owner_authenticated);
        let owner =
            Session::from_bytes_with_password(output, "copy.pdf".into(), None, 4, Some("owner"))
                .unwrap();
        assert!(owner.permissions().owner_authenticated);
    }
}

#[test]
fn unsupported_source_features_are_precise_refusals_not_success_shaped_artifacts() {
    for variant in 0..4 {
        let mut pdf = fixture();
        let page = pdf.get_pages()[&1];
        match variant {
            0 => {
                pdf.catalog_mut()
                    .unwrap()
                    .set("AcroForm", dictionary! { "Fields"=>Vec::<Object>::new() });
            }
            1 => {
                pdf.catalog_mut()
                    .unwrap()
                    .set("OCProperties", dictionary! {});
            }
            2 => {
                let annotation = pdf.add_object(dictionary! { "Type"=>"Annot","Subtype"=>"Link","Rect"=>vec![0.into(),0.into(),20.into(),20.into()] });
                pdf.get_dictionary_mut(page)
                    .unwrap()
                    .set("Annots", vec![Object::Reference(annotation)]);
            }
            _ => {
                pdf.catalog_mut().unwrap().set(
                    "OpenAction",
                    dictionary! { "S"=>"JavaScript","JS"=>Object::string_literal("not executed") },
                );
            }
        }
        let report = reopened(bytes(pdf));
        assert_eq!(report.status, "unsupported");
        assert!(!report.authoring_allowed);
        assert!(report.candidates.is_empty() && !report.issues.is_empty());
    }
}
