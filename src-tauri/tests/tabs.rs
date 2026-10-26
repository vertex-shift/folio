use folio_core::{
    document::{Edit, Session},
    redaction::Region,
    workspace::{OpenOutcome, Workspace},
};

#[test]
fn tabs_keep_independent_bytes_history_revisions_and_unsaved_changes() {
    let mut workspace = Workspace::default();
    let first = workspace.bootstrap().unwrap();
    let edited = workspace
        .current_mut(first.id, first.revision)
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    let first_bytes = workspace.document(first.id).unwrap().bytes().to_vec();
    let second = workspace.open_demo().unwrap();
    assert_eq!(workspace.snapshot().unwrap().documents.len(), 2);
    assert_eq!(workspace.document(first.id).unwrap().bytes(), first_bytes);
    let note = workspace
        .current_mut(second.id, second.revision)
        .unwrap()
        .apply(Edit::Note {
            page: 1,
            point: [50.0, 100.0],
            text: "Second tab note".into(),
        })
        .unwrap();
    workspace
        .current_mut(first.id, edited.revision)
        .unwrap()
        .undo()
        .unwrap();
    assert_eq!(
        workspace
            .document(second.id)
            .unwrap()
            .info()
            .unwrap()
            .annotations[0]
            .text,
        "Second tab note"
    );
    assert_eq!(
        workspace.document(first.id).unwrap().info().unwrap().pages[0].rotation,
        0
    );
    assert!(workspace.current_mut(first.id, edited.revision).is_err());
    assert!(workspace.current_mut(second.id, note.revision).is_ok());
    workspace.activate(first.id).unwrap();
    assert_eq!(workspace.current().unwrap().id, first.id);
    assert!(workspace.is_dirty());
    workspace.activate(second.id).unwrap();
    assert_eq!(workspace.current().unwrap().id, second.id);
}

#[test]
fn closing_a_tab_checks_its_own_revision_edits_and_pending_marks() {
    let mut workspace = Workspace::default();
    let first = workspace.bootstrap().unwrap();
    let second = workspace.open_demo().unwrap();
    workspace.set_draft(first.id, true).unwrap();
    assert!(workspace.has_unsaved_changes());
    assert!(
        workspace
            .close_document(first.id, first.revision, false)
            .is_err()
    );
    assert_eq!(workspace.current().unwrap().id, second.id);
    workspace.set_draft(first.id, false).unwrap();
    let edited = workspace
        .current_mut(first.id, first.revision)
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    assert!(
        workspace
            .close_document(first.id, first.revision, true)
            .is_err()
    );
    assert!(
        workspace
            .close_document(first.id, edited.revision, false)
            .is_err()
    );
    let remaining = workspace
        .close_document(first.id, edited.revision, true)
        .unwrap();
    assert_eq!(remaining.documents.len(), 1);
    assert_eq!(remaining.active_id, Some(second.id));
    assert!(!workspace.has_unsaved_changes());
    assert!(workspace.document(first.id).is_err());
    assert!(workspace.activate(first.id).is_err());
}

#[test]
fn the_last_closed_tab_stays_empty_and_new_identifiers_are_never_reused() {
    let mut workspace = Workspace::default();
    let first = workspace.bootstrap().unwrap();
    let second = workspace.open_demo().unwrap();
    let third = workspace.open_demo().unwrap();
    workspace.activate(second.id).unwrap();
    let next = workspace
        .close_document(second.id, second.revision, false)
        .unwrap();
    assert_eq!(next.active_id, Some(third.id));
    workspace
        .close_document(third.id, third.revision, false)
        .unwrap();
    let empty = workspace
        .close_document(first.id, first.revision, false)
        .unwrap();
    assert!(empty.documents.is_empty());
    assert_eq!(empty.active_id, None);
    assert!(workspace.snapshot().unwrap().documents.is_empty());
    let reopened = workspace.open_demo().unwrap();
    assert!(reopened.id > third.id);
    assert!(workspace.current_mut(first.id, first.revision).is_err());
}

#[test]
fn each_documents_redaction_job_survives_other_tabs_and_only_its_own_new_generation_replaces_it() {
    let mut workspace = Workspace::default();
    let bytes = Session::blank(72.0, 72.0, 1).unwrap().bytes().to_vec();
    let first = workspace
        .open_bytes(bytes.clone(), "First.pdf".into())
        .unwrap();
    let second = workspace.open_bytes(bytes, "Second.pdf".into()).unwrap();
    let regions = [Region {
        page: 0,
        rect: [10.0, 10.0, 20.0, 20.0],
    }];
    let first_job = workspace
        .begin_redaction(first.id, first.revision, 150, &regions)
        .unwrap();
    let second_job = workspace
        .begin_redaction(second.id, second.revision, 150, &regions)
        .unwrap();
    assert!(workspace.check_redaction(first_job.job_id).is_ok());
    assert!(workspace.check_redaction(second_job.job_id).is_ok());
    let replacement = workspace
        .begin_redaction(first.id, first.revision, 150, &regions)
        .unwrap();
    assert!(workspace.check_redaction(first_job.job_id).is_err());
    assert!(workspace.check_redaction(replacement.job_id).is_ok());
    assert!(workspace.check_redaction(second_job.job_id).is_ok());
    workspace
        .close_document(first.id, first.revision, false)
        .unwrap();
    assert!(workspace.check_redaction(replacement.job_id).is_err());
    assert!(workspace.check_redaction(second_job.job_id).is_ok());
}

#[test]
fn batch_open_reports_bad_files_and_keeps_every_successful_document() {
    let folder = tempfile::tempdir().unwrap();
    let first = folder.path().join("First.pdf");
    let bad = folder.path().join("Bad.pdf");
    let second = folder.path().join("Second.pdf");
    let bytes = Session::blank(72.0, 72.0, 1).unwrap().bytes().to_vec();
    std::fs::write(&first, &bytes).unwrap();
    std::fs::write(&bad, b"not a PDF").unwrap();
    std::fs::write(&second, &bytes).unwrap();
    let mut workspace = Workspace::default();
    let opened = workspace.request_open_paths(vec![first, bad, second]);
    assert_eq!(opened.len(), 3);
    assert!(matches!(opened[0], OpenOutcome::Opened { .. }));
    assert!(
        matches!(&opened[1], OpenOutcome::Failed { name, message } if name == "Bad.pdf" && !message.is_empty())
    );
    assert!(matches!(opened[2], OpenOutcome::Opened { .. }));
    let snapshot = workspace.snapshot().unwrap();
    assert_eq!(snapshot.documents.len(), 2);
    assert_eq!(snapshot.documents[0].name, "First.pdf");
    assert_eq!(snapshot.documents[1].name, "Second.pdf");
    assert_eq!(snapshot.active_id, Some(snapshot.documents[1].id));
}

#[test]
fn saving_or_exporting_cannot_overwrite_another_open_tabs_source_or_saved_copy() {
    let folder = tempfile::tempdir().unwrap();
    let original = folder.path().join("First.pdf");
    let copy = folder.path().join("First-edited.pdf");
    let unused = folder.path().join("New-output.pdf");
    std::fs::write(&original, Session::blank(72.0, 72.0, 1).unwrap().bytes()).unwrap();
    let mut workspace = Workspace::default();
    let first = workspace.open_path(original.clone()).unwrap();
    workspace
        .current_mut(first.id, first.revision)
        .unwrap()
        .save_as(&copy)
        .unwrap();
    let second = workspace.open_demo().unwrap();
    assert!(
        workspace
            .ensure_output_available(second.id, &original)
            .is_err()
    );
    assert!(workspace.ensure_output_available(second.id, &copy).is_err());
    assert!(
        workspace
            .ensure_output_available(second.id, &unused)
            .is_ok()
    );
    #[cfg(unix)]
    {
        let alias = folder.path().join("Alias.pdf");
        std::os::unix::fs::symlink(&original, &alias).unwrap();
        assert!(
            workspace
                .ensure_output_available(second.id, &alias)
                .is_err()
        );
    }
    assert!(workspace.ensure_output_available(first.id, &copy).is_ok());
}
