use tauri::AppHandle;

use super::with_state;
use crate::comparison::{MAX_REPORT_BYTES, PageResult, Plan, Request, Side};

#[tauri::command]
pub(super) async fn begin_comparison(app: AppHandle, request: Request) -> Result<Plan, String> {
    with_state(app, move |state| state.workspace.begin_comparison(request)).await
}

#[tauri::command]
pub(super) async fn check_comparison(app: AppHandle, comparison_id: u64) -> Result<Plan, String> {
    with_state(app, move |state| {
        state.workspace.check_comparison(comparison_id)
    })
    .await
}

#[tauri::command]
pub(super) async fn comparison_render_bytes(
    app: AppHandle,
    comparison_id: u64,
    side: Side,
) -> Result<tauri::ipc::Response, String> {
    with_state(app, move |state| {
        Ok(tauri::ipc::Response::new(
            state
                .workspace
                .comparison_render_bytes(comparison_id, side)?,
        ))
    })
    .await
}

#[tauri::command]
pub(super) async fn finish_comparison(
    app: AppHandle,
    request: tauri::ipc::Request<'_>,
) -> Result<(), String> {
    let comparison_id = request
        .headers()
        .get("x-folio-comparison-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or("The comparison identifier is missing or invalid.")?;
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("Comparison results must be sent as bounded UTF-8 JSON.".into());
    };
    if bytes.len() > MAX_REPORT_BYTES {
        return Err("Comparison results exceed 4 MiB.".into());
    }
    let pages: Vec<PageResult> = serde_json::from_slice(bytes)
        .map_err(|error| format!("The comparison result is invalid: {error}"))?;
    with_state(app, move |state| {
        state.workspace.finish_comparison(comparison_id, pages)
    })
    .await
}

#[tauri::command]
pub(super) async fn save_comparison_report(
    app: AppHandle,
    comparison_id: u64,
    allow_unencrypted: bool,
) -> Result<Option<String>, String> {
    let plan = with_state(app.clone(), move |state| {
        state
            .workspace
            .comparison_report(comparison_id, allow_unencrypted)?;
        state.workspace.check_comparison(comparison_id)
    })
    .await?;
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_title("Export a separate comparison report")
        .add_filter("JSON comparison report", &["json"])
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
            .write_comparison_report(comparison_id, allow_unencrypted, &path)?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
}

#[tauri::command]
pub(super) async fn cancel_comparison(app: AppHandle, comparison_id: u64) -> Result<bool, String> {
    with_state(app, move |state| {
        Ok(state.workspace.cancel_comparison(comparison_id))
    })
    .await
}
