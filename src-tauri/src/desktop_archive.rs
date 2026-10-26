use tauri::AppHandle;

use super::with_state;
use crate::{
    archive::{Plan, Request, SourceRef},
    preflight::Report,
};

#[tauri::command]
pub(super) async fn inspect_technical(app: AppHandle, source: SourceRef) -> Result<Report, String> {
    with_state(app, move |state| state.workspace.inspect_technical(source)).await
}

#[tauri::command]
pub(super) async fn check_inspection(app: AppHandle, inspection_id: u64) -> Result<Report, String> {
    with_state(app, move |state| {
        state.workspace.check_inspection(inspection_id)
    })
    .await
}

#[tauri::command]
pub(super) async fn cancel_inspection(app: AppHandle, inspection_id: u64) -> Result<bool, String> {
    with_state(app, move |state| {
        Ok(state.workspace.cancel_inspection(inspection_id))
    })
    .await
}

#[tauri::command]
pub(super) async fn save_inspection_report(
    app: AppHandle,
    inspection_id: u64,
    allow_unencrypted: bool,
) -> Result<Option<String>, String> {
    let plan = with_state(app.clone(), move |state| {
        state
            .workspace
            .inspection_report(inspection_id, allow_unencrypted)?;
        state.workspace.check_inspection(inspection_id)
    })
    .await?;
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_title("Export separate technical preflight report")
        .add_filter("JSON technical report", &["json"])
        .set_file_name(&plan.suggested_name)
        .save_file()
        .await
    else {
        return Ok(None);
    };
    let mut path = file.path().to_path_buf();
    if path.extension().is_none() {
        path.set_extension("json");
    }
    with_state(app, move |state| {
        state
            .workspace
            .write_inspection_report(inspection_id, allow_unencrypted, &path)?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
}

#[tauri::command]
pub(super) async fn begin_archive(app: AppHandle, request: Request) -> Result<Plan, String> {
    with_state(app, move |state| state.workspace.begin_archive(request)).await
}

#[tauri::command]
pub(super) async fn check_archive(app: AppHandle, archive_id: u64) -> Result<Plan, String> {
    with_state(app, move |state| state.workspace.check_archive(archive_id)).await
}

#[tauri::command]
pub(super) async fn archive_render_bytes(
    app: AppHandle,
    archive_id: u64,
) -> Result<tauri::ipc::Response, String> {
    with_state(app, move |state| {
        Ok(tauri::ipc::Response::new(
            state.workspace.archive_render_bytes(archive_id)?,
        ))
    })
    .await
}

#[tauri::command]
pub(super) async fn stage_archive_page(
    app: AppHandle,
    request: tauri::ipc::Request<'_>,
) -> Result<(), String> {
    let number = |name: &str| {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| format!("The archival {name} header is missing or invalid."))
    };
    let archive_id = number("x-folio-archive-id")?;
    let index = usize::try_from(number("x-folio-archive-index")?)
        .map_err(|_| "Invalid archival page index.")?;
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("Archival pages must be sent as raw PNG pixels.".into());
    };
    if bytes.len() > crate::archive::MAX_PNG_BYTES {
        return Err("An archival page exceeds 64 MiB of PNG.".into());
    }
    let bytes = bytes.clone();
    with_state(app, move |state| {
        state
            .workspace
            .stage_archive_page(archive_id, index, &bytes)
    })
    .await
}

#[tauri::command]
pub(super) async fn save_archive_copy(
    app: AppHandle,
    archive_id: u64,
) -> Result<Option<String>, String> {
    let plan = with_state(app.clone(), move |state| {
        state.workspace.archive_bytes(archive_id)?;
        state.workspace.check_archive(archive_id)
    })
    .await?;
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_title("Save fresh image-only PDF/A-2b archival copy")
        .add_filter("PDF archival copy", &["pdf"])
        .set_file_name(&plan.suggested_name)
        .save_file()
        .await
    else {
        return Ok(None);
    };
    let mut path = file.path().to_path_buf();
    if path.extension().is_none() {
        path.set_extension("pdf");
    }
    with_state(app, move |state| {
        // The dialog can outlive edits, cancellation, drafts, or closing a tab.
        state.workspace.write_archive_copy(archive_id, &path)?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
}

#[tauri::command]
pub(super) async fn cancel_archive(app: AppHandle, archive_id: u64) -> Result<bool, String> {
    with_state(app, move |state| {
        Ok(state.workspace.cancel_archive(archive_id))
    })
    .await
}
