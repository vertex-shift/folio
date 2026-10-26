use std::io::Read;

use tauri::AppHandle;

use super::with_state;
use crate::{
    conversion::{
        CreationKind, CreationPlan, CreationSettings, CreationStatus, MAX_CREATION_FILES,
    },
    document::{DocumentInfo, Result, invalid},
};

#[tauri::command]
pub(super) async fn begin_creation(
    app: AppHandle,
    kind: CreationKind,
) -> std::result::Result<CreationPlan, String> {
    with_state(app, move |state| state.workspace.begin_creation(kind)).await
}

#[tauri::command]
pub(super) async fn choose_creation_files(
    app: AppHandle,
    job_id: u64,
) -> std::result::Result<Option<CreationPlan>, String> {
    let plan = with_state(app.clone(), move |state| {
        state.workspace.creation_plan(job_id)
    })
    .await?;
    let picker = rfd::AsyncFileDialog::new().set_title("Choose files for a new PDF");
    let picker = match plan.kind {
        CreationKind::Images => picker.add_filter("Still PNG or JPEG", &["png", "jpg", "jpeg"]),
        CreationKind::Text => picker.add_filter("UTF-8 text", &["txt", "text"]),
    };
    let Some(files) = picker.pick_files().await else {
        with_state(app, move |state| state.workspace.cancel_creation(job_id)).await?;
        return Ok(None);
    };
    with_state(app, move |state| {
        let result = (|| -> Result<CreationPlan> {
            let plan = state.workspace.creation_plan(job_id)?;
            if !plan.inputs.is_empty() || files.len() > MAX_CREATION_FILES {
                return Err(invalid(
                    "Choose a fresh batch of no more than 50 local files.",
                ));
            }
            let mut current = plan;
            for file in files {
                state.workspace.creation_plan(job_id)?;
                let path = file.path();
                let name = path
                    .file_name()
                    .ok_or_else(|| invalid("The selected file has no filename."))?
                    .to_string_lossy()
                    .into_owned();
                let mut input = std::fs::File::open(path)?;
                let metadata = input.metadata()?;
                let limit = current.kind.file_limit();
                if !metadata.is_file() || metadata.len() > limit as u64 {
                    return Err(invalid(
                        "The chosen file is not a regular file within the creation size limit.",
                    ));
                }
                let mut bytes = Vec::new();
                (&mut input)
                    .take(limit as u64 + 1)
                    .read_to_end(&mut bytes)?;
                current = state.workspace.stage_creation_file(job_id, name, bytes)?;
            }
            Ok(current)
        })();
        if result.is_err() {
            state.workspace.cancel_creation(job_id)?;
        }
        result
    })
    .await
    .map(Some)
}

#[tauri::command]
pub(super) async fn prepare_creation(
    app: AppHandle,
    job_id: u64,
    settings: CreationSettings,
    order: Vec<usize>,
) -> std::result::Result<CreationStatus, String> {
    with_state(app, move |state| {
        state.workspace.prepare_creation(job_id, settings, order)
    })
    .await
}

#[tauri::command]
pub(super) async fn creation_status(
    app: AppHandle,
    job_id: u64,
) -> std::result::Result<CreationStatus, String> {
    with_state(app, move |state| state.workspace.creation_status(job_id)).await
}

#[tauri::command]
pub(super) async fn commit_creation(
    app: AppHandle,
    job_id: u64,
) -> std::result::Result<DocumentInfo, String> {
    with_state(app, move |state| state.workspace.commit_creation(job_id)).await
}

#[tauri::command]
pub(super) async fn cancel_creation(
    app: AppHandle,
    job_id: u64,
) -> std::result::Result<bool, String> {
    with_state(app, move |state| state.workspace.cancel_creation(job_id)).await
}
