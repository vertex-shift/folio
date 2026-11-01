use folio_core::{
    accessibility::{Assignment, CandidateKind, CopyRequest, Role},
    archive::SourceRef,
    document::Session,
    ocr::{Direction, ExistingTextPolicy, Options, PageResult, Word},
    protection::CopyProtection,
    workspace::{OpenOutcome, Workspace},
};
use lopdf::{Document, Object, Stream, content::Content, dictionary};

fn bytes(mut document: Document) -> Vec<u8> {
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

fn scan() -> Vec<u8> {
    let mut pdf = Document::with_version("1.7");
    let pages = pdf.new_object_id();
    let image = pdf.add_object(Stream::new(dictionary! {
        "Type"=>"XObject","Subtype"=>"Image","Width"=>1,"Height"=>1,"ColorSpace"=>"DeviceRGB","BitsPerComponent"=>8,
    }, vec![210, 40, 65]));
    let contents = pdf.add_object(Stream::new(
        dictionary! {},
        b"q 300 0 0 600 24 24 cm /Image Do Q".to_vec(),
    ));
    let page = pdf.add_object(dictionary! {
        "Type"=>"Page","Parent"=>pages,"MediaBox"=>vec![0.into(),0.into(),400.into(),700.into()],
        "Contents"=>contents,"Resources"=>dictionary! {"XObject"=>dictionary! {"Image"=>image}},
    });
    pdf.objects.insert(
        pages,
        dictionary! {"Type"=>"Pages","Kids"=>vec![Object::Reference(page)],"Count"=>1}.into(),
    );
    let root = pdf.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages});
    pdf.trailer.set("Root", root);
    bytes(pdf)
}

fn recognize(
    language: &str,
    words: &[&str],
    direction: Direction,
    protected: bool,
) -> (Workspace, u64, String) {
    let mut source = scan();
    if protected {
        let session = Session::from_bytes(source, "scan.pdf".into(), None, 1).unwrap();
        let protection: CopyProtection = serde_json::from_value(serde_json::json!({
            "mode":"protect","openingPassword":"reader","ownerPassword":"owner",
            "permissions":{"print":"high","copy":true,"edit":true,"annotate":true,"fillForms":true,"assemble":true},
        })).unwrap();
        source = session.protection_copy(&protection, None).unwrap();
    }
    let mut workspace = Workspace::default();
    let info = if protected {
        let challenge = workspace
            .request_open_bytes(source, "protected.pdf".into())
            .unwrap();
        let request_id = match challenge {
            OpenOutcome::PasswordRequired { request_id, .. } => request_id,
            _ => panic!(),
        };
        match workspace.unlock_document(request_id, "reader").unwrap() {
            OpenOutcome::Opened { document, .. } => document,
            _ => panic!(),
        }
    } else {
        workspace.open_bytes(source, "scan.pdf".into()).unwrap()
    };
    let plan = workspace
        .begin_ocr(
            info.id,
            info.revision,
            Options {
                pages: vec![0],
                dpi: 150,
                languages: vec![language.into()],
                layout: "horizontal_print".into(),
                segmentation: Default::default(),
                existing_text: ExistingTextPolicy::Skip,
            },
        )
        .unwrap();
    let mut logical = String::new();
    let words = words
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let top = 40.0 + index as f64 * 90.0;
            logical.push_str(text);
            let separator = if index + 1 < words.len() { "\n" } else { "" };
            logical.push_str(separator);
            Word {
                text: (*text).into(),
                bounds: [40.0, top, 450.0, top + 35.0],
                baseline: [40.0, top + 36.0, 450.0, top + 36.0],
                line: index as u32,
                line_bounds: [40.0, top, 450.0, top + 35.0],
                direction,
                separator: separator.into(),
            }
        })
        .collect();
    workspace
        .stage_ocr_page(plan.job_id, PageResult { page: 0, words })
        .unwrap();
    workspace.commit_ocr(plan.job_id).unwrap();
    (workspace, info.id, logical)
}

fn recipe(workspace: &mut Workspace, id: u64) -> CopyRequest {
    let revision = workspace.document(id).unwrap().revision();
    let report = workspace
        .inspect_accessibility(SourceRef {
            document_id: id,
            revision,
        })
        .unwrap();
    assert!(report.authoring_allowed, "{:?}", report.issues);
    assert_eq!(report.candidates.len(), 2);
    assert_eq!(report.candidates[0].kind, CandidateKind::Text);
    assert!(
        report.candidates[0]
            .description
            .contains("accuracy is not verified")
    );
    CopyRequest {
        source: report.source,
        source_epoch: report.source_epoch,
        title: "Human reviewed OCR structure".into(),
        language: "fr-FR".into(),
        reviewed: true,
        entries: report
            .candidates
            .iter()
            .map(|candidate| Assignment {
                candidate_id: candidate.id.clone(),
                role: if candidate.kind == CandidateKind::Text {
                    Role::P
                } else {
                    Role::Figure
                },
                alt: (candidate.kind != CandidateKind::Text)
                    .then(|| "Original scanned page, described by the reviewing human.".into()),
                language: None,
            })
            .collect(),
    }
}

fn source_pdf(workspace: &Workspace, id: u64) -> Document {
    Document::load_mem(&workspace.document(id).unwrap().render_bytes().unwrap()).unwrap()
}

fn ocr_stream(pdf: &Document) -> lopdf::ObjectId {
    *pdf.objects
        .iter()
        .find(|(_, object)| {
            object
                .as_stream()
                .is_ok_and(|stream| stream.dict.has(b"FolioOCR"))
        })
        .unwrap()
        .0
}

#[test]
fn canonical_ocr_text_gets_reviewed_page_semantics_and_retains_original_streams_fonts_and_source_state()
 {
    for (language, text, direction) in [
        ("eng", vec!["Original", "e\u{301}"], Direction::Ltr),
        ("fra", vec!["café", "été"], Direction::Ltr),
        ("ara", vec!["هذا", "كِتاب"], Direction::Rtl),
        ("hin", vec!["क्‍षि", "किताब"], Direction::Ltr),
        ("jpn", vec!["新しい本があります。"], Direction::Ltr),
    ] {
        let (mut workspace, id, logical) = recognize(language, &text, direction, false);
        let source = workspace.document(id).unwrap().bytes().to_vec();
        let state = serde_json::to_value(workspace.document(id).unwrap().info().unwrap()).unwrap();
        let before = source_pdf(&workspace, id);
        let stream = ocr_stream(&before);
        let request = recipe(&mut workspace, id);
        let plan = workspace.begin_accessibility_copy(request).unwrap();
        let output = workspace.accessibility_copy_bytes(plan.job_id).unwrap();
        let after = Document::load_mem(&output).unwrap();
        assert_eq!(
            before.get_object(stream).unwrap(),
            after.get_object(stream).unwrap()
        );
        let page = after.get_dictionary(after.get_pages()[&1]).unwrap();
        assert!(
            page.get(b"Contents")
                .unwrap()
                .as_array()
                .unwrap()
                .contains(&Object::Reference(stream)),
            "original OCR stream is still referenced"
        );
        for (id, object) in &before.objects {
            if object.as_stream().is_ok_and(|stream| {
                !stream.dict.get(b"Type").is_ok_and(|kind| {
                    kind.as_name()
                        .is_ok_and(|kind| matches!(kind, b"XRef" | b"ObjStm"))
                })
            }) || object.as_dict().is_ok_and(|dictionary| {
                dictionary.get(b"Type").is_ok_and(|kind| {
                    kind.as_name()
                        .is_ok_and(|kind| matches!(kind, b"Font" | b"FontDescriptor"))
                })
            }) {
                assert_eq!(
                    after.get_object(*id).unwrap(),
                    object,
                    "original font/image/content object {id:?} changed"
                );
            }
        }
        assert_eq!(workspace.document(id).unwrap().bytes(), source);
        assert_eq!(
            serde_json::to_value(workspace.document(id).unwrap().info().unwrap()).unwrap(),
            state
        );
        let opened = workspace.open_bytes(output, "tagged.pdf".into()).unwrap();
        let report = workspace
            .inspect_accessibility(SourceRef {
                document_id: opened.id,
                revision: 0,
            })
            .unwrap();
        assert!(report.complete, "{:?}", report.issues);
        let nodes = &report.structure[0].children;
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].role, "P");
        assert_eq!(nodes[0].text.as_deref(), Some(logical.as_str()));
        assert_eq!(nodes[0].bindings[0].mcid, 0);
        assert!(nodes[0].bindings[0].stream.is_none());
        assert_eq!(nodes[1].role, "Figure");
        assert_eq!(nodes[1].bindings[0].mcid, 1);
    }
}

#[test]
fn protected_ocr_tagged_copies_keep_the_original_passwords_and_permissions() {
    let (mut workspace, id, logical) = recognize("fra", &["café", "été"], Direction::Ltr, true);
    let permissions = serde_json::to_value(workspace.document(id).unwrap().permissions()).unwrap();
    assert!(
        !workspace
            .document(id)
            .unwrap()
            .permissions()
            .owner_authenticated
    );
    let request = recipe(&mut workspace, id);
    let plan = workspace.begin_accessibility_copy(request).unwrap();
    let output = workspace.accessibility_copy_bytes(plan.job_id).unwrap();
    assert!(Session::from_bytes(output.clone(), "protected-tagged.pdf".into(), None, 50).is_err());
    let reader = Session::from_bytes_with_password(
        output.clone(),
        "protected-tagged.pdf".into(),
        None,
        50,
        Some("reader"),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(reader.permissions()).unwrap(),
        permissions
    );
    let owner = Session::from_bytes_with_password(
        output,
        "protected-tagged.pdf".into(),
        None,
        51,
        Some("owner"),
    )
    .unwrap();
    assert!(owner.permissions().owner_authenticated);
    let info = workspace
        .open_bytes(
            reader.render_bytes().unwrap(),
            "testing-decrypted-copy.pdf".into(),
        )
        .unwrap();
    let report = workspace
        .inspect_accessibility(SourceRef {
            document_id: info.id,
            revision: 0,
        })
        .unwrap();
    assert!(report.complete);
    assert_eq!(
        report.structure[0].children[0].text.as_deref(),
        Some(logical.as_str())
    );
}

#[test]
fn marker_names_and_forged_ocr_glyph_unicode_actualtext_or_visible_operations_never_authorize_semantics()
 {
    let (workspace, id, _) = recognize("fra", &["café", "été"], Direction::Ltr, false);
    let source = source_pdf(&workspace, id);
    let stream_id = ocr_stream(&source);
    for variant in 0..10 {
        let mut pdf = source.clone();
        if variant <= 4 || variant == 9 {
            let stream = pdf.get_object(stream_id).unwrap().as_stream().unwrap();
            let mut content = Content::decode(&stream.get_plain_content().unwrap()).unwrap();
            match variant {
                0 => {
                    content
                        .operations
                        .iter_mut()
                        .find(|operation| operation.operator == "Tr")
                        .unwrap()
                        .operands = vec![0.into()]
                }
                1 => {
                    let operation = content
                        .operations
                        .iter_mut()
                        .find(|operation| operation.operator == "Tj")
                        .unwrap();
                    operation.operands = vec![Object::String(
                        vec![0xff, 0xff],
                        lopdf::StringFormat::Hexadecimal,
                    )];
                }
                2 => {
                    let span = content
                        .operations
                        .iter_mut()
                        .find(|operation| operation.operator == "BDC")
                        .unwrap();
                    span.operands[1]
                        .as_dict_mut()
                        .unwrap()
                        .set("ActualText", lopdf::text_string("forged"));
                }
                3 => {
                    let span = content
                        .operations
                        .iter_mut()
                        .find(|operation| operation.operator == "BDC")
                        .unwrap();
                    span.operands[1].as_dict_mut().unwrap().set("MCID", 0);
                }
                4 => content.operations.insert(
                    2,
                    lopdf::content::Operation::new("Do", vec![Object::Name(b"Image".to_vec())]),
                ),
                _ => {
                    content.operations = Content::decode(b"q BT /Fake 1 Tf 3 Tr (pretend) Tj ET Q")
                        .unwrap()
                        .operations;
                }
            }
            let stream = pdf
                .get_object_mut(stream_id)
                .unwrap()
                .as_stream_mut()
                .unwrap();
            stream.set_plain_content(content.encode().unwrap());
            stream.compress().unwrap();
        } else {
            let font = *pdf
                .objects
                .iter()
                .find(|(_, object)| {
                    object.as_dict().is_ok_and(|dictionary| {
                        dictionary
                            .get(b"Subtype")
                            .is_ok_and(|value| value.as_name().is_ok_and(|value| value == b"Type0"))
                    })
                })
                .unwrap()
                .0;
            let dictionary = pdf.get_dictionary(font).unwrap();
            let descendant = dictionary
                .get(b"DescendantFonts")
                .unwrap()
                .as_array()
                .unwrap()[0]
                .as_reference()
                .unwrap();
            let descriptor = pdf
                .get_dictionary(descendant)
                .unwrap()
                .get(b"FontDescriptor")
                .unwrap()
                .as_reference()
                .unwrap();
            match variant {
                5 => {
                    let map = pdf
                        .get_dictionary(descendant)
                        .unwrap()
                        .get(b"CIDToGIDMap")
                        .unwrap()
                        .as_reference()
                        .unwrap();
                    let stream = pdf.get_object_mut(map).unwrap().as_stream_mut().unwrap();
                    stream.content[2..4].copy_from_slice(&[0, 0]);
                }
                6 => {
                    let values = pdf
                        .get_dictionary_mut(descendant)
                        .unwrap()
                        .get_mut(b"W")
                        .unwrap()
                        .as_array_mut()
                        .unwrap();
                    values[1].as_array_mut().unwrap()[0] = Object::Real(4000.0);
                }
                7 => {
                    let file = pdf
                        .get_dictionary(descriptor)
                        .unwrap()
                        .get(b"FontFile2")
                        .unwrap()
                        .as_reference()
                        .unwrap();
                    let stream = pdf.get_object_mut(file).unwrap().as_stream_mut().unwrap();
                    let mut plain = stream.get_plain_content().unwrap();
                    plain[100] ^= 1;
                    stream.set_plain_content(plain);
                    stream.compress().unwrap();
                }
                _ => {
                    pdf.get_dictionary_mut(descriptor)
                        .unwrap()
                        .set("Ascent", 10000);
                }
            }
        }
        let mut checked = Workspace::default();
        let opened = checked.open_bytes(bytes(pdf), "forged.pdf".into()).unwrap();
        let report = checked
            .inspect_accessibility(SourceRef {
                document_id: opened.id,
                revision: 0,
            })
            .unwrap();
        assert!(
            !report.authoring_allowed && report.candidates.is_empty(),
            "variant {variant}"
        );
        assert_eq!(report.status, "unsupported", "variant {variant}");
        assert!(!report.issues.is_empty());
    }
}

#[test]
fn actual_saved_ocr_acceptance_inputs_compose_when_explicitly_supplied() {
    let Some(directory) = std::env::var_os("FOLIO_OCR_SEMANTIC_FIXTURES") else {
        return;
    };
    for (filename, language) in [
        ("Actual-eng-OCR.pdf", "en-US"),
        ("Actual-fra-OCR.pdf", "fr-FR"),
    ] {
        let source = std::fs::read(std::path::Path::new(&directory).join(filename)).unwrap();
        let mut workspace = Workspace::default();
        let info = workspace
            .open_bytes(source.clone(), filename.into())
            .unwrap();
        let report = workspace
            .inspect_accessibility(SourceRef {
                document_id: info.id,
                revision: info.revision,
            })
            .unwrap();
        assert!(report.authoring_allowed, "{filename}: {:?}", report.issues);
        let logical = report
            .candidates
            .iter()
            .find(|candidate| candidate.description.contains("canonical invisible OCR"))
            .unwrap()
            .text
            .clone()
            .unwrap();
        let request = CopyRequest {
            source: report.source,
            source_epoch: report.source_epoch,
            title: "Human-reviewed actual OCR composition".into(),
            language: language.into(),
            reviewed: true,
            entries: report
                .candidates
                .iter()
                .map(|candidate| Assignment {
                    candidate_id: candidate.id.clone(),
                    role: if candidate.kind == CandidateKind::Text {
                        Role::P
                    } else {
                        Role::Figure
                    },
                    alt: (candidate.kind != CandidateKind::Text)
                        .then(|| "Original printed source page; human review is required.".into()),
                    language: None,
                })
                .collect(),
        };
        let plan = workspace.begin_accessibility_copy(request).unwrap();
        let output = workspace.accessibility_copy_bytes(plan.job_id).unwrap();
        assert_eq!(workspace.document(info.id).unwrap().bytes(), source);
        let opened = workspace.open_bytes(output, "reviewed.pdf".into()).unwrap();
        let checked = workspace
            .inspect_accessibility(SourceRef {
                document_id: opened.id,
                revision: 0,
            })
            .unwrap();
        assert!(checked.complete, "{filename}: {:?}", checked.issues);
        assert!(
            checked.structure[0]
                .children
                .iter()
                .any(|node| node.text.as_deref() == Some(logical.as_str()))
        );
    }
}
