use folio_core::{
    document::Edit,
    updates::{Lifecycle, Phase, authorize_close},
    workspace::Workspace,
};

#[test]
fn rejected_or_canceled_quit_retains_ready_update_and_unsaved_inactive_tab() {
    let mut workspace = Workspace::default();
    let first = workspace.create_document(72.0, 144.0).unwrap();
    let second = workspace.open_demo().unwrap();
    workspace.activate(second.id).unwrap();
    let mut update = Lifecycle::default();
    assert!(update.begin_check());
    update.finish(Some("verified update"));
    let original = workspace.document(first.id).unwrap().bytes().to_vec();
    assert!(authorize_close(&mut workspace, false).is_err());
    assert_eq!(update.status.phase, Phase::Ready);
    assert_eq!(workspace.document(first.id).unwrap().bytes(), original);
    // Cancel/review does not call begin_close; only an accepted discard can install.
    authorize_close(&mut workspace, true).unwrap();
    assert_eq!(update.begin_close(), Some("verified update"));
}

#[test]
fn pending_drafts_on_clean_tabs_block_update_installation() {
    let mut workspace = Workspace::default();
    let document = workspace.open_demo().unwrap();
    assert!(!document.dirty);
    workspace.set_draft(document.id, true).unwrap();
    assert!(authorize_close(&mut workspace, false).is_err());
    workspace.set_draft(document.id, false).unwrap();
    authorize_close(&mut workspace, false).unwrap();
}

#[test]
fn failed_install_restores_recovery_and_never_mutates_source_or_history() {
    let directory = tempfile::tempdir().unwrap();
    let mut workspace = Workspace::default();
    workspace.configure_recovery(directory.path().join("recovery"));
    workspace.set_recovery_enabled(true).unwrap();
    let document = workspace.open_demo().unwrap();
    let session = workspace
        .current_mut(document.id, document.revision)
        .unwrap();
    session
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    workspace.sync_recovery();
    assert_eq!(workspace.recovery_status().checkpointed, 1);
    let before = workspace.document(document.id).unwrap().bytes().to_vec();
    authorize_close(&mut workspace, true).unwrap();
    assert_eq!(workspace.recovery_status().checkpointed, 0);
    workspace.resume_recovery_after_failed_exit();
    assert_eq!(workspace.recovery_status().checkpointed, 1);
    let session = workspace.document(document.id).unwrap();
    assert_eq!(session.bytes(), before);
    assert!(session.info().unwrap().can_undo);
}
