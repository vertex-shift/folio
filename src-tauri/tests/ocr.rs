use std::{collections::BTreeMap, sync::Arc};

use folio_core::{
    document::{Edit, Session},
    ocr::{Direction, ExistingTextPolicy, Options, PageResult, Word},
    workspace::{OpenOutcome, Workspace},
};
use lopdf::{
    Document, LoadOptions, Object, Stream,
    content::Content,
    dictionary,
    encryption::{
        EncryptionState, EncryptionVersion, Permissions,
        crypt_filters::{Aes128CryptFilter, Aes256CryptFilter, CryptFilter},
    },
};

fn synthetic(pages: usize, text: bool) -> Vec<u8> {
    let mut document = Document::with_version("1.7");
    let parent = document.new_object_id();
    let font = document.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let shared_resources = document.add_object(dictionary! {
        "Font" => dictionary! { "FolioOCR1" => font },
    });
    let content = document.add_object(Stream::new(dictionary! {}, if text {
        b"q 0.1 0.2 0.3 rg 20 50 100 30 re f Q BT /FolioOCR1 12 Tf 0 80 Td (Existing source) Tj ET".to_vec()
    } else {
        b"q 0.1 0.2 0.3 rg 20 50 100 30 re f Q 0.5 0 0 0.5 11 9 cm".to_vec()
    }));
    let ids: Vec<Object> = (0..pages)
        .map(|index| {
            document
                .add_object(dictionary! {
                    "Type" => "Page", "Parent" => parent, "Contents" => content,
                    "Rotate" => ((index % 4) * 90) as i64, "UserUnit" => 2,
                })
                .into()
        })
        .collect();
    document.objects.insert(
        parent,
        dictionary! {
            "Type" => "Pages", "Count" => pages as i64, "Kids" => ids,
            "MediaBox" => vec![(-40).into(), 10.into(), 360.into(), 250.into()],
            "CropBox" => vec![(-20).into(), 30.into(), 320.into(), 230.into()],
            "Resources" => shared_resources,
        }
        .into(),
    );
    let root = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => parent });
    document.trailer.set("Root", root);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

fn options(pages: Vec<usize>) -> Options {
    Options {
        pages,
        dpi: 150,
        languages: vec!["eng".into()],
        layout: "horizontal_print".into(),
        segmentation: Default::default(),
        existing_text: ExistingTextPolicy::Skip,
    }
}

fn words(page: usize) -> PageResult {
    PageResult {
        page,
        words: vec![
            Word {
                text: "FOLIO".into(),
                bounds: [50.0, 70.0, 150.0, 100.0],
                baseline: [50.0, 101.0, 150.0, 101.0],
                line: 0,
                line_bounds: [50.0, 70.0, 310.0, 103.0],
                direction: Direction::Ltr,
                separator: " ".into(),
            },
            Word {
                text: "OFFLINE".into(),
                bounds: [170.0, 70.0, 310.0, 100.0],
                baseline: [170.0, 101.0, 310.0, 101.0],
                line: 0,
                line_bounds: [50.0, 70.0, 310.0, 103.0],
                direction: Direction::Ltr,
                separator: "".into(),
            },
        ],
    }
}

#[test]
fn dedicated_ocr_fonts_preserve_logical_unicode_combining_joiners_and_cjk_after_saved_reopen() {
    let cases = [
        ("fra", "café été", Direction::Ltr),
        ("spa", "niño corazón", Direction::Ltr),
        ("deu", "größere Bücher", Direction::Ltr),
        ("ita", "città perché", Direction::Ltr),
        ("por", "coração ação", Direction::Ltr),
        ("nld", "nieuwe boeken", Direction::Ltr),
        ("ell", "Νέα βιβλία", Direction::Ltr),
        ("rus", "Новая книга", Direction::Ltr),
        ("ara", "هذا كِتاب", Direction::Rtl),
        ("hin", "क्‍षि किताब", Direction::Ltr),
        ("chi_sim", "新的图书故事", Direction::Ltr),
        ("chi_tra", "新的圖書故事", Direction::Ltr),
        ("jpn", "新しい本があります。", Direction::Ltr),
        ("kor", "새로운 책 이야기", Direction::Ltr),
        ("eng", "e\u{301} exact", Direction::Ltr),
    ];
    for (language, phrase, direction) in cases {
        let mut workspace = Workspace::default();
        let source = synthetic(1, false);
        let info = workspace
            .open_bytes(source.clone(), format!("{language}.pdf"))
            .unwrap();
        let mut selected = options(vec![0]);
        selected.languages = vec![language.into()];
        let plan = workspace.begin_ocr(info.id, 0, selected).unwrap();
        assert_eq!(plan.languages, [language]);
        let segments = phrase.split(' ').collect::<Vec<_>>();
        let result = PageResult {
            page: 0,
            words: segments
                .iter()
                .enumerate()
                .map(|(index, text)| {
                    let left = if direction == Direction::Rtl {
                        600.0 - index as f64 * 200.0
                    } else {
                        50.0 + index as f64 * 200.0
                    };
                    Word {
                        text: (*text).into(),
                        bounds: [left, 70.0, left + 160.0, 100.0],
                        baseline: [left, 101.0, left + 160.0, 101.0],
                        line: 0,
                        line_bounds: [50.0, 70.0, 760.0, 105.0],
                        direction,
                        separator: if index + 1 < segments.len() {
                            " ".into()
                        } else {
                            "".into()
                        },
                    }
                })
                .collect(),
        };
        workspace.stage_ocr_page(plan.job_id, result).unwrap();
        let result = workspace.commit_ocr(plan.job_id).unwrap();
        assert_eq!(result.languages, [language]);
        let bytes = workspace.document(info.id).unwrap().bytes();
        let document = Document::load_mem(bytes).unwrap();
        assert_eq!(
            document.extract_text(&[1]).unwrap().trim(),
            phrase,
            "{language}"
        );
        let page = document.get_pages()[&1];
        let resources = document
            .get_dictionary(page)
            .unwrap()
            .get(b"Resources")
            .unwrap()
            .as_dict()
            .unwrap();
        let fonts = resources.get(b"Font").unwrap().as_dict().unwrap();
        let font = fonts
            .iter()
            .find(|(name, _)| name.starts_with(b"FolioOCR1_"))
            .unwrap()
            .1;
        let font = document
            .get_dictionary(font.as_reference().unwrap())
            .unwrap();
        assert!(
            font.get(b"BaseFont")
                .unwrap()
                .as_name()
                .unwrap()
                .starts_with(b"FolioOCRLogicalV1_")
        );
        let raw = Content::decode(&document.get_page_content(page)).unwrap();
        assert!(
            raw.operations
                .iter()
                .any(|operation| operation.operator == "Tr" && operation.operands == [3.into()])
        );
        let actual = raw
            .operations
            .iter()
            .filter(|operation| operation.operator == "BDC")
            .map(|operation| {
                let value = operation.operands[1]
                    .as_dict()
                    .unwrap()
                    .get(b"ActualText")
                    .unwrap()
                    .as_str()
                    .unwrap();
                assert_eq!(&value[..2], &[0xfe, 0xff]);
                String::from_utf16(
                    &value[2..]
                        .chunks_exact(2)
                        .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
                        .collect::<Vec<_>>(),
                )
                .unwrap()
            })
            .collect::<String>();
        assert_eq!(actual, phrase);
        let reopened =
            Session::from_bytes(bytes.to_vec(), format!("{language}-saved.pdf"), None, 99).unwrap();
        assert_eq!(
            Document::load_mem(reopened.bytes())
                .unwrap()
                .extract_text(&[1])
                .unwrap()
                .trim(),
            phrase
        );
        workspace.current_mut(info.id, 1).unwrap().undo().unwrap();
        assert_eq!(workspace.document(info.id).unwrap().bytes(), source);
    }
}

#[test]
fn catalog_layout_glyph_and_structure_failures_are_atomic_and_cannot_resurrect_ocr_jobs() {
    let mut workspace = Workspace::default();
    let source = synthetic(1, false);
    let info = workspace
        .open_bytes(source.clone(), "scan.pdf".into())
        .unwrap();
    for ids in [
        vec![],
        vec!["eng", "eng"],
        vec!["eng", "fra", "deu", "spa"],
        vec!["jpn_vert"],
        vec!["osd"],
        vec!["../../eng"],
    ] {
        let mut selected = options(vec![0]);
        selected.languages = ids.iter().map(|id| (*id).into()).collect();
        assert!(workspace.begin_ocr(info.id, 0, selected).is_err());
    }

    let mut selected = options(vec![0]);
    selected.layout = "vertical".into();
    assert!(workspace.begin_ocr(info.id, 0, selected).is_err());
    for invalid_text in ["😀", "किताब", "هذا", "\u{200d}", "\u{202e}Folio"] {
        let plan = workspace.begin_ocr(info.id, 0, options(vec![0])).unwrap();
        let mut result = words(0);
        result.words[0].text = invalid_text.into();
        assert!(workspace.stage_ocr_page(plan.job_id, result).is_err());
        assert_eq!(workspace.document(info.id).unwrap().bytes(), source);
        workspace.cancel_ocr(plan.job_id);
    }
    for marker in ["StructTreeRoot", "MarkInfo", "FolioTaggedCopy"] {
        let mut tagged = Document::load_mem(&source).unwrap();
        tagged
            .catalog_mut()
            .unwrap()
            .set(marker, dictionary! {"Marked"=>true});
        let mut bytes = Vec::new();
        tagged.save_to(&mut bytes).unwrap();
        let info = workspace
            .open_bytes(bytes.clone(), format!("{marker}.pdf"))
            .unwrap();
        assert!(workspace.begin_ocr(info.id, 0, options(vec![0])).is_err());
        assert_eq!(workspace.document(info.id).unwrap().bytes(), bytes);
    }
}

#[test]
fn horizontal_japanese_prolonged_sound_marks_keep_the_actual_line_baseline() {
    let mut workspace = Workspace::default();
    let info = workspace
        .open_bytes(synthetic(1, false), "Japanese punctuation.pdf".into())
        .unwrap();
    let mut selected = options(vec![0]);
    selected.languages = vec!["jpn".into()];
    let plan = workspace.begin_ocr(info.id, 0, selected).unwrap();
    let result = PageResult {
        page: 0,
        words: vec![Word {
            text: "ー".into(),
            bounds: [100.0, 75.0, 180.0, 80.0],
            baseline: [100.0, 105.0, 180.0, 105.0],
            line: 0,
            line_bounds: [50.0, 70.0, 500.0, 110.0],
            direction: Direction::Ltr,
            separator: "".into(),
        }],
    };
    workspace.stage_ocr_page(plan.job_id, result).unwrap();
    workspace.commit_ocr(plan.job_id).unwrap();
    assert_eq!(
        Document::load_mem(workspace.document(info.id).unwrap().bytes())
            .unwrap()
            .extract_text(&[1])
            .unwrap()
            .trim(),
        "ー"
    );
}

#[test]
fn fractional_user_unit_mirrors_renderer_multiplication_order_and_exact_pixel_ceil() {
    let mut source = Document::load_mem(&synthetic(1, false)).unwrap();
    let page = source.get_dictionary_mut(source.get_pages()[&1]).unwrap();
    page.set(
        "MediaBox",
        vec![(-40).into(), 56.into(), 120.into(), 360.into()],
    );
    page.set(
        "CropBox",
        vec![(-32).into(), 64.into(), 112.into(), 352.into()],
    );
    page.set("Rotate", 270);
    page.set("UserUnit", 1.25);
    let mut bytes = Vec::new();
    source.save_to(&mut bytes).unwrap();
    let mut workspace = Workspace::default();
    let info = workspace
        .open_bytes(bytes, "fractional crop.pdf".into())
        .unwrap();
    for (dpi, width, height) in [
        (300, 1501, 751),
        (150, 751, 376),
        (96, 480, 240),
        (72, 360, 180),
    ] {
        let mut settings = options(vec![0]);
        settings.dpi = dpi;
        let plan = workspace.begin_ocr(info.id, 0, settings).unwrap();
        assert_eq!(plan.pages[0].pixel_width, width);
        assert_eq!(plan.pages[0].pixel_height, height);
        assert_eq!(plan.pages[0].source_bounds, [-32.0, 64.0, 112.0, 352.0]);
        assert_eq!(plan.pages[0].rotation, 270);
        assert_eq!(plan.pages[0].user_unit, 1.25);
        workspace.cancel_ocr(plan.job_id);
    }
}

#[test]
fn oversized_scans_support_explicit_lower_dpi_without_resizing_original_pages() {
    let mut document = Document::load_mem(&synthetic(1, false)).unwrap();
    let page_id = document.get_pages()[&1];
    let original_content = document.get_page_content(page_id);
    let page = document.get_dictionary_mut(page_id).unwrap();
    page.set(
        "MediaBox",
        vec![0.into(), 0.into(), 2400.into(), 3600.into()],
    );
    page.set(
        "CropBox",
        vec![0.into(), 0.into(), 2400.into(), 3600.into()],
    );
    page.set("UserUnit", 1);
    let mut source = Vec::new();
    document.save_to(&mut source).unwrap();
    let mut workspace = Workspace::default();
    let info = workspace
        .open_bytes(source.clone(), "Synthetic oversized scan.pdf".into())
        .unwrap();
    for dpi in [0, 71, 75, 97, 301] {
        let mut settings = options(vec![0]);
        settings.dpi = dpi;
        assert!(workspace.begin_ocr(info.id, 0, settings).is_err());
    }
    for dpi in [150, 300] {
        let mut settings = options(vec![0]);
        settings.dpi = dpi;
        let error = workspace
            .begin_ocr(info.id, 0, settings)
            .unwrap_err()
            .to_string();
        assert!(error.contains("24-megapixel") && error.contains("96, 72 DPI"));
        assert_eq!(workspace.document(info.id).unwrap().bytes(), source);
    }
    for (dpi, dimensions) in [(72, (2400, 3600)), (96, (3200, 4800))] {
        let revision = workspace.document(info.id).unwrap().revision();
        let mut settings = options(vec![0]);
        settings.dpi = dpi;
        let plan = workspace.begin_ocr(info.id, revision, settings).unwrap();
        assert_eq!(plan.dpi, dpi);
        assert_eq!(
            (plan.pages[0].pixel_width, plan.pages[0].pixel_height),
            dimensions
        );
        workspace.stage_ocr_page(plan.job_id, words(0)).unwrap();
        let outcome = workspace.commit_ocr(plan.job_id).unwrap();
        assert_eq!(outcome.word_count, 2);
        let saved = Document::load_mem(workspace.document(info.id).unwrap().bytes()).unwrap();
        let saved_page_id = saved.get_pages()[&1];
        let saved_page = saved.get_dictionary(saved_page_id).unwrap();
        assert_eq!(
            saved_page.get(b"MediaBox").unwrap(),
            document
                .get_dictionary(page_id)
                .unwrap()
                .get(b"MediaBox")
                .unwrap()
        );
        assert!(
            saved
                .get_page_content(saved_page_id)
                .ends_with(&original_content)
        );
        assert!(saved.extract_text(&[1]).unwrap().contains("FOLIO"));
        workspace
            .current_mut(info.id, outcome.document.revision)
            .unwrap()
            .undo()
            .unwrap();
        assert_eq!(workspace.document(info.id).unwrap().bytes(), source);
    }
}

#[test]
fn staged_ocr_is_atomic_persistent_undoable_and_preserves_original_streams_and_shared_resources() {
    let source = synthetic(4, false);
    let original = Document::load_mem(&source).unwrap();
    let mut workspace = Workspace::default();
    let info = workspace
        .open_bytes(source.clone(), "Scan.pdf".into())
        .unwrap();
    let other = workspace.open_demo().unwrap();
    let other_bytes = workspace.document(other.id).unwrap().bytes().to_vec();
    let plan = workspace
        .begin_ocr(info.id, 0, options(vec![3, 1, 0, 2]))
        .unwrap();
    assert_eq!(
        plan.pages.iter().map(|page| page.page).collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
    for page in &plan.pages {
        assert_eq!(page.source_bounds, [-20.0, 30.0, 320.0, 230.0]);
        assert_eq!(page.user_unit, 2.0);
        workspace
            .stage_ocr_page(plan.job_id, words(page.page))
            .unwrap();
        assert_eq!(workspace.document(info.id).unwrap().bytes(), source);
    }
    let outcome = workspace.commit_ocr(plan.job_id).unwrap();
    assert_eq!(outcome.word_count, 8);
    assert_eq!(outcome.document.revision, 1);
    assert!(outcome.document.dirty && outcome.document.can_undo);
    assert_eq!(workspace.document(other.id).unwrap().bytes(), other_bytes);
    assert_eq!(workspace.current().unwrap().id, other.id);
    let saved = Document::load_mem(workspace.document(info.id).unwrap().bytes()).unwrap();
    let extracted = saved.extract_text(&[1, 2, 3, 4]).unwrap();
    assert_eq!(extracted.matches("FOLIO OFFLINE").count(), 4, "{extracted}");
    for (id, object) in &original.objects {
        if object.as_stream().is_ok_and(|stream| {
            stream.dict.get(b"Type").and_then(Object::as_name).ok() != Some(b"XRef")
        }) || object.as_dict().is_ok_and(|dict| dict.has(b"Font"))
        {
            assert_eq!(
                saved.objects.get(id),
                Some(object),
                "source stream/resource changed"
            );
        }
    }
    for id in saved.get_pages().values() {
        let page = saved.get_dictionary(*id).unwrap();
        let contents = page.get(b"Contents").unwrap().as_array().unwrap();
        assert_eq!(contents.len(), 2);
        let hidden = saved
            .get_object(contents[0].as_reference().unwrap())
            .unwrap()
            .as_stream()
            .unwrap();
        let operations = Content::decode(&hidden.get_plain_content().unwrap())
            .unwrap()
            .operations;
        assert_eq!(operations.first().unwrap().operator, "q");
        assert_eq!(operations.last().unwrap().operator, "Q");
        assert_eq!(
            operations
                .iter()
                .find(|op| op.operator == "Tr")
                .unwrap()
                .operands,
            vec![3.into()]
        );
        assert!(
            operations
                .iter()
                .find(|op| op.operator == "Tf")
                .unwrap()
                .operands[0]
                .as_name()
                .unwrap()
                .starts_with(b"FolioOCR")
        );
    }
    let undo = workspace.current_mut(info.id, 1).unwrap().undo().unwrap();
    assert_eq!(undo.revision, 0);
    assert_eq!(workspace.document(info.id).unwrap().bytes(), source);
    workspace.current_mut(info.id, 0).unwrap().redo().unwrap();
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("Recognized.pdf");
    workspace
        .current_mut(info.id, 1)
        .unwrap()
        .save_as(&path)
        .unwrap();
    let reopened = Session::from_path(path, 90).unwrap();
    assert!(!reopened.info().unwrap().dirty);
    assert_eq!(
        Document::load_mem(reopened.bytes())
            .unwrap()
            .extract_text(&[1])
            .unwrap()
            .trim(),
        "FOLIO OFFLINE"
    );
}

#[test]
fn incomplete_canceled_stale_closed_and_pending_draft_jobs_never_change_the_pdf() {
    let source = synthetic(2, false);
    let mut workspace = Workspace::default();
    let info = workspace
        .open_bytes(source.clone(), "Scan.pdf".into())
        .unwrap();
    let plan = workspace
        .begin_ocr(info.id, 0, options(vec![0, 1]))
        .unwrap();
    workspace.stage_ocr_page(plan.job_id, words(0)).unwrap();
    assert!(workspace.commit_ocr(plan.job_id).is_err());
    assert_eq!(workspace.document(info.id).unwrap().bytes(), source);
    let plan = workspace.begin_ocr(info.id, 0, options(vec![0])).unwrap();
    workspace.stage_ocr_page(plan.job_id, words(0)).unwrap();
    assert!(workspace.cancel_ocr(plan.job_id));
    assert!(workspace.commit_ocr(plan.job_id).is_err());
    let plan = workspace.begin_ocr(info.id, 0, options(vec![0])).unwrap();
    workspace
        .current_mut(info.id, 0)
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    workspace.current_mut(info.id, 1).unwrap().undo().unwrap();
    assert_eq!(workspace.document(info.id).unwrap().bytes(), source);
    assert!(
        workspace.stage_ocr_page(plan.job_id, words(0)).is_err(),
        "undo must not resurrect a stale job"
    );
    let plan = workspace.begin_ocr(info.id, 0, options(vec![0])).unwrap();
    workspace.set_draft(info.id, true).unwrap();
    assert!(workspace.stage_ocr_page(plan.job_id, words(0)).is_err());
    assert!(workspace.begin_ocr(info.id, 0, options(vec![0])).is_err());
    workspace.set_draft(info.id, false).unwrap();
    let plan = workspace.begin_ocr(info.id, 0, options(vec![0])).unwrap();
    workspace.close_document(info.id, 0, true).unwrap();
    assert!(workspace.commit_ocr(plan.job_id).is_err());
}

#[test]
fn skip_force_and_no_text_outcomes_are_explicit_and_do_not_create_empty_history() {
    let source = synthetic(1, true);
    let mut workspace = Workspace::default();
    let info = workspace
        .open_bytes(source.clone(), "Text.pdf".into())
        .unwrap();
    let plan = workspace.begin_ocr(info.id, 0, options(vec![0])).unwrap();
    assert!(plan.pages[0].skipped && plan.pages[0].existing_text);
    assert!(workspace.stage_ocr_page(plan.job_id, words(0)).is_err());
    let outcome = workspace.commit_ocr(plan.job_id).unwrap();
    assert_eq!(outcome.skipped_pages, vec![0]);
    assert_eq!(outcome.word_count, 0);
    assert_eq!(outcome.document.revision, 0);
    assert!(!outcome.document.can_undo);
    assert_eq!(workspace.document(info.id).unwrap().bytes(), source);
    let mut force = options(vec![0]);
    force.existing_text = ExistingTextPolicy::Force;
    let plan = workspace.begin_ocr(info.id, 0, force).unwrap();
    assert!(!plan.pages[0].skipped);
    workspace.stage_ocr_page(plan.job_id, words(0)).unwrap();
    workspace.commit_ocr(plan.job_id).unwrap();
    let output = Document::load_mem(workspace.document(info.id).unwrap().bytes()).unwrap();
    let text = output.extract_text(&[1]).unwrap();
    assert!(text.contains("Existing source") && text.contains("FOLIO OFFLINE"));
    let info = workspace
        .open_bytes(synthetic(1, false), "Blank.pdf".into())
        .unwrap();
    let plan = workspace.begin_ocr(info.id, 0, options(vec![0])).unwrap();
    workspace
        .stage_ocr_page(
            plan.job_id,
            PageResult {
                page: 0,
                words: vec![],
            },
        )
        .unwrap();
    let outcome = workspace.commit_ocr(plan.job_id).unwrap();
    assert_eq!(outcome.no_text_pages, vec![0]);
    assert_eq!(outcome.document.revision, 0);
    assert!(!outcome.document.can_undo);
}

#[test]
fn nested_form_text_is_detected_without_requiring_a_supported_source_font() {
    let mut source = Document::load_mem(&synthetic(1, false)).unwrap();
    let page = source.get_pages()[&1];
    let form = source.add_object(Stream::new(dictionary! {
        "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
        "Resources" => dictionary! {},
    }, b"BT /UnknownFont 10 Tf (Nested source) Tj ET".to_vec()));
    let content = source.add_object(Stream::new(dictionary! {}, b"q /Nested Do Q".to_vec()));
    let page = source.get_dictionary_mut(page).unwrap();
    page.set("Contents", content);
    page.set(
        "Resources",
        dictionary! { "XObject" => dictionary! { "Nested" => form } },
    );
    let mut bytes = Vec::new();
    source.save_to(&mut bytes).unwrap();
    let mut workspace = Workspace::default();
    let info = workspace.open_bytes(bytes, "Nested.pdf".into()).unwrap();
    assert!(
        workspace
            .begin_ocr(info.id, 0, options(vec![0]))
            .unwrap()
            .pages[0]
            .skipped
    );
}

#[test]
fn invalid_language_geometry_order_fonts_and_limits_fail_without_a_partial_mutation() {
    let mut workspace = Workspace::default();
    let info = workspace
        .open_bytes(synthetic(2, false), "Scan.pdf".into())
        .unwrap();
    let mut unsupported = options(vec![0]);
    unsupported.languages = vec!["../../eng".into()];
    assert!(workspace.begin_ocr(info.id, 0, unsupported).is_err());
    assert!(
        workspace
            .begin_ocr(info.id, 0, options(vec![0, 0]))
            .is_err()
    );
    assert!(workspace.begin_ocr(info.id, 0, options(vec![2])).is_err());
    assert!(
        workspace
            .begin_ocr(info.id, 0, options(vec![0; 51]))
            .is_err()
    );
    let plan = workspace
        .begin_ocr(info.id, 0, options(vec![0, 1]))
        .unwrap();
    assert!(workspace.begin_ocr(info.id, 0, options(vec![0])).is_err());
    assert!(workspace.stage_ocr_page(plan.job_id, words(1)).is_err());
    workspace.stage_ocr_page(plan.job_id, words(0)).unwrap();
    let mut invalid = words(1);
    invalid.words[0].text = "中文".into();
    assert!(workspace.stage_ocr_page(plan.job_id, invalid).is_err());
    let mut invalid = words(1);
    invalid.words[0].bounds[0] = -1.0;
    assert!(workspace.stage_ocr_page(plan.job_id, invalid).is_err());
    let mut invalid = words(1);
    invalid.words[0].baseline[3] += 500.0;
    assert!(workspace.stage_ocr_page(plan.job_id, invalid).is_err());
    assert_eq!(workspace.document(info.id).unwrap().revision(), 0);
    assert!(workspace.commit_ocr(plan.job_id).is_err());
    assert_eq!(workspace.document(info.id).unwrap().revision(), 0);
}

fn encrypted(version: u8, permissions: Permissions, signed: bool) -> Vec<u8> {
    let mut source = Document::load_mem(&synthetic(1, false)).unwrap();
    source.trailer.set(
        "ID",
        vec![
            Object::string_literal("synthetic-ocr"),
            Object::string_literal("synthetic-ocr"),
        ],
    );
    if signed {
        let marker = source.add_object(dictionary! {
            "Type" => "Sig", "ByteRange" => vec![0.into(), 1.into(), 2.into(), 3.into()],
            "Contents" => Object::string_literal("synthetic marker, not a real signature"),
        });
        source
            .catalog_mut()
            .unwrap()
            .set("FolioTestSignature", marker);
    }
    let method = match version {
        1 => EncryptionVersion::V1 {
            document: &source,
            owner_password: "owner",
            user_password: "user",
            permissions,
        },
        2 => EncryptionVersion::V2 {
            document: &source,
            owner_password: "owner",
            user_password: "user",
            permissions,
            key_length: 128,
        },
        4 => {
            let filter: Arc<dyn CryptFilter> = Arc::new(Aes128CryptFilter);
            EncryptionVersion::V4 {
                document: &source,
                owner_password: "owner",
                user_password: "user",
                permissions,
                encrypt_metadata: true,
                crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
                stream_filter: b"StdCF".to_vec(),
                string_filter: b"StdCF".to_vec(),
            }
        }
        5 => {
            let filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
            EncryptionVersion::V5 {
                owner_password: "owner",
                user_password: "user",
                permissions,
                encrypt_metadata: true,
                file_encryption_key: &[41; 32],
                crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
                stream_filter: b"StdCF".to_vec(),
                string_filter: b"StdCF".to_vec(),
            }
        }
        _ => panic!("unsupported synthetic encryption"),
    };
    source
        .encrypt(&EncryptionState::try_from(method).unwrap())
        .unwrap();
    let mut bytes = Vec::new();
    source.save_to(&mut bytes).unwrap();
    bytes
}

fn unlock(workspace: &mut Workspace, bytes: Vec<u8>, password: &str) -> u64 {
    let OpenOutcome::PasswordRequired { request_id, .. } = workspace
        .request_open_bytes(bytes, "Protected scan.pdf".into())
        .unwrap()
    else {
        panic!("expected password challenge")
    };
    let OpenOutcome::Opened { document, .. } =
        workspace.unlock_document(request_id, password).unwrap()
    else {
        panic!("expected unlocked PDF")
    };
    document.id
}

#[test]
fn rc4_and_aes_ocr_saves_reopen_with_original_passwords_permissions_and_encryption() {
    for version in [1, 2, 4, 5] {
        let source = encrypted(version, Permissions::all(), false);
        let mut workspace = Workspace::default();
        let id = unlock(&mut workspace, source.clone(), "user");
        let plan = workspace.begin_ocr(id, 0, options(vec![0])).unwrap();
        workspace.stage_ocr_page(plan.job_id, words(0)).unwrap();
        assert!(
            workspace
                .commit_ocr(plan.job_id)
                .unwrap()
                .document
                .password_protected
        );
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("OCR protected.pdf");
        workspace
            .current_mut(id, 1)
            .unwrap()
            .save_as(&path)
            .unwrap();
        let saved = std::fs::read(&path).unwrap();
        assert!(Document::load_mem(&saved).unwrap().is_encrypted());
        assert!(!String::from_utf8_lossy(&saved).contains("FOLIO OFFLINE"));
        for password in ["user", "owner"] {
            let reopened = Session::from_bytes_with_password(
                saved.clone(),
                "OCR.pdf".into(),
                None,
                9,
                Some(password),
            )
            .unwrap();
            let text = Document::load_mem(&reopened.render_bytes().unwrap())
                .unwrap()
                .extract_text(&[1])
                .unwrap();
            assert!(text.contains("FOLIO OFFLINE"));
            let original =
                Document::load_mem_with_options(&source, LoadOptions::with_password(password))
                    .unwrap();
            let changed =
                Document::load_mem_with_options(&saved, LoadOptions::with_password(password))
                    .unwrap();
            assert_eq!(
                original.trailer.get(b"ID").unwrap(),
                changed.trailer.get(b"ID").unwrap()
            );
            assert_eq!(
                original.encryption_state.as_ref().unwrap().version(),
                changed.encryption_state.as_ref().unwrap().version()
            );
            assert_eq!(
                original
                    .encryption_state
                    .as_ref()
                    .unwrap()
                    .file_encryption_key(),
                changed
                    .encryption_state
                    .as_ref()
                    .unwrap()
                    .file_encryption_key()
            );
            assert_eq!(
                original.encryption_state.as_ref().unwrap().permissions(),
                changed.encryption_state.as_ref().unwrap().permissions()
            );
        }
        workspace.current_mut(id, 1).unwrap().undo().unwrap();
        assert_eq!(workspace.document(id).unwrap().bytes(), source);
        workspace.current_mut(id, 0).unwrap().redo().unwrap();
        assert!(
            Document::load_mem(workspace.document(id).unwrap().bytes())
                .unwrap()
                .is_encrypted()
        );
    }
}

#[test]
fn signed_and_restricted_documents_cannot_be_ocrd_but_owner_access_can() {
    for signed in [false, true] {
        let bytes = encrypted(5, Permissions::PRINTABLE, signed);
        let mut workspace = Workspace::default();
        let id = unlock(&mut workspace, bytes.clone(), "user");
        assert!(workspace.begin_ocr(id, 0, options(vec![0])).is_err());
        let id = unlock(&mut workspace, bytes, "owner");
        let plan = workspace.begin_ocr(id, 0, options(vec![0]));
        assert_eq!(plan.is_ok(), !signed);
        if let Ok(plan) = plan {
            workspace.stage_ocr_page(plan.job_id, words(0)).unwrap();
            workspace.commit_ocr(plan.job_id).unwrap();
            let protected = workspace.document(id).unwrap().bytes().to_vec();
            let user = Session::from_bytes_with_password(
                protected.clone(),
                "OCR.pdf".into(),
                None,
                91,
                Some("user"),
            )
            .unwrap();
            assert!(user.info().unwrap().read_only_reason.is_some());
            assert!(!user.info().unwrap().copy_allowed);
            let output =
                Document::load_mem_with_options(&protected, LoadOptions::with_password("owner"))
                    .unwrap();
            assert_eq!(
                output
                    .encryption_state
                    .as_ref()
                    .unwrap()
                    .permissions()
                    .intersection(Permissions::all()),
                Permissions::PRINTABLE
            );
        }
    }
}

#[test]
fn sequential_job_ownership_and_render_geometry_are_bounded_before_recognition() {
    let mut workspace = Workspace::default();
    let mut jobs = Vec::new();
    let info = workspace.create_document(100.0, 100.0).unwrap();
    jobs.push(
        workspace
            .begin_ocr(info.id, 0, options(vec![0]))
            .unwrap()
            .job_id,
    );
    let next = workspace.create_document(100.0, 100.0).unwrap();
    assert!(workspace.begin_ocr(next.id, 0, options(vec![0])).is_err());
    workspace.cancel_ocr(jobs[0]);
    let plan = workspace.begin_ocr(next.id, 0, options(vec![0])).unwrap();
    workspace.cancel_ocr(plan.job_id);

    let mut source = Document::load_mem(&synthetic(1, false)).unwrap();
    let id = source.get_pages()[&1];
    source
        .get_dictionary_mut(id)
        .unwrap()
        .set("UserUnit", 75_000);
    let mut bytes = Vec::new();
    source.save_to(&mut bytes).unwrap();
    let mut workspace = Workspace::default();
    let info = workspace
        .open_bytes(bytes, "Oversized scan.pdf".into())
        .unwrap();
    let error = workspace
        .begin_ocr(info.id, 0, options(vec![0]))
        .unwrap_err()
        .to_string();
    assert!(error.contains("24-megapixel") && error.contains("16,384"));
    assert_eq!(workspace.document(info.id).unwrap().revision(), 0);

    let mut source = Document::load_mem(&synthetic(1, false)).unwrap();
    let page = source.get_dictionary_mut(source.get_pages()[&1]).unwrap();
    page.set(
        "MediaBox",
        vec![10_000_000.into(), 10.into(), 10_000_400.into(), 250.into()],
    );
    page.set(
        "CropBox",
        vec![10_000_020.into(), 30.into(), 10_000_360.into(), 230.into()],
    );
    let mut bytes = Vec::new();
    source.save_to(&mut bytes).unwrap();
    let info = workspace
        .open_bytes(bytes.clone(), "Imprecise origin.pdf".into())
        .unwrap();
    let plan = workspace.begin_ocr(info.id, 0, options(vec![0])).unwrap();
    workspace.stage_ocr_page(plan.job_id, words(0)).unwrap();
    let error = workspace.commit_ocr(plan.job_id).unwrap_err().to_string();
    assert!(error.contains("precision"));
    assert_eq!(workspace.document(info.id).unwrap().bytes(), bytes);
}
