use tauri::AppHandle;

use super::{pdf_output_path, with_state};
use crate::{
    attachments::{AttachmentInfo, read_picked_file},
    document::{DocumentInfo, atomic_write},
    metadata::MetadataInfo,
    outlines::BookmarkInfo,
    protection::CopyProtection,
};

#[tauri::command]
pub(super) async fn document_metadata(
    app: AppHandle,
    document_id: u64,
    revision: u64,
) -> Result<MetadataInfo, String> {
    with_state(app, move |state| {
        state
            .workspace
            .check_revision(document_id, revision)?
            .metadata()
    })
    .await
}

#[tauri::command]
pub(super) async fn document_bookmarks(
    app: AppHandle,
    document_id: u64,
    revision: u64,
) -> Result<Vec<BookmarkInfo>, String> {
    with_state(app, move |state| {
        state
            .workspace
            .check_revision(document_id, revision)?
            .bookmarks()
    })
    .await
}

#[tauri::command]
pub(super) async fn document_attachments(
    app: AppHandle,
    document_id: u64,
    revision: u64,
) -> Result<Vec<AttachmentInfo>, String> {
    with_state(app, move |state| {
        state
            .workspace
            .check_revision(document_id, revision)?
            .attachments()
    })
    .await
}

#[tauri::command]
pub(super) async fn add_document_attachment(
    app: AppHandle,
    document_id: u64,
    revision: u64,
) -> Result<Option<DocumentInfo>, String> {
    with_state(app.clone(), move |state| {
        state
            .workspace
            .ensure_export_ready(document_id, revision)?
            .ensure_management_writable()
    })
    .await?;
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_title("Add a file attachment (never opened or executed)")
        .pick_file()
        .await
    else {
        return Ok(None);
    };
    let path = file.path().to_path_buf();
    with_state(app, move |state| {
        state
            .workspace
            .ensure_export_ready(document_id, revision)?
            .ensure_management_writable()?;
        let (name, bytes) = read_picked_file(&path)?;
        state
            .workspace
            .current_mut(document_id, revision)?
            .add_attachment(&name, bytes)
    })
    .await
    .map(Some)
}

#[tauri::command]
pub(super) async fn export_document_attachment(
    app: AppHandle,
    document_id: u64,
    revision: u64,
    attachment_id: String,
) -> Result<Option<String>, String> {
    let (name, bytes) = with_state(app.clone(), move |state| {
        state
            .workspace
            .ensure_export_ready(document_id, revision)?
            .attachment_bytes(&attachment_id)
    })
    .await?;
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_title("Save untrusted attachment (do not open automatically)")
        .set_file_name(&name)
        .save_file()
        .await
    else {
        return Ok(None);
    };
    let path = file.path().to_path_buf();
    with_state(app, move |state| {
        let session = state.workspace.ensure_export_ready(document_id, revision)?;
        session.ensure_separate_output(&path)?;
        state
            .workspace
            .ensure_output_available(document_id, &path)?;
        atomic_write(&path, &bytes)?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await
    .map(Some)
}

#[tauri::command]
pub(super) async fn export_protection_copy(
    app: AppHandle,
    document_id: u64,
    revision: u64,
    protection: CopyProtection,
    authorization_password: Option<String>,
) -> Result<Option<String>, String> {
    let (bytes, name) = with_state(app.clone(), move |state| {
        let session = state.workspace.ensure_export_ready(document_id, revision)?;
        let suffix = if matches!(protection, CopyProtection::Unprotect) {
            "unprotected"
        } else {
            "protected"
        };
        let stem = crate::attachments::suggested_basename(&session.name);
        let stem = stem
            .strip_suffix(".pdf")
            .or_else(|| stem.strip_suffix(".PDF"))
            .unwrap_or(&stem);
        let name = format!("{stem}-{suffix}.pdf");
        Ok((
            session.protection_copy(&protection, authorization_password.as_deref())?,
            name,
        ))
    })
    .await?;
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_title("Export a separate PDF protection copy")
        .add_filter("PDF document", &["pdf"])
        .set_file_name(&name)
        .save_file()
        .await
    else {
        return Ok(None);
    };
    let path = pdf_output_path(file.path().to_path_buf())?;
    with_state(app, move |state| {
        let session = state.workspace.ensure_export_ready(document_id, revision)?;
        session.ensure_separate_output(&path)?;
        state
            .workspace
            .ensure_output_available(document_id, &path)?;
        atomic_write(&path, &bytes)?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await
    .map(Some)
}
