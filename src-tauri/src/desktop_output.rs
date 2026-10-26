use tauri::{AppHandle, Manager};

use super::with_state;
use crate::{
    document::invalid,
    output::{
        MAX_IMAGE_BYTES, OptimizationReport, OutputKind, OutputPlan, OutputRequest, output_path,
    },
};

#[tauri::command]
pub(super) async fn begin_document_output(
    app: AppHandle,
    request: OutputRequest,
) -> std::result::Result<OutputPlan, String> {
    with_state(app, move |state| state.workspace.begin_output(request)).await
}

#[tauri::command]
pub(super) async fn output_render_bytes(
    app: AppHandle,
    job_id: u64,
) -> std::result::Result<tauri::ipc::Response, String> {
    with_state(app, move |state| {
        Ok(tauri::ipc::Response::new(
            state.workspace.output_render_bytes(job_id)?,
        ))
    })
    .await
}

#[tauri::command]
pub(super) async fn stage_document_output(
    app: AppHandle,
    request: tauri::ipc::Request<'_>,
) -> std::result::Result<(), String> {
    let number = |name: &str| {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| format!("The {name} output header is missing or invalid."))
    };
    let job_id = number("x-folio-job-id")?;
    let index = usize::try_from(number("x-folio-output-index")?)
        .map_err(|_| "The output page index is invalid.")?;
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("Output pages must be sent as binary data.".into());
    };
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("A rendered page exceeds the 64 MiB image limit.".into());
    }
    let bytes = bytes.clone();
    with_state(app, move |state| {
        state.workspace.stage_output_page(job_id, index, bytes)
    })
    .await
}

#[tauri::command]
pub(super) async fn prepare_optimized_output(
    app: AppHandle,
    job_id: u64,
) -> std::result::Result<OptimizationReport, String> {
    with_state(app, move |state| {
        state.workspace.prepare_optimized_output(job_id)
    })
    .await
}

#[tauri::command]
pub(super) async fn check_document_output(
    app: AppHandle,
    job_id: u64,
) -> std::result::Result<OutputPlan, String> {
    with_state(app, move |state| state.workspace.check_output_ready(job_id)).await
}

#[tauri::command]
pub(super) async fn save_document_output(
    app: AppHandle,
    job_id: u64,
) -> std::result::Result<Option<String>, String> {
    let plan = with_state(app.clone(), move |state| {
        let plan = state.workspace.check_output_ready(job_id)?;
        if plan.kind == OutputKind::Print {
            return Err(invalid(
                "Use the print dialog for a prepared print snapshot.",
            ));
        }
        Ok(plan)
    })
    .await?;
    let extension = plan.kind.extension(plan.pages.len());
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_title(if plan.kind == OutputKind::Optimize {
            "Save a losslessly optimized PDF copy"
        } else {
            "Export a separate copy"
        })
        .add_filter("Selected output format", &[extension])
        .set_file_name(&plan.suggested_name)
        .save_file()
        .await
    else {
        return Ok(None);
    };
    let path = output_path(file.path().to_path_buf(), &plan).map_err(|error| error.to_string())?;
    with_state(app, move |state| {
        state.workspace.write_output_copy(job_id, &path)?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await
    .map(Some)
}

#[tauri::command]
pub(super) async fn print_document_output(
    app: AppHandle,
    job_id: u64,
) -> std::result::Result<(), String> {
    with_state(app.clone(), move |state| {
        if state.workspace.check_output_ready(job_id)?.kind != OutputKind::Print {
            return Err(invalid(
                "Only a fully prepared print snapshot can open the print dialog.",
            ));
        }
        Ok(())
    })
    .await?;
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        let window = app
            .get_webview_window("main")
            .ok_or_else(|| "The Folio print window is unavailable.".to_owned())?;
        window
            .print()
            .map_err(|error| format!("The platform print dialog could not be opened: {error}"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Err("Printing is not supported on this platform.".into())
    }
}

#[tauri::command]
pub(super) async fn cancel_document_output(
    app: AppHandle,
    job_id: u64,
) -> std::result::Result<bool, String> {
    with_state(app, move |state| Ok(state.workspace.cancel_output(job_id))).await
}
