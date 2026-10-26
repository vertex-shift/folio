use folio_core::{
    attachments::{MAX_ATTACHMENT_BYTES, suggested_basename},
    document::{Edit, Session},
    metadata::Fields,
    protection::{CopyPermissions, CopyProtection, PrintPermission},
    workspace::Workspace,
};
use lopdf::{Document, LoadOptions, Object, Stream, decode_text_string, dictionary, text_string};

fn bytes(document: &mut Document) -> Vec<u8> {
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

fn reopen(session: &Session) -> Session {
    Session::from_bytes(session.bytes().to_vec(), "Copy.pdf".into(), None, 2).unwrap()
}

fn sample() -> Document {
    Document::load_mem(Session::blank(200.0, 300.0, 1).unwrap().bytes()).unwrap()
}

fn fields() -> Fields {
    Fields {
        title: "Résumé & <東京> 📄".into(),
        author: "Zoë / Ελληνικά".into(),
        subject: "A Unicode subject".into(),
        keywords: "local; 文書; review".into(),
    }
}

fn protected() -> CopyProtection {
    CopyProtection::Protect {
        opening_password: "reader".into(),
        owner_password: "owner".into(),
        permissions: CopyPermissions {
            print: PrintPermission::Low,
            copy: false,
            edit: false,
            annotate: false,
            fill_forms: true,
            assemble: false,
        },
    }
}

fn open_protected(bytes: Vec<u8>, password: &str) -> Session {
    Session::from_bytes_with_password(bytes, "Protected.pdf".into(), None, 1, Some(password))
        .unwrap()
}

#[test]
fn metadata_is_unicode_undoable_and_persistent_without_losing_unrelated_info() {
    let mut document = sample();
    let info = document.add_object(dictionary! {
        "Title" => text_string("Original title"), "Producer" => text_string("Original synthetic producer"),
        "CreationDate" => Object::string_literal("D:20260101000000Z"),
    });
    document.trailer.set("Info", info);
    let mut session =
        Session::from_bytes(bytes(&mut document), "Original.pdf".into(), None, 1).unwrap();
    let before = session.bytes().to_vec();
    session.apply(Edit::Metadata { fields: fields() }).unwrap();
    assert_eq!(session.metadata().unwrap().fields, fields());
    assert_eq!(reopen(&session).metadata().unwrap().fields, fields());
    let output = Document::load_mem(session.bytes()).unwrap();
    let info = output
        .dereference(output.trailer.get(b"Info").unwrap())
        .unwrap()
        .1
        .as_dict()
        .unwrap();
    assert_eq!(
        decode_text_string(info.get(b"Producer").unwrap()).unwrap(),
        "Original synthetic producer"
    );
    session.undo().unwrap();
    assert_eq!(session.bytes(), before);
    session.redo().unwrap();
    assert_eq!(session.metadata().unwrap().fields, fields());
    session
        .apply(Edit::Metadata {
            fields: Fields::default(),
        })
        .unwrap();
    assert_eq!(
        reopen(&session).metadata().unwrap().fields,
        Fields::default()
    );
}

#[test]
fn metadata_updates_existing_xmp_properties_attributes_and_keywords_without_destroying_other_rdf() {
    let mut document = sample();
    let xmp = br#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/"><r:RDF xmlns:r="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<r:Description xmlns:d="http://purl.org/dc/elements/1.1/" xmlns:p="http://ns.adobe.com/pdf/1.3/"
 xmlns:t="urn:original-synthetic-test" p:Keywords="old-keywords" t:Keep="keep-attribute">
<d:title><r:Alt><r:li xml:lang="fr">ancien titre</r:li><r:li xml:lang="x-default">old-title</r:li></r:Alt></d:title>
<d:creator><r:Seq><r:li>old-author</r:li></r:Seq></d:creator>
<d:description>old-subject</d:description><d:subject><r:Bag><r:li>old-keyword-bag</r:li></r:Bag></d:subject>
<t:Keep><t:Nested>keep-content &amp; text</t:Nested></t:Keep></r:Description></r:RDF></x:xmpmeta>"#;
    let metadata = document.add_object(Stream::new(
        dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
        xmp.to_vec(),
    ));
    document.catalog_mut().unwrap().set("Metadata", metadata);
    let mut session = Session::from_bytes(bytes(&mut document), "XMP.pdf".into(), None, 1).unwrap();
    assert_eq!(session.metadata().unwrap().fields.title, "old-title");
    assert!(session.metadata().unwrap().editable);
    session.apply(Edit::Metadata { fields: fields() }).unwrap();
    let reopened = reopen(&session);
    assert_eq!(reopened.metadata().unwrap().fields, fields());
    let output = Document::load_mem(session.bytes()).unwrap();
    let stream = output
        .dereference(output.catalog().unwrap().get(b"Metadata").unwrap())
        .unwrap()
        .1
        .as_stream()
        .unwrap();
    let xml = String::from_utf8(stream.get_plain_content_with_limit(1024 * 1024).unwrap()).unwrap();
    for old in [
        "old-title",
        "ancien titre",
        "old-author",
        "old-subject",
        "old-keywords",
        "old-keyword-bag",
    ] {
        assert!(!xml.contains(old), "stale XMP property: {old}");
    }
    assert!(xml.contains("keep-content &amp; text"));
    assert!(xml.contains("keep-attribute"));
    assert!(xml.contains("Résumé &amp; &lt;東京&gt; 📄"));
    session.undo().unwrap();
    assert_eq!(session.metadata().unwrap().fields.title, "old-title");
}

#[test]
fn unsupported_xmp_is_rejected_transactionally_instead_of_leaving_contradictory_metadata() {
    for xmp in [
        "<broken",
        "<!DOCTYPE x [<!ENTITY custom 'synthetic'>]><x>&custom;</x>",
        "<?xml version='1.0' encoding='UTF-16'?><x/>",
        "<x/>",
    ] {
        let mut document = sample();
        let metadata = document.add_object(Stream::new(dictionary! {}, xmp.as_bytes().to_vec()));
        document.catalog_mut().unwrap().set("Metadata", metadata);
        let mut session =
            Session::from_bytes(bytes(&mut document), "Bad XMP.pdf".into(), None, 1).unwrap();
        let original = session.bytes().to_vec();
        assert!(!session.metadata().unwrap().editable);
        assert!(session.apply(Edit::Metadata { fields: fields() }).is_err());
        assert_eq!(session.bytes(), original);
        assert_eq!(session.revision(), 0);
    }
}

#[test]
fn bookmark_authoring_preserves_destinations_actions_and_hierarchy_across_history_and_page_changes()
{
    let mut session = Session::demo(1).unwrap();
    let original = session.bookmarks().unwrap();
    assert!(!original.is_empty());
    let previous = session.bytes().to_vec();
    let at = original
        .iter()
        .filter(|entry| entry.parent.is_none())
        .count();
    session
        .apply(Edit::BookmarkCreate {
            parent: None,
            at,
            title: "資料 📄".into(),
            page: 1,
        })
        .unwrap();
    let created = session
        .bookmarks()
        .unwrap()
        .into_iter()
        .find(|entry| entry.title == "資料 📄")
        .unwrap();
    assert_eq!(created.page, Some(1));
    session
        .apply(Edit::BookmarkCreate {
            parent: Some(created.id.clone()),
            at: 0,
            title: "Child".into(),
            page: 2,
        })
        .unwrap();
    session
        .apply(Edit::BookmarkRename {
            id: created.id.clone(),
            title: "Renamed 東京".into(),
        })
        .unwrap();
    session
        .apply(Edit::BookmarkMove {
            id: created.id.clone(),
            parent: None,
            at: 0,
        })
        .unwrap();
    let entries = reopen(&session).bookmarks().unwrap();
    assert_eq!(entries[0].title, "Renamed 東京");
    assert_eq!(entries[1].title, "Child");
    assert_eq!(entries[1].parent, Some(created.id.clone()));
    for before in &original {
        let after = entries.iter().find(|entry| entry.id == before.id).unwrap();
        assert_eq!(after.title, before.title);
        assert_eq!(after.page, before.page);
        assert_eq!(after.named_destination, before.named_destination);
        assert_eq!(after.url, before.url);
    }
    session.apply(Edit::Move { from: 1, to: 0 }).unwrap();
    assert_eq!(session.bookmarks().unwrap()[0].page, Some(0));
    let child = entries[1].id.clone();
    let bytes_before_invalid = session.bytes().to_vec();
    assert!(
        session
            .apply(Edit::BookmarkMove {
                id: created.id.clone(),
                parent: Some(child),
                at: 0
            })
            .is_err()
    );
    assert_eq!(session.bytes(), bytes_before_invalid);
    session
        .apply(Edit::BookmarkDelete {
            id: created.id.clone(),
        })
        .unwrap();
    assert_eq!(session.bookmarks().unwrap().len(), original.len());
    session.undo().unwrap();
    assert!(
        session
            .bookmarks()
            .unwrap()
            .iter()
            .any(|entry| entry.title == "Child")
    );
    for _ in 0..5 {
        session.undo().unwrap();
    }
    assert_eq!(session.bytes(), previous);
}

#[test]
fn bookmark_renaming_and_moves_leave_unrecognized_actions_and_appearance_intact() {
    let mut document = sample();
    let root = document.add_object(dictionary! { "Type" => "Outlines" });
    let action =
        dictionary! { "S" => "GoToR", "F" => text_string("another-synthetic.pdf"), "D" => 0 };
    let bookmark = document.add_object(dictionary! {
        "Title" => text_string("External action"), "Parent" => root, "A" => action.clone(),
        "C" => vec![1.into(), 0.into(), 0.into()], "F" => 2,
        "SyntheticExtension" => text_string("retain-me"),
    });
    document
        .get_dictionary_mut(root)
        .unwrap()
        .set("First", bookmark);
    document
        .get_dictionary_mut(root)
        .unwrap()
        .set("Last", bookmark);
    document.catalog_mut().unwrap().set("Outlines", root);
    let mut session =
        Session::from_bytes(bytes(&mut document), "Actions.pdf".into(), None, 1).unwrap();
    let entry = session.bookmarks().unwrap().remove(0);
    assert!(entry.page.is_none());
    session
        .apply(Edit::BookmarkRename {
            id: entry.id,
            title: "New name".into(),
        })
        .unwrap();
    let output = Document::load_mem(session.bytes()).unwrap();
    let node = output.get_dictionary(bookmark).unwrap();
    assert_eq!(node.get(b"A").unwrap().as_dict().unwrap(), &action);
    assert_eq!(node.get(b"F").unwrap().as_i64().unwrap(), 2);
    assert_eq!(
        decode_text_string(node.get(b"SyntheticExtension").unwrap()).unwrap(),
        "retain-me"
    );
}

#[test]
fn cyclic_outline_and_attachment_trees_fail_without_mutating_the_pdf() {
    let mut document = sample();
    let root = document.add_object(dictionary! { "Type" => "Outlines" });
    let item =
        document.add_object(dictionary! { "Title" => text_string("Cycle"), "Parent" => root });
    document.get_dictionary_mut(item).unwrap().set("Next", item);
    document
        .get_dictionary_mut(root)
        .unwrap()
        .set("First", item);
    document.get_dictionary_mut(root).unwrap().set("Last", item);
    document.catalog_mut().unwrap().set("Outlines", root);
    let names = document.add_object(dictionary! {});
    document
        .get_dictionary_mut(names)
        .unwrap()
        .set("Kids", vec![names.into()]);
    document
        .catalog_mut()
        .unwrap()
        .set("Names", dictionary! { "EmbeddedFiles" => names });
    let mut session =
        Session::from_bytes(bytes(&mut document), "Cycles.pdf".into(), None, 1).unwrap();
    let before = session.bytes().to_vec();
    assert!(session.bookmarks().is_err());
    assert!(session.attachments().is_err());
    assert!(
        session
            .apply(Edit::BookmarkDelete {
                id: format!("{}:{}", item.0, item.1)
            })
            .is_err()
    );
    assert!(
        session
            .add_attachment("safe.txt", b"synthetic".to_vec())
            .is_err()
    );
    assert_eq!(session.bytes(), before);
}

#[test]
fn attachment_add_export_remove_and_history_round_trip_unicode_and_preserve_other_name_trees() {
    let mut document = sample();
    document.catalog_mut().unwrap().set("Names", dictionary! {
        "Dests" => dictionary! { "Names" => vec![text_string("keep-destination"), Object::Array(vec![Object::Integer(0), Object::Name(b"Fit".to_vec())])] },
        "SyntheticExtension" => text_string("keep-name-tree-property"),
    });
    let mut session =
        Session::from_bytes(bytes(&mut document), "Attachments.pdf".into(), None, 1).unwrap();
    let original = session.bytes().to_vec();
    let payload = b"Original synthetic attachment. Never execute.";
    session
        .add_attachment("資料 café.txt", payload.to_vec())
        .unwrap();
    let attachments = session.attachments().unwrap();
    assert_eq!(attachments[0].name, "資料 café.txt");
    assert_eq!(attachments[0].byte_length, payload.len());
    assert_eq!(
        session.attachment_bytes(&attachments[0].id).unwrap().1,
        payload
    );
    let mut reopened = reopen(&session);
    assert_eq!(reopened.attachments().unwrap()[0].name, "資料 café.txt");
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("Saved.pdf");
    reopened.save_as(&path).unwrap();
    assert_eq!(
        Session::from_path(path, 3).unwrap().attachments().unwrap()[0].name,
        "資料 café.txt"
    );
    session
        .apply(Edit::AttachmentRemove {
            id: attachments[0].id.clone(),
        })
        .unwrap();
    assert!(session.attachments().unwrap().is_empty());
    session.undo().unwrap();
    assert_eq!(
        session.attachment_bytes(&attachments[0].id).unwrap().1,
        payload
    );
    session.undo().unwrap();
    assert_eq!(session.bytes(), original);
    session.redo().unwrap();
    let output = Document::load_mem(session.bytes()).unwrap();
    let names = output
        .dereference(output.catalog().unwrap().get(b"Names").unwrap())
        .unwrap()
        .1
        .as_dict()
        .unwrap();
    assert!(names.has(b"Dests"));
    assert!(names.has(b"SyntheticExtension"));
}

#[test]
fn attachment_payload_limits_apply_before_and_during_decompression() {
    let mut session = Session::demo(1).unwrap();
    let before = session.bytes().to_vec();
    assert!(
        session
            .add_attachment("too-big.bin", vec![0; MAX_ATTACHMENT_BYTES + 1])
            .is_err()
    );
    assert_eq!(session.bytes(), before);
    let mut document = sample();
    let mut stream = Stream::new(
        dictionary! { "Type" => "EmbeddedFile" },
        vec![b'x'; MAX_ATTACHMENT_BYTES + 1],
    );
    stream.compress().unwrap();
    assert!(stream.content.len() < 64 * 1024);
    let embedded = document.add_object(stream);
    let specification = document.add_object(dictionary! {
        "Type" => "Filespec", "UF" => text_string("bomb.bin"),
        "EF" => dictionary! { "UF" => embedded },
    });
    document.catalog_mut().unwrap().set("Names", dictionary! {
        "EmbeddedFiles" => dictionary! { "Names" => vec![text_string("bomb.bin"), specification.into()] },
    });
    let session = Session::from_bytes(bytes(&mut document), "Bomb.pdf".into(), None, 1).unwrap();
    assert!(session.attachments().is_err());
    assert!(session.attachment_bytes("0").is_err());
}

#[test]
fn attachment_export_names_cannot_suggest_traversal_or_windows_devices() {
    assert_eq!(suggested_basename("../../資料.txt"), "資料.txt");
    assert_eq!(suggested_basename("C:\\private\\safe.txt"), "safe.txt");
    assert_eq!(suggested_basename("NUL.txt"), "_NUL.txt");
    assert_eq!(suggested_basename(".."), "attachment.bin");
    assert_eq!(suggested_basename("bad:\0?.txt"), "bad___.txt");
    assert_eq!(suggested_basename(&"📄".repeat(120)).len(), 240);
}

#[test]
fn attachment_entry_limits_and_malformed_filters_are_enforced_exactly() {
    for count in [128, 129] {
        let mut document = sample();
        let payload = document.add_object(Stream::new(dictionary! {}, b"synthetic".to_vec()));
        let specification = document.add_object(dictionary! {
            "UF" => text_string("payload.txt"), "EF" => dictionary! { "UF" => payload },
        });
        let pairs: Vec<Object> = (0..count)
            .flat_map(|index| {
                [
                    text_string(&format!("entry-{index:03}")),
                    specification.into(),
                ]
            })
            .collect();
        document.catalog_mut().unwrap().set(
            "Names",
            dictionary! {
                "EmbeddedFiles" => dictionary! { "Names" => pairs },
            },
        );
        let session =
            Session::from_bytes(bytes(&mut document), "Count.pdf".into(), None, 1).unwrap();
        if count == 128 {
            assert_eq!(session.attachments().unwrap().len(), 128);
        } else {
            assert!(session.attachments().is_err());
        }
    }
    let mut document = sample();
    let payload = document.add_object(Stream::new(
        dictionary! { "Filter" => 42 },
        b"not-decoded".to_vec(),
    ));
    let specification = document.add_object(dictionary! {
        "UF" => text_string("malformed.bin"), "EF" => dictionary! { "UF" => payload },
    });
    document.catalog_mut().unwrap().set("Names", dictionary! {
        "EmbeddedFiles" => dictionary! { "Names" => vec![text_string("malformed.bin"), specification.into()] },
    });
    let session = Session::from_bytes(bytes(&mut document), "Filters.pdf".into(), None, 1).unwrap();
    assert!(session.attachments().is_err());
    assert!(session.attachment_bytes("0").is_err());
}

#[test]
fn metadata_field_and_encoded_packet_limits_are_not_character_count_proxies() {
    let mut session = Session::demo(1).unwrap();
    let mut within = fields();
    within.title = "é".repeat(2048);
    session
        .apply(Edit::Metadata {
            fields: within.clone(),
        })
        .unwrap();
    assert_eq!(session.metadata().unwrap().fields.title.len(), 4096);
    let before = session.bytes().to_vec();
    within.title.push('x');
    assert!(session.apply(Edit::Metadata { fields: within }).is_err());
    assert_eq!(session.bytes(), before);
    let mut document = sample();
    let metadata = document.add_object(Stream::new(dictionary! {}, vec![b' '; 1024 * 1024 + 1]));
    document.catalog_mut().unwrap().set("Metadata", metadata);
    let session =
        Session::from_bytes(bytes(&mut document), "Large XMP.pdf".into(), None, 1).unwrap();
    assert!(!session.metadata().unwrap().editable);
}

#[test]
fn normalized_owner_and_opening_password_collisions_are_rejected() {
    let session = Session::demo(1).unwrap();
    let options = CopyProtection::Protect {
        opening_password: "word space".into(),
        owner_password: "word\u{00a0}space".into(),
        permissions: CopyPermissions {
            print: PrintPermission::High,
            copy: true,
            edit: true,
            annotate: true,
            fill_forms: true,
            assemble: true,
        },
    };
    assert!(session.protection_copy(&options, None).is_err());
}

#[test]
fn protection_copy_is_real_aes256_r6_with_explicit_permissions_and_unchanged_session() {
    let mut session = Session::demo(1).unwrap();
    session.apply(Edit::Metadata { fields: fields() }).unwrap();
    session
        .add_attachment("synthetic.txt", b"private synthetic data".to_vec())
        .unwrap();
    let before = session.bytes().to_vec();
    let revision = session.revision();
    let output = session.protection_copy(&protected(), None).unwrap();
    assert!(Document::load_mem(&output).unwrap().is_encrypted());
    assert!(!String::from_utf8_lossy(&output).contains("private synthetic data"));
    assert!(Session::from_bytes(output.clone(), "Protected.pdf".into(), None, 2).is_err());
    let reader = open_protected(output.clone(), "reader");
    let access = reader.info().unwrap().permissions;
    assert!(!access.owner_authenticated);
    assert!(access.print_allowed);
    assert!(!access.high_quality_print_allowed);
    assert!(!access.copy_allowed);
    assert!(access.fill_forms_allowed);
    assert!(!access.modify_allowed);
    assert!(reader.attachment_bytes("0").is_err());
    let opened =
        Document::load_mem_with_options(&output, LoadOptions::with_password("reader")).unwrap();
    let state = opened.encryption_state.unwrap();
    assert_eq!(state.version(), 5);
    assert_eq!(state.revision(), 6);
    assert_eq!(state.file_encryption_key().len(), 32);
    let owner = open_protected(output, "owner");
    assert!(owner.info().unwrap().permissions.owner_authenticated);
    assert_eq!(owner.metadata().unwrap().fields, fields());
    assert_eq!(
        owner.attachment_bytes("0").unwrap().1,
        b"private synthetic data"
    );
    assert_eq!(session.bytes(), before);
    assert_eq!(session.revision(), revision);
    assert!(session.is_dirty());
    assert!(session.saved_path.is_none());
}

#[test]
fn replacing_or_removing_protection_requires_owner_even_if_user_permissions_allow_every_edit() {
    let mut session = Session::demo(1).unwrap();
    session.apply(Edit::Metadata { fields: fields() }).unwrap();
    let all = CopyProtection::Protect {
        opening_password: "reader".into(),
        owner_password: "owner".into(),
        permissions: CopyPermissions {
            print: PrintPermission::High,
            copy: true,
            edit: true,
            annotate: true,
            fill_forms: true,
            assemble: true,
        },
    };
    let original = session.protection_copy(&all, None).unwrap();
    let reader = open_protected(original.clone(), "reader");
    assert!(reader.info().unwrap().read_only_reason.is_none());
    for protection in [&protected(), &CopyProtection::Unprotect] {
        assert!(reader.protection_copy(protection, None).is_err());
        assert!(reader.protection_copy(protection, Some("reader")).is_err());
        assert!(reader.protection_copy(protection, Some("wrong")).is_err());
        assert_eq!(reader.bytes(), original);
    }
    let plain = reader
        .protection_copy(&CopyProtection::Unprotect, Some("owner"))
        .unwrap();
    let opened = Session::from_bytes(plain, "Plain.pdf".into(), None, 3).unwrap();
    assert!(!opened.info().unwrap().password_protected);
    assert_eq!(opened.metadata().unwrap().fields, fields());
    let changed = reader.protection_copy(&protected(), Some("owner")).unwrap();
    assert!(
        !open_protected(changed, "reader")
            .info()
            .unwrap()
            .copy_allowed
    );
    assert_eq!(reader.bytes(), original);
}

#[test]
fn protection_copy_guards_signatures_invalid_passwords_and_pending_drafts() {
    let mut document = sample();
    document.catalog_mut().unwrap().set(
        "Perms",
        dictionary! { "DocMDP" => dictionary! { "Type" => "Sig" } },
    );
    let session =
        Session::from_bytes(bytes(&mut document), "Certified.pdf".into(), None, 1).unwrap();
    assert!(session.info().unwrap().read_only_reason.is_some());
    assert!(session.protection_copy(&protected(), None).is_err());
    assert!(
        session
            .protection_copy(&CopyProtection::Unprotect, None)
            .is_err()
    );
    let session = Session::demo(1).unwrap();
    for (opening_password, owner_password) in [
        ("same".into(), "same".into()),
        ("".into(), "".into()),
        ("".into(), "é".repeat(64)),
    ] {
        let options = CopyProtection::Protect {
            opening_password,
            owner_password,
            permissions: CopyPermissions {
                print: PrintPermission::None,
                copy: false,
                edit: false,
                annotate: false,
                fill_forms: false,
                assemble: false,
            },
        };
        assert!(session.protection_copy(&options, None).is_err());
    }
    let mut workspace = Workspace::default();
    let first = workspace.bootstrap().unwrap();
    workspace.set_draft(first.id, true).unwrap();
    assert!(
        workspace
            .ensure_export_ready(first.id, first.revision)
            .is_err()
    );
    let second = workspace.open_demo().unwrap();
    assert!(
        workspace
            .ensure_export_ready(second.id, second.revision)
            .is_ok()
    );
    workspace.set_draft(first.id, false).unwrap();
    workspace
        .current_mut(first.id, first.revision)
        .unwrap()
        .apply(Edit::Metadata { fields: fields() })
        .unwrap();
    assert!(
        workspace
            .ensure_export_ready(first.id, first.revision)
            .is_err()
    );
}

#[test]
fn protected_metadata_bookmarks_and_attachments_keep_original_passwords_through_undo_and_save() {
    let source = Session::demo(1)
        .unwrap()
        .protection_copy(&protected(), None)
        .unwrap();
    let mut owner = open_protected(source.clone(), "owner");
    owner.apply(Edit::Metadata { fields: fields() }).unwrap();
    owner
        .apply(Edit::BookmarkCreate {
            parent: None,
            at: 0,
            title: "Protected bookmark".into(),
            page: 0,
        })
        .unwrap();
    owner
        .add_attachment("秘密.txt", b"synthetic protected payload".to_vec())
        .unwrap();
    assert!(Document::load_mem(owner.bytes()).unwrap().is_encrypted());
    let reopened = open_protected(owner.bytes().to_vec(), "owner");
    assert_eq!(reopened.metadata().unwrap().fields, fields());
    assert_eq!(reopened.attachments().unwrap()[0].name, "秘密.txt");
    assert_eq!(reopened.bookmarks().unwrap()[0].title, "Protected bookmark");
    owner.undo().unwrap();
    owner.redo().unwrap();
    assert_eq!(
        open_protected(owner.bytes().to_vec(), "owner")
            .attachments()
            .unwrap()[0]
            .name,
        "秘密.txt"
    );
    let mut reader = open_protected(source.clone(), "reader");
    assert!(reader.apply(Edit::Metadata { fields: fields() }).is_err());
    assert!(reader.add_attachment("no.txt", b"no".to_vec()).is_err());
    assert_eq!(reader.bytes(), source);
}
