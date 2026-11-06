use std::fs;

use folio_core::{
    document::{Edit, Session},
    redaction::Region,
    workspace::{OpenOutcome, Workspace},
};
use lopdf::{
    Document, Object, dictionary,
    encryption::{EncryptionState, EncryptionVersion, Permissions},
};

fn workspace(root: &std::path::Path) -> Workspace {
    let mut workspace = Workspace::default();
    workspace.configure_recovery(root.to_path_buf());
    assert!(workspace.recovery_status().available);
    workspace
}

fn rotate(workspace: &mut Workspace, id: u64) {
    let revision = workspace.document(id).unwrap().revision();
    workspace
        .current_mut(id, revision)
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    workspace.sync_recovery();
}

fn copies(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "folio-recovery"))
        .collect()
}

#[test]
fn opt_in_restart_recovers_exact_applied_bytes_and_preserves_sources_and_other_tabs() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("recovery");
    let source = directory.path().join("source.pdf");
    let saved = directory.path().join("previous-copy.pdf");
    let source_bytes = Session::blank(72.0, 144.0, 1).unwrap().bytes().to_vec();
    fs::write(&source, &source_bytes).unwrap();
    let expected;
    {
        let mut workspace = workspace(&root);
        assert!(!workspace.recovery_status().enabled);
        let first = workspace.open_path(source.clone()).unwrap();
        rotate(&mut workspace, first.id);
        assert!(copies(&root).is_empty());
        workspace.set_recovery_enabled(true).unwrap();
        assert_eq!(copies(&root).len(), 1);
        let revision = workspace.document(first.id).unwrap().revision();
        workspace
            .current_mut(first.id, revision)
            .unwrap()
            .save_as(&saved)
            .unwrap();
        workspace.sync_recovery();
        assert!(copies(&root).is_empty());
        rotate(&mut workspace, first.id);
        expected = workspace.document(first.id).unwrap().bytes().to_vec();
        let second = workspace.create_document(100.0, 100.0).unwrap();
        workspace.sync_recovery();
        assert_eq!(copies(&root).len(), 2);
        assert_ne!(first.id, second.id);
        assert_eq!(fs::read(&source).unwrap(), source_bytes);
    }
    let saved_bytes = fs::read(&saved).unwrap();
    let token;
    {
        let mut workspace = workspace(&root);
        let entries = workspace.recovery_status().entries;
        assert_eq!(entries.len(), 2);
        token = entries
            .iter()
            .find(|entry| entry.name == "previous-copy.pdf")
            .unwrap()
            .token
            .clone();
        let other = workspace.open_demo().unwrap();
        let recovered = workspace.recover_document(token.clone()).unwrap();
        assert!(recovered.dirty);
        assert!(!recovered.can_undo && !recovered.can_redo);
        assert_eq!(workspace.document(recovered.id).unwrap().bytes(), expected);
        assert!(workspace.document(other.id).is_ok());
        assert!(workspace.recover_document(token.clone()).is_err());
        assert!(
            workspace
                .close_document(recovered.id, recovered.revision, false)
                .is_err()
        );
        assert!(
            workspace
                .ensure_output_available(recovered.id, &source)
                .is_err()
        );
        assert!(
            workspace
                .ensure_output_available(recovered.id, &saved)
                .is_err()
        );
        assert!(
            workspace
                .current_mut(recovered.id, recovered.revision)
                .unwrap()
                .save_as(&source)
                .is_err()
        );
        rotate(&mut workspace, recovered.id);
        // A second crash must retain the original no-clobber paths.
    }
    let mut workspace = workspace(&root);
    let recovered = workspace.recover_document(token).unwrap();
    assert!(
        workspace
            .ensure_output_available(recovered.id, &saved)
            .is_err()
    );
    assert_eq!(fs::read(&source).unwrap(), source_bytes);
    assert_eq!(fs::read(&saved).unwrap(), saved_bytes);
    let output = directory.path().join("recovered.pdf");
    workspace
        .current_mut(recovered.id, recovered.revision)
        .unwrap()
        .save_as(&output)
        .unwrap();
    workspace.sync_recovery();
    assert_eq!(copies(&root).len(), 1);
    assert!(Session::from_path(output, 99).is_ok());
    workspace.set_recovery_enabled(false).unwrap();
    assert!(copies(&root).is_empty());
}

#[test]
fn pending_forms_redactions_and_staging_remove_checkpoints_without_clearing_draft_guards() {
    let directory = tempfile::tempdir().unwrap();
    {
        let mut workspace = workspace(directory.path());
        workspace.set_recovery_enabled(true).unwrap();
        let info = workspace.create_document(100.0, 100.0).unwrap();
        workspace.sync_recovery();
        assert_eq!(copies(directory.path()).len(), 1);
        workspace.set_draft(info.id, true).unwrap();
        workspace.sync_recovery();
        assert!(copies(directory.path()).is_empty());
        assert_eq!(workspace.recovery_status().excluded.len(), 1);
        assert!(workspace.has_unsaved_changes());
        assert!(
            workspace
                .close_document(info.id, info.revision, false)
                .is_err()
        );
        workspace.set_draft(info.id, false).unwrap();
        workspace.sync_recovery();
        assert_eq!(copies(directory.path()).len(), 1);
        workspace
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
        workspace.sync_recovery();
        assert!(copies(directory.path()).is_empty());
        assert!(workspace.has_unsaved_changes());
    }
    assert!(
        workspace(directory.path())
            .recovery_status()
            .entries
            .is_empty()
    );
}

#[test]
fn protected_pdfs_never_persist_passwords_or_rendering_bytes_even_after_owner_unlock() {
    let directory = tempfile::tempdir().unwrap();
    let mut document =
        Document::load_mem(Session::blank(100.0, 100.0, 1).unwrap().bytes()).unwrap();
    document.trailer.set(
        "ID",
        vec![
            Object::string_literal("recovery-test"),
            Object::string_literal("recovery-test"),
        ],
    );
    let encryption = EncryptionState::try_from(EncryptionVersion::V2 {
        document: &document,
        owner_password: "synthetic-owner-secret",
        user_password: "synthetic-user-secret",
        permissions: Permissions::all(),
        key_length: 128,
    })
    .unwrap();
    document.encrypt(&encryption).unwrap();
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    {
        let mut workspace = workspace(directory.path());
        workspace.set_recovery_enabled(true).unwrap();
        let challenge = workspace
            .request_open_bytes(bytes.clone(), "protected.pdf".into())
            .unwrap();
        let OpenOutcome::PasswordRequired { request_id, .. } = challenge else {
            panic!("password challenge");
        };
        let OpenOutcome::Opened { document, .. } = workspace
            .unlock_document(request_id, "synthetic-owner-secret")
            .unwrap()
        else {
            panic!("owner unlock");
        };
        rotate(&mut workspace, document.id);
        assert!(workspace.document(document.id).unwrap().is_dirty());
        assert!(copies(directory.path()).is_empty());
        assert_eq!(workspace.recovery_status().excluded.len(), 1);
        for entry in fs::read_dir(directory.path()).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name() == "writer.lock" {
                assert_eq!(entry.metadata().unwrap().len(), 0);
                continue;
            }
            let contents = fs::read(entry.path()).unwrap();
            assert!(
                !contents
                    .windows(22)
                    .any(|window| window == b"synthetic-owner-secret")
            );
            assert!(!contents.starts_with(b"%PDF"));
        }
    }
    assert!(
        workspace(directory.path())
            .recovery_status()
            .entries
            .is_empty()
    );
    assert!(
        Session::from_bytes_with_password(
            bytes,
            "protected.pdf".into(),
            None,
            1,
            Some("synthetic-user-secret")
        )
        .is_ok()
    );
}

#[test]
fn cleanup_is_serialized_with_save_close_discard_and_explicit_quit() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("recovery");
    {
        let mut workspace = workspace(&root);
        workspace.set_recovery_enabled(true).unwrap();
        let info = workspace.create_document(100.0, 100.0).unwrap();
        workspace.sync_recovery();
        workspace
            .close_document(info.id, info.revision, true)
            .unwrap();
        // Repeated/late status syncs cannot recreate a closed tab's checkpoint.
        workspace.sync_recovery();
        workspace.sync_recovery();
        assert!(copies(&root).is_empty());
        workspace.create_document(100.0, 100.0).unwrap();
        workspace.sync_recovery();
        workspace.clean_recovery_exit().unwrap();
        workspace.sync_recovery();
        assert!(copies(&root).is_empty());
    }
    {
        let mut workspace = workspace(&root);
        workspace.create_document(100.0, 100.0).unwrap();
        workspace.sync_recovery();
    }
    let mut workspace = workspace(&root);
    let token = workspace.recovery_status().entries[0].token.clone();
    assert!(workspace.discard_recovery("../source.pdf").is_err());
    workspace.discard_recovery(&token).unwrap();
    workspace.sync_recovery();
    assert!(copies(&root).is_empty());
}

#[test]
fn damaged_copies_failed_writes_exclusive_lock_and_interrupted_temps_are_explicit() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("recovery");
    {
        let mut workspace = workspace(&root);
        let mut other = Workspace::default();
        other.configure_recovery(root.clone());
        assert!(!other.recovery_status().available);
        assert!(other.recovery_status().error.unwrap().contains("locked"));
        workspace.set_recovery_enabled(true).unwrap();
        let info = workspace.create_document(100.0, 100.0).unwrap();
        workspace.sync_recovery();
        let checkpoint = copies(&root).pop().unwrap();
        fs::remove_file(&checkpoint).unwrap();
        fs::create_dir(&checkpoint).unwrap();
        rotate(&mut workspace, info.id);
        assert!(workspace.recovery_status().error.is_some());
        assert!(workspace.document(info.id).unwrap().is_dirty());
        fs::remove_dir(&checkpoint).unwrap();
        workspace.sync_recovery();
        assert!(workspace.recovery_status().error.is_none());
    }
    let checkpoint = copies(&root).pop().unwrap();
    let mut bytes = fs::read(&checkpoint).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(&checkpoint, bytes).unwrap();
    fs::write(
        root.join(".folio-recovery-tmp-interrupted"),
        b"partial synthetic PDF",
    )
    .unwrap();
    let mut workspace = workspace(&root);
    assert!(!root.join(".folio-recovery-tmp-interrupted").exists());
    let token = workspace.recovery_status().entries[0].token.clone();
    assert!(
        workspace
            .recover_document(token.clone())
            .unwrap_err()
            .to_string()
            .contains("damaged")
    );
    assert_eq!(workspace.recovery_status().entries.len(), 1);
    fs::write(&checkpoint, b"bad header").unwrap();
    assert!(workspace.recovery_status().entries[0].problem.is_some());
    workspace.discard_recovery(&token).unwrap();
}

#[test]
fn copy_count_limit_never_evicts_previous_work_or_claims_recent_edits_are_checkpointed() {
    let directory = tempfile::tempdir().unwrap();
    let mut workspace = workspace(directory.path());
    workspace.set_recovery_enabled(true).unwrap();
    for _ in 0..17 {
        workspace.create_document(72.0, 72.0).unwrap();
    }

    workspace.sync_recovery();
    assert_eq!(copies(directory.path()).len(), 16);
    assert!(
        workspace
            .recovery_status()
            .error
            .unwrap()
            .contains("16-copy")
    );
    assert_eq!(workspace.snapshot().unwrap().documents.len(), 17);
    workspace.set_recovery_enabled(false).unwrap();
    assert!(copies(directory.path()).is_empty());
    workspace.sync_recovery();
    assert!(copies(directory.path()).is_empty());
}

#[test]
fn storage_byte_budget_and_read_only_guards_fail_without_persisting_document_contents() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("budget");
    let mut workspace = workspace(&root);
    workspace.set_recovery_enabled(true).unwrap();
    let info = workspace.create_document(72.0, 72.0).unwrap();
    let occupied = root.join("00000000000000000000000000000000.folio-recovery");
    fs::File::create(&occupied)
        .unwrap()
        .set_len(256 * 1024 * 1024)
        .unwrap();
    workspace.sync_recovery();
    assert_eq!(copies(&root).len(), 1);
    assert!(
        workspace
            .recovery_status()
            .error
            .unwrap()
            .contains("256 MiB")
    );
    assert!(workspace.document(info.id).unwrap().is_dirty());
    workspace
        .discard_recovery("00000000000000000000000000000000")
        .unwrap();
    workspace.sync_recovery();
    assert_eq!(workspace.recovery_status().checkpointed, 1);

    let mut document = Document::load_mem(Session::blank(72.0, 72.0, 1).unwrap().bytes()).unwrap();
    let signature = document.add_object(dictionary! {
        "Type" => "Sig", "ByteRange" => vec![0.into(), 1.into(), 2.into(), 3.into()],
        "Contents" => Object::string_literal("synthetic read-only marker"),
    });
    document
        .catalog_mut()
        .unwrap()
        .set("FolioTestSignature", signature);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    let signed = Session::from_bytes(bytes, "signed.pdf".into(), None, 1).unwrap();
    assert!(signed.info().unwrap().read_only_reason.is_some());
    let protected_root = directory.path().join("readonly");
    let mut store = folio_core::recovery::Store::open(protected_root.clone()).unwrap();
    store.set_enabled(true).unwrap();
    assert!(store.sync(&[&signed]).is_err());
    assert!(copies(&protected_root).is_empty());
}
