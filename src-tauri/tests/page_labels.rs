use folio_core::{
    document::{Edit, Session},
    page_labels::{Range, Style},
    protection::{CopyPermissions, CopyProtection, PrintPermission},
};
use lopdf::{Document, Object, Stream, dictionary, text_string};

fn source(count: usize, labels: Option<Object>) -> Vec<u8> {
    let mut document = Document::with_version("1.7");
    let parent = document.new_object_id();
    let font = document.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let pages = (0..count).map(|index| {
        let content = document.add_object(Stream::new(dictionary! {},
            format!("BT /F1 18 Tf 40 500 Td (Synthetic page {}) Tj ET", index + 1).into_bytes()));
        Object::Reference(document.add_object(dictionary! {
            "Type" => "Page", "Parent" => parent, "MediaBox" => vec![0.into(), 0.into(), 420.into(), 600.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } }, "Contents" => content,
        }))
    }).collect::<Vec<_>>();
    document.objects.insert(
        parent,
        dictionary! { "Type" => "Pages", "Count" => count as i64, "Kids" => pages }.into(),
    );
    let mut catalog = dictionary! { "Type" => "Catalog", "Pages" => parent };
    if let Some(labels) = labels {
        catalog.set("PageLabels", labels);
    }
    let root = document.add_object(catalog);
    document.trailer.set("Root", root);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

fn basic_labels() -> Object {
    dictionary! { "Nums" => vec![
        0.into(), dictionary! { "S" => "r", "St" => 1 }.into(),
        2.into(), dictionary! { "S" => "D", "St" => 1 }.into(),
    ] }
    .into()
}

fn session(bytes: Vec<u8>) -> Session {
    Session::from_bytes(bytes, "Synthetic labels.pdf".into(), None, 1).unwrap()
}

fn labels(session: &Session) -> Vec<String> {
    session.info().unwrap().page_labels.unwrap().labels.unwrap()
}

fn range(style: Style, prefix: &str, start: u32) -> Range {
    Range {
        page: 0,
        style,
        prefix: prefix.into(),
        start,
    }
}

#[test]
fn all_label_styles_and_unicode_prefixes_are_persistent_undoable_metadata_not_page_stamps() {
    for (style, prefix, start, expected) in [
        (Style::Decimal, "Sec-", 7, vec!["Sec-7", "Sec-8", "Sec-9"]),
        (Style::RomanLower, "", 9, vec!["ix", "x", "xi"]),
        (Style::RomanUpper, "", 39, vec!["XXXIX", "XL", "XLI"]),
        (Style::LettersUpper, "", 26, vec!["Z", "AA", "BB"]),
        (Style::LettersLower, "§-", 26, vec!["§-z", "§-aa", "§-bb"]),
        (
            Style::Prefix,
            "Appendix",
            1,
            vec!["Appendix", "Appendix", "Appendix"],
        ),
    ] {
        let original = source(3, None);
        let before = Document::load_mem(&original).unwrap();
        let mut current = session(original.clone());
        current
            .apply(Edit::PageLabels {
                ranges: vec![range(style, prefix, start)],
            })
            .unwrap();
        assert_eq!(labels(&current), expected);
        let saved = Document::load_mem(current.bytes()).unwrap();
        for (number, id) in saved.get_pages() {
            assert_eq!(
                saved.get_page_content(id),
                before.get_page_content(before.get_pages()[&number])
            );
            assert_eq!(
                saved.get_dictionary(id).unwrap().get(b"MediaBox").unwrap(),
                before
                    .get_dictionary(before.get_pages()[&number])
                    .unwrap()
                    .get(b"MediaBox")
                    .unwrap()
            );
        }
        let reopened = session(current.bytes().to_vec());
        assert_eq!(labels(&reopened), expected);
        assert!(current.info().unwrap().dirty && current.info().unwrap().can_undo);
        current.undo().unwrap();
        assert_eq!(current.bytes(), original);
        current.redo().unwrap();
        assert_eq!(labels(&current), expected);
    }
}

#[test]
fn moving_deleting_and_extracting_pages_keep_their_original_labels() {
    let original = source(4, Some(basic_labels()));
    let mut current = session(original.clone());
    current.apply(Edit::Move { from: 2, to: 0 }).unwrap();
    assert_eq!(labels(&current), ["1", "i", "ii", "2"]);
    let extracted = session(current.extract_pages(&[3, 0]).unwrap());
    assert_eq!(labels(&extracted), ["2", "1"]);
    assert_eq!(labels(&current), ["1", "i", "ii", "2"]);
    current.apply(Edit::Delete { page: 1 }).unwrap();
    assert_eq!(labels(&current), ["1", "ii", "2"]);
    current.undo().unwrap();
    assert_eq!(labels(&current), ["1", "i", "ii", "2"]);
    current.undo().unwrap();
    assert_eq!(current.bytes(), original);
}

#[test]
fn duplicate_blank_and_imported_pages_preserve_existing_labels_without_guessing_unique_names() {
    let mut current = session(source(4, Some(basic_labels())));
    current.apply(Edit::Duplicate { page: 0 }).unwrap();
    assert_eq!(labels(&current), ["i", "i", "ii", "1", "2"]);
    current
        .apply(Edit::BlankPage {
            at: 1,
            width: 420.0,
            height: 600.0,
        })
        .unwrap();
    assert_eq!(labels(&current), ["i", "2", "i", "ii", "1", "2"]);
    let donor = source(
        2,
        Some(
            dictionary! { "Nums" => vec![
                0.into(), dictionary! { "S" => "D", "P" => text_string("A-"), "St" => 4 }.into(),
            ] }
            .into(),
        ),
    );
    let mut target = session(source(4, Some(basic_labels())));
    target.import_pages(donor, 2).unwrap();
    assert_eq!(labels(&target), ["i", "ii", "A-4", "A-5", "1", "2"]);
    let mut plain = session(source(2, None));
    plain
        .import_pages(source(4, Some(basic_labels())), 1)
        .unwrap();
    assert_eq!(labels(&plain), ["1", "i", "ii", "1", "2", "6"]);
}

#[test]
fn reset_removes_only_label_metadata_and_preserves_unlabelled_page_operation_behavior() {
    let ordinary = session(source(
        4,
        Some(
            dictionary! { "Nums" => vec![0.into(), dictionary! { "S" => "D", "St" => 1 }.into()] }
                .into(),
        ),
    ));
    assert_eq!(labels(&ordinary), ["1", "2", "3", "4"]);
    let mut current = session(source(4, Some(basic_labels())));
    current.apply(Edit::PageLabels { ranges: vec![] }).unwrap();
    assert!(current.info().unwrap().page_labels.is_none());
    current.apply(Edit::Move { from: 2, to: 0 }).unwrap();
    assert!(current.info().unwrap().page_labels.is_none());
    assert!(
        session(current.extract_pages(&[3, 0]).unwrap())
            .info()
            .unwrap()
            .page_labels
            .is_none()
    );
}

#[test]
fn nested_number_trees_and_indirect_unicode_prefixes_are_read_without_mutating_source_bytes() {
    let mut document = Document::load_mem(&source(4, None)).unwrap();
    let prefix = document.add_object(text_string("Pré-"));
    let first = document.add_object(dictionary! {
        "Nums" => vec![0.into(), dictionary! { "Type" => "PageLabel", "S" => "r", "P" => prefix }.into()],
        "Limits" => vec![0.into(), 0.into()],
    });
    let second = document.add_object(dictionary! {
        "Nums" => vec![2.into(), dictionary! { "S" => "D", "St" => 5 }.into()],
        "Limits" => vec![2.into(), 2.into()],
    });
    let root = document.add_object(dictionary! { "Kids" => vec![first.into(), second.into()], "Limits" => vec![0.into(), 2.into()] });
    document.catalog_mut().unwrap().set("PageLabels", root);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    let current = session(bytes.clone());
    assert_eq!(labels(&current), ["Pré-i", "Pré-ii", "5", "6"]);
    assert_eq!(current.bytes(), bytes);
}

#[test]
fn invalid_label_trees_are_explicit_read_issues_and_page_operations_refuse_to_discard_them() {
    for labels in [
        dictionary! { "Nums" => vec![1.into(), dictionary! { "S" => "D" }.into()] },
        dictionary! { "Nums" => vec![0.into()] },
        dictionary! { "Nums" => vec![0.into(), dictionary! { "S" => "Unknown" }.into()] },
        dictionary! { "Nums" => vec![0.into(), dictionary! { "S" => "D", "St" => 0 }.into()] },
        dictionary! { "Nums" => vec![0.into(), dictionary! { "P" => text_string("\n") }.into()] },
        dictionary! { "Nums" => vec![0.into(), dictionary! { "P" => text_string(&"x".repeat(513)) }.into()] },
        dictionary! { "Nums" => vec![0.into(), dictionary! { "S" => "D" }.into()], "Limits" => vec![0.into(), 3.into()] },
        dictionary! { "Nums" => vec![0.into(), dictionary! {}.into()], "Kids" => Vec::<Object>::new() },
    ] {
        let bytes = source(4, Some(labels.into()));
        let mut current = session(bytes.clone());
        let info = current.info().unwrap().page_labels.unwrap();
        assert!(info.labels.is_none() && info.issue.is_some());
        assert!(current.apply(Edit::Move { from: 2, to: 0 }).is_err());
        assert!(current.extract_pages(&[1]).is_err());
        assert_eq!(current.bytes(), bytes);
        assert_eq!(current.revision(), 0);
        current.apply(Edit::PageLabels { ranges: vec![] }).unwrap();
        assert!(current.info().unwrap().page_labels.is_none());
        current.undo().unwrap();
        assert_eq!(current.bytes(), bytes);
    }
}

#[test]
fn cyclic_label_trees_and_excessive_numbering_fail_without_panics_or_partial_history() {
    let mut document = Document::load_mem(&source(3, None)).unwrap();
    let root = document.new_object_id();
    document
        .objects
        .insert(root, dictionary! { "Kids" => vec![root.into()] }.into());
    document.catalog_mut().unwrap().set("PageLabels", root);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    let current = session(bytes);
    assert!(
        current
            .info()
            .unwrap()
            .page_labels
            .unwrap()
            .issue
            .unwrap()
            .contains("cycle")
    );
    let mut current = session(source(3, None));
    let original = current.bytes().to_vec();
    for ranges in [
        vec![range(Style::Decimal, "", u32::MAX)],
        vec![range(Style::LettersUpper, "", u32::MAX)],
        vec![range(Style::RomanUpper, "", u32::MAX)],
        vec![Range {
            page: 1,
            ..range(Style::Decimal, "", 1)
        }],
        vec![range(Style::Decimal, "", 1); 257],
    ] {
        assert!(current.apply(Edit::PageLabels { ranges }).is_err());
        assert_eq!(current.bytes(), original);
        assert_eq!(current.revision(), 0);
        assert!(!current.info().unwrap().can_undo);
    }
}

#[test]
fn protected_label_updates_keep_passwords_permissions_and_exact_undo_bytes() {
    let original = session(source(4, Some(basic_labels())));
    let protected = original
        .protection_copy(
            &CopyProtection::Protect {
                opening_password: "reader".into(),
                owner_password: "owner".into(),
                permissions: CopyPermissions {
                    print: PrintPermission::High,
                    copy: true,
                    edit: false,
                    annotate: false,
                    fill_forms: true,
                    assemble: false,
                },
            },
            None,
        )
        .unwrap();
    let mut reader = Session::from_bytes_with_password(
        protected.clone(),
        "Restricted.pdf".into(),
        None,
        1,
        Some("reader"),
    )
    .unwrap();
    assert!(
        reader
            .apply(Edit::PageLabels {
                ranges: vec![range(Style::Decimal, "B-", 1)]
            })
            .is_err()
    );
    assert_eq!(reader.bytes(), protected);
    let mut owner = Session::from_bytes_with_password(
        protected.clone(),
        "Protected.pdf".into(),
        None,
        2,
        Some("owner"),
    )
    .unwrap();
    owner
        .apply(Edit::PageLabels {
            ranges: vec![range(Style::Decimal, "B-", 1)],
        })
        .unwrap();
    let reopened = Session::from_bytes_with_password(
        owner.bytes().to_vec(),
        "Saved.pdf".into(),
        None,
        3,
        Some("reader"),
    )
    .unwrap();
    assert_eq!(labels(&reopened), ["B-1", "B-2", "B-3", "B-4"]);
    assert!(reopened.info().unwrap().password_protected);
    assert!(!reopened.info().unwrap().permissions.modify_allowed);
    owner.undo().unwrap();
    assert_eq!(owner.bytes(), protected);
}

#[test]
fn occupied_signature_sources_keep_label_metadata_read_only() {
    let mut document = Document::load_mem(&source(4, Some(basic_labels()))).unwrap();
    let signature = document.add_object(dictionary! {
        "Type" => "Sig",
        "ByteRange" => vec![0.into(), 1.into(), 2.into(), 3.into()],
        "Contents" => Object::String(b"synthetic occupied signature".to_vec(), lopdf::StringFormat::Hexadecimal),
    });
    let field = document.add_object(dictionary! { "FT" => "Sig", "V" => signature });
    document
        .catalog_mut()
        .unwrap()
        .set("AcroForm", dictionary! { "Fields" => vec![field.into()] });
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    let mut current = session(bytes.clone());
    assert!(current.info().unwrap().read_only_reason.is_some());
    assert_eq!(labels(&current), ["i", "ii", "1", "2"]);
    assert!(
        current
            .apply(Edit::PageLabels {
                ranges: vec![range(Style::Decimal, "", 1)]
            })
            .is_err()
    );
    assert_eq!(current.bytes(), bytes);
}
