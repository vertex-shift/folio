use std::{collections::BTreeMap, sync::Arc};

use folio_core::{
    document::{Edit, Error, Session},
    redaction::Region,
    text_edit::ReplacementFont,
    workspace::{OpenOutcome, Workspace},
};
use lopdf::{
    Document, LoadOptions, Object, Stream, dictionary,
    encryption::{
        EncryptionState, EncryptionVersion, Permissions,
        crypt_filters::{Aes128CryptFilter, Aes256CryptFilter, CryptFilter},
    },
};

fn encrypted(version: u8, user: &str, permissions: Permissions, signed: bool) -> Vec<u8> {
    let mut document = Document::load_mem(Session::blank(72.0, 72.0, 1).unwrap().bytes()).unwrap();
    document.trailer.set(
        "ID",
        vec![
            Object::string_literal("synthetic-file-id"),
            Object::string_literal("synthetic-file-id"),
        ],
    );
    let page = document.get_pages()[&1];
    let font = document.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let content = document.add_object(Stream::new(
        dictionary! {},
        b"BT /F1 4 Tf 2 30 Td (Synthetic protected sample) Tj ET".to_vec(),
    ));
    let page = document.get_dictionary_mut(page).unwrap();
    page.set("Contents", content);
    page.set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F1" => font } },
    );
    if signed {
        let signature = document.add_object(dictionary! {
            "Type" => "Sig", "ByteRange" => vec![0.into(), 1.into(), 2.into(), 3.into()],
            "Contents" => Object::string_literal("synthetic signature marker, not a real signature"),
        });
        document
            .catalog_mut()
            .unwrap()
            .set("FolioTestSignature", signature);
    }
    let method = match version {
        1 => EncryptionVersion::V1 {
            document: &document,
            owner_password: "owner",
            user_password: user,
            permissions,
        },
        2 => EncryptionVersion::V2 {
            document: &document,
            owner_password: "owner",
            user_password: user,
            permissions,
            key_length: 128,
        },
        4 => {
            let filter: Arc<dyn CryptFilter> = Arc::new(Aes128CryptFilter);
            EncryptionVersion::V4 {
                document: &document,
                owner_password: "owner",
                user_password: user,
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
                user_password: user,
                permissions,
                encrypt_metadata: true,
                file_encryption_key: &[42; 32],
                crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
                stream_filter: b"StdCF".to_vec(),
                string_filter: b"StdCF".to_vec(),
            }
        }
        _ => panic!("unsupported synthetic encryption version"),
    };
    let state = EncryptionState::try_from(method).unwrap();
    document.encrypt(&state).unwrap();
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

fn session(bytes: Vec<u8>, password: &str) -> Session {
    Session::from_bytes_with_password(bytes, "Protected.pdf".into(), None, 1, Some(password))
        .unwrap()
}

fn decrypted(bytes: &[u8], password: &str) -> Document {
    let document =
        Document::load_mem_with_options(bytes, LoadOptions::with_password(password)).unwrap();
    assert!(document.was_encrypted());
    assert!(!document.is_encrypted());
    document
}

fn challenge(outcome: OpenOutcome) -> u64 {
    match outcome {
        OpenOutcome::PasswordRequired {
            request_id,
            incorrect,
            ..
        } => {
            assert!(!incorrect);
            request_id
        }
        _ => panic!("expected a password challenge"),
    }
}

fn open(workspace: &mut Workspace, bytes: Vec<u8>, password: &str) {
    let request = challenge(
        workspace
            .request_open_bytes(bytes, "Protected.pdf".into())
            .unwrap(),
    );
    assert!(matches!(
        workspace.unlock_document(request, password).unwrap(),
        OpenOutcome::Opened { .. }
    ));
}

#[test]
fn protected_pdfs_prompt_and_wrong_passwords_or_cancellation_preserve_current_work() {
    let mut workspace = Workspace::default();
    let first = workspace.bootstrap().unwrap();
    workspace
        .current_mut(first.id, first.revision)
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    let before = workspace.current().unwrap().bytes().to_vec();
    let bytes = encrypted(4, "user", Permissions::all(), false);
    let request = challenge(
        workspace
            .request_open_bytes(bytes, "Protected.pdf".into())
            .unwrap(),
    );
    let retry = workspace.unlock_document(request, "wrong").unwrap();
    assert!(
        matches!(retry, OpenOutcome::PasswordRequired { request_id, incorrect: true, .. } if request_id == request)
    );
    assert_eq!(workspace.current().unwrap().id, first.id);
    assert_eq!(workspace.current().unwrap().bytes(), before);
    assert!(workspace.is_dirty());
    assert!(workspace.cancel_open(request));
    assert!(workspace.unlock_document(request, "user").is_err());
    assert_eq!(workspace.current().unwrap().bytes(), before);
}

#[test]
fn rc4_and_aes_saves_history_and_rendering_preserve_protection() {
    for version in [1, 2, 4, 5] {
        let original = encrypted(version, "user", Permissions::all(), false);
        assert!(matches!(
            Session::from_bytes(original.clone(), "test.pdf".into(), None, 1),
            Err(Error::PasswordRequired { incorrect: false })
        ));
        let mut session = session(original.clone(), "user");
        assert!(session.info().unwrap().password_protected);
        assert_eq!(session.bytes(), original);
        let rendered = Document::load_mem(&session.render_bytes().unwrap()).unwrap();
        assert!(!rendered.is_encrypted());
        assert_eq!(
            rendered.extract_text(&[1]).unwrap().trim(),
            "Synthetic protected sample"
        );
        let changed = session
            .apply(Edit::Rotate {
                page: 0,
                degrees: 90,
            })
            .unwrap();
        assert_eq!(changed.pages[0].rotation, 90);
        assert!(Document::load_mem(session.bytes()).unwrap().is_encrypted());
        for password in ["user", "owner"] {
            let output = decrypted(session.bytes(), password);
            let initial = decrypted(&original, password);
            assert_eq!(
                output
                    .encryption_state
                    .as_ref()
                    .unwrap()
                    .file_encryption_key(),
                initial
                    .encryption_state
                    .as_ref()
                    .unwrap()
                    .file_encryption_key(),
                "key changed for version {version}"
            );
            assert_eq!(
                output.trailer.get(b"ID").unwrap(),
                initial.trailer.get(b"ID").unwrap(),
                "file ID changed for version {version}"
            );
            assert_eq!(
                output.get_page_content(output.get_pages()[&1]),
                initial.get_page_content(initial.get_pages()[&1]),
                "stream changed for version {version}, password {password}"
            );
            assert_eq!(
                output.extract_text(&[1]).unwrap().trim(),
                "Synthetic protected sample",
                "content changed for version {version}"
            );
            assert_eq!(
                output.encryption_state.as_ref().unwrap().version(),
                i64::from(version)
            );
        }
        assert_eq!(session.undo().unwrap().pages[0].rotation, 0);
        assert_eq!(session.bytes(), original);
        assert_eq!(session.redo().unwrap().pages[0].rotation, 90);
        decrypted(session.bytes(), "user");
        assert!(!String::from_utf8_lossy(session.bytes()).contains("Synthetic protected sample"));
    }
}

#[test]
fn encrypted_text_replacement_annotations_and_disk_copies_reopen_with_both_passwords() {
    let original = encrypted(4, "user", Permissions::all(), false);
    let folder = tempfile::tempdir().unwrap();
    let source = folder.path().join("Original.pdf");
    let copy = folder.path().join("Edited.pdf");
    std::fs::write(&source, &original).unwrap();
    let mut working = Session::from_bytes_with_password(
        original.clone(),
        "Original.pdf".into(),
        Some(source.clone()),
        1,
        Some("user"),
    )
    .unwrap();
    let run = working
        .text_runs(0)
        .unwrap()
        .into_iter()
        .find(|run| run.text.is_some())
        .unwrap();
    working
        .apply(Edit::ReplaceText {
            page: 0,
            path: run.path,
            expected: run.text.unwrap(),
            replacement: "Replaced sample".into(),
            font: ReplacementFont::Original,
        })
        .unwrap();
    working
        .apply(Edit::Note {
            page: 0,
            point: [12.0, 12.0],
            text: "Private note".into(),
        })
        .unwrap();
    working.save_as(&copy).unwrap();
    assert_eq!(std::fs::read(source).unwrap(), original);
    for password in ["user", "owner"] {
        let opened = session(std::fs::read(&copy).unwrap(), password);
        assert_eq!(opened.info().unwrap().annotations[0].text, "Private note");
        assert!(
            opened
                .text_runs(0)
                .unwrap()
                .iter()
                .any(|run| run.text.as_deref() == Some("Replaced sample"))
        );
    }
    assert!(Session::from_path(copy, 2).is_err());
}

#[test]
fn restricted_user_access_is_read_only_but_owner_access_can_edit() {
    let bytes = encrypted(5, "user", Permissions::PRINTABLE, false);
    let mut reader = session(bytes.clone(), "user");
    let info = reader.info().unwrap();
    assert!(info.read_only_reason.is_some());
    assert!(!info.copy_allowed);
    assert!(
        reader
            .apply(Edit::Rotate {
                page: 0,
                degrees: 90
            })
            .is_err()
    );
    assert!(reader.extract_pages(&[0]).is_err());
    assert_eq!(reader.bytes(), bytes);
    let mut owner = session(bytes, "owner");
    assert!(owner.info().unwrap().read_only_reason.is_none());
    assert!(owner.info().unwrap().copy_allowed);
    owner
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    let output = decrypted(owner.bytes(), "user");
    assert_eq!(
        output
            .encryption_state
            .unwrap()
            .permissions()
            .intersection(Permissions::all()),
        Permissions::PRINTABLE
    );
}

#[test]
fn empty_opening_password_is_explicit_and_does_not_grant_owner_access() {
    let bytes = encrypted(4, "", Permissions::PRINTABLE, false);
    assert!(matches!(
        Session::from_bytes(bytes.clone(), "Restricted.pdf".into(), None, 1),
        Err(Error::PasswordRequired { incorrect: false })
    ));
    assert!(matches!(
        Session::from_bytes_with_password(
            bytes.clone(),
            "Restricted.pdf".into(),
            None,
            1,
            Some("wrong")
        ),
        Err(Error::PasswordRequired { incorrect: true })
    ));
    assert!(
        session(bytes.clone(), "")
            .info()
            .unwrap()
            .read_only_reason
            .is_some()
    );
    assert!(
        session(bytes, "owner")
            .info()
            .unwrap()
            .read_only_reason
            .is_none()
    );
}

#[test]
fn extraction_keeps_passwords_and_source_state() {
    for version in [1, 2, 4, 5] {
        let original = encrypted(version, "user", Permissions::all(), false);
        let session = session(original.clone(), "user");
        let extracted = session.extract_pages(&[0]).unwrap();
        for password in ["user", "owner"] {
            let document = decrypted(&extracted, password);
            assert_eq!(
                document.extract_text(&[1]).unwrap().trim(),
                "Synthetic protected sample"
            );
        }
        assert_eq!(session.bytes(), original);
        assert!(!session.is_dirty());
    }
}

#[test]
fn encrypted_signatures_stay_read_only_and_original_bytes_are_preserved() {
    let original = encrypted(4, "user", Permissions::all(), true);
    let mut session = session(original.clone(), "owner");
    assert!(
        session
            .info()
            .unwrap()
            .read_only_reason
            .unwrap()
            .contains("signed")
    );
    assert!(
        session
            .apply(Edit::Rotate {
                page: 0,
                degrees: 90
            })
            .is_err()
    );
    assert!(session.extract_pages(&[0]).is_err());
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("Signed-copy.pdf");
    session.save_as(&path).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), original);
}

#[test]
fn simultaneous_password_requests_are_independent_and_oversized_inputs_are_explicit() {
    let mut workspace = Workspace::default();
    let first = workspace.bootstrap().unwrap();
    let bytes = encrypted(4, "user", Permissions::all(), false);
    let old = challenge(
        workspace
            .request_open_bytes(bytes.clone(), "Old.pdf".into())
            .unwrap(),
    );
    let new = challenge(
        workspace
            .request_open_bytes(bytes.clone(), "New.pdf".into())
            .unwrap(),
    );
    assert!(workspace.cancel_open(old));
    assert!(workspace.unlock_document(old, "user").is_err());
    assert!(workspace.unlock_document(new, &"x".repeat(1025)).is_err());
    assert!(matches!(
        workspace.unlock_document(new, "wrong").unwrap(),
        OpenOutcome::PasswordRequired { .. }
    ));
    workspace
        .current_mut(first.id, first.revision)
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    assert!(matches!(
        workspace.unlock_document(new, "user").unwrap(),
        OpenOutcome::Opened { .. }
    ));
    assert_ne!(workspace.current().unwrap().id, first.id);
    assert!(workspace.document(first.id).unwrap().is_dirty());
}

#[test]
fn path_opening_caches_only_the_selected_source_and_does_not_serialize_passwords() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("Synthetic protected.pdf");
    let password = "synthetic-password-not-in-metadata";
    let original = encrypted(5, password, Permissions::all(), false);
    std::fs::write(&path, &original).unwrap();
    let mut workspace = Workspace::default();
    workspace.bootstrap().unwrap();
    let result = workspace.request_open_path(path.clone()).unwrap();
    assert!(!serde_json::to_string(&result).unwrap().contains(password));
    let request = challenge(result);
    std::fs::remove_file(path).unwrap();
    let result = workspace.unlock_document(request, password).unwrap();
    assert!(!serde_json::to_string(&result).unwrap().contains(password));
    assert_eq!(workspace.current().unwrap().bytes(), original);
    assert_eq!(workspace.current().unwrap().info().unwrap().page_count, 1);
}

#[test]
fn unsupported_security_handlers_are_not_misreported_as_password_challenges() {
    let bytes = encrypted(4, "user", Permissions::all(), false);
    let mut document = Document::load_mem(&bytes).unwrap();
    let encrypt = document
        .trailer
        .get(b"Encrypt")
        .unwrap()
        .as_reference()
        .unwrap();
    document
        .get_dictionary_mut(encrypt)
        .unwrap()
        .set("Filter", "Adobe.PubSec");
    let mut unsupported = Vec::new();
    document.save_to(&mut unsupported).unwrap();
    let error = match Session::from_bytes(unsupported, "Unsupported.pdf".into(), None, 1) {
        Ok(_) => panic!("unsupported protection must fail"),
        Err(error) => error,
    };
    assert!(!matches!(error, Error::PasswordRequired { .. }));
    assert!(error.to_string().contains("unsupported"));
}

#[test]
fn imports_use_destination_protection_and_never_silently_decrypt_into_plain_documents() {
    let source = encrypted(4, "source", Permissions::all(), false);
    let mut workspace = Workspace::default();
    let plain = workspace.bootstrap().unwrap();
    let request = challenge(
        workspace
            .request_import_bytes(
                plain.id,
                plain.revision,
                1,
                source.clone(),
                "Donor.pdf".into(),
            )
            .unwrap(),
    );
    assert!(workspace.unlock_document(request, "source").is_err());
    assert_eq!(workspace.current().unwrap().id, plain.id);
    open(
        &mut workspace,
        encrypted(5, "destination", Permissions::all(), false),
        "destination",
    );
    let info = workspace.current().unwrap().info().unwrap();
    let request = challenge(
        workspace
            .request_import_bytes(info.id, info.revision, 1, source, "Donor.pdf".into())
            .unwrap(),
    );
    let result = workspace.unlock_document(request, "source").unwrap();
    assert!(matches!(
        result,
        OpenOutcome::Opened {
            imported_at: Some(1),
            ..
        }
    ));
    let result = decrypted(workspace.current().unwrap().bytes(), "destination");
    assert_eq!(result.get_pages().len(), 2);
    assert_eq!(
        result.extract_text(&[2]).unwrap().trim(),
        "Synthetic protected sample"
    );
}

#[test]
fn pending_imports_are_guarded_by_their_target_tab_not_the_current_tab() {
    let mut workspace = Workspace::default();
    open(
        &mut workspace,
        encrypted(5, "destination", Permissions::all(), false),
        "destination",
    );
    let target = workspace.current().unwrap().info().unwrap();
    let donor = encrypted(4, "source", Permissions::all(), false);
    let request = challenge(
        workspace
            .request_import_bytes(
                target.id,
                target.revision,
                1,
                donor.clone(),
                "Donor.pdf".into(),
            )
            .unwrap(),
    );
    let other = workspace.open_demo().unwrap();
    assert!(
        matches!(workspace.unlock_document(request, "source").unwrap(),
        OpenOutcome::Opened { document, imported_at: Some(1) } if document.id == target.id)
    );
    assert_eq!(workspace.current().unwrap().id, other.id);
    assert_eq!(
        workspace
            .document(target.id)
            .unwrap()
            .info()
            .unwrap()
            .page_count,
        2
    );
    let revision = workspace.document(target.id).unwrap().revision();
    let stale = challenge(
        workspace
            .request_import_bytes(target.id, revision, 1, donor, "Donor.pdf".into())
            .unwrap(),
    );
    workspace
        .current_mut(target.id, revision)
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    assert!(workspace.unlock_document(stale, "source").is_err());
    assert_eq!(workspace.current().unwrap().id, other.id);
    assert!(!workspace.document(other.id).unwrap().is_dirty());
}

#[test]
fn encrypted_tabs_keep_their_own_passwords_when_an_inactive_document_is_edited() {
    let mut workspace = Workspace::default();
    open(
        &mut workspace,
        encrypted(4, "first-password", Permissions::all(), false),
        "first-password",
    );
    let first = workspace.current().unwrap().info().unwrap();
    open(
        &mut workspace,
        encrypted(5, "second-password", Permissions::all(), false),
        "second-password",
    );
    let second = workspace.current().unwrap().info().unwrap();
    workspace
        .current_mut(first.id, first.revision)
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    workspace
        .current_mut(second.id, second.revision)
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 180,
        })
        .unwrap();
    for (id, password, other_password, rotation) in [
        (first.id, "first-password", "second-password", 90),
        (second.id, "second-password", "first-password", 180),
    ] {
        let bytes = workspace.document(id).unwrap().bytes();
        let reopened = session(bytes.to_vec(), password);
        assert_eq!(reopened.info().unwrap().pages[0].rotation, rotation);
        assert!(
            Document::load_mem_with_options(bytes, LoadOptions::with_password(other_password))
                .is_err()
        );
    }
    assert_eq!(workspace.current().unwrap().id, second.id);
}

#[test]
fn redacted_copies_preserve_passwords_without_original_objects_or_text() {
    for version in [2, 4, 5] {
        let mut workspace = Workspace::default();
        workspace.bootstrap().unwrap();
        let original = encrypted(version, "user", Permissions::all(), false);
        open(&mut workspace, original.clone(), "user");
        let info = workspace.current().unwrap().info().unwrap();
        let plan = workspace
            .begin_redaction(
                info.id,
                info.revision,
                150,
                &[Region {
                    page: 0,
                    rect: [10.0, 10.0, 30.0, 30.0],
                }],
            )
            .unwrap();
        let page = &plan.pages[0];
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, page.pixel_width, page.pixel_height);
            encoder.set_color(png::ColorType::Rgb);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&vec![
                    255;
                    page.pixel_width as usize
                        * page.pixel_height as usize
                        * 3
                ])
                .unwrap();
        }
        workspace
            .stage_redaction_page(plan.job_id, 0, &png)
            .unwrap();
        let redacted = workspace.redaction_bytes(plan.job_id).unwrap();
        for password in ["user", "owner"] {
            let output = decrypted(&redacted, password);
            assert!(output.extract_text(&[1]).unwrap().trim().is_empty());
            assert!(
                !output
                    .objects
                    .values()
                    .any(|value| value.as_dict().is_ok_and(|dict| dict
                        .get(b"Type")
                        .and_then(Object::as_name)
                        .ok()
                        == Some(b"Font")))
            );
            let image = output
                .objects
                .values()
                .filter_map(|value| value.as_stream().ok())
                .find(|stream| {
                    stream.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Image")
                })
                .unwrap();
            let rgb = image.decompressed_content().unwrap();
            let mask = &page.masks[0];
            let offset = (mask.y as usize * page.pixel_width as usize + mask.x as usize) * 3;
            assert_eq!(&rgb[offset..offset + 3], &[0, 0, 0]);
            assert_eq!(&rgb[0..3], &[255, 255, 255]);
        }
        assert_eq!(workspace.current().unwrap().bytes(), original);
        assert!(!workspace.is_dirty());
    }
}
