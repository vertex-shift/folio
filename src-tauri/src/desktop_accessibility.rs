use tauri::AppHandle;

use super::with_state;
use crate::{
    accessibility::{CopyPlan, CopyRequest, Inspection},
    archive::SourceRef,
};

#[tauri::command]
pub(super) async fn inspect_accessibility(
    app: AppHandle,
    source: SourceRef,
) -> Result<Inspection, String> {
    with_state(app, move |state| {
        state.workspace.inspect_accessibility(source)
    })
    .await
}

#[tauri::command]
pub(super) async fn check_accessibility_inspection(
    app: AppHandle,
    inspection_id: u64,
) -> Result<Inspection, String> {
    with_state(app, move |state| {
        state
            .workspace
            .check_accessibility_inspection(inspection_id)
    })
    .await
}

#[tauri::command]
pub(super) async fn cancel_accessibility_inspection(
    app: AppHandle,
    inspection_id: u64,
) -> Result<bool, String> {
    with_state(app, move |state| {
        Ok(state
            .workspace
            .cancel_accessibility_inspection(inspection_id))
    })
    .await
}

#[tauri::command]
pub(super) async fn begin_accessibility_copy(
    app: AppHandle,
    request: CopyRequest,
) -> Result<CopyPlan, String> {
    with_state(app, move |state| {
        state.workspace.begin_accessibility_copy(request)
    })
    .await
}

#[tauri::command]
pub(super) async fn check_accessibility_copy(
    app: AppHandle,
    job_id: u64,
) -> Result<CopyPlan, String> {
    with_state(app, move |state| {
        state.workspace.check_accessibility_copy(job_id)
    })
    .await
}

#[tauri::command]
pub(super) async fn cancel_accessibility_copy(app: AppHandle, job_id: u64) -> Result<bool, String> {
    with_state(app, move |state| {
        Ok(state.workspace.cancel_accessibility_copy(job_id))
    })
    .await
}

#[tauri::command]
pub(super) async fn save_accessibility_copy(
    app: AppHandle,
    job_id: u64,
) -> Result<Option<String>, String> {
    let plan = with_state(app.clone(), move |state| {
        state.workspace.check_accessibility_copy(job_id)
    })
    .await?;
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_title("Save separate reviewed tagged PDF copy")
        .add_filter("Tagged PDF copy", &["pdf"])
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
        state.workspace.write_accessibility_copy(job_id, &path)?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
}
