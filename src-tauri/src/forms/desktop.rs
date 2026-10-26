use super::{pdf_output_path, with_state};
use crate::{
    document::DocumentInfo,
    forms::{designer, flatten},
};
use tauri::AppHandle;

#[tauri::command]
pub async fn form_designer(
    app: AppHandle,
    document_id: u64,
    revision: u64,
) -> Result<designer::DesignerInfo, String> {
    with_state(app, move |state| {
        state
            .workspace
            .check_revision(document_id, revision)?
            .form_designer()
    })
    .await
}

#[tauri::command]
pub async fn design_form_fields(
    app: AppHandle,
    document_id: u64,
    revision: u64,
    changes: Vec<designer::Change>,
) -> Result<DocumentInfo, String> {
    with_state(app, move |state| {
        state
            .workspace
            .current_mut(document_id, revision)?
            .design_form_fields(changes)
    })
    .await
}

#[tauri::command]
pub async fn inspect_flattened_form_copy(
    app: AppHandle,
    document_id: u64,
    revision: u64,
) -> Result<flatten::FlattenInfo, String> {
    with_state(app, move |state| {
        state
            .workspace
            .check_form_flattening(document_id, revision)?
            .flattened_form_info()
    })
    .await
}

#[tauri::command]
pub async fn export_flattened_form_copy(
    app: AppHandle,
    document_id: u64,
    revision: u64,
) -> Result<Option<String>, String> {
    let name = with_state(app.clone(), move |state| {
        let session = state
            .workspace
            .check_form_flattening(document_id, revision)?;
        session.flattened_form_info()?;
        Ok(format!(
            "{}-static.pdf",
            session.name.strip_suffix(".pdf").unwrap_or(&session.name)
        ))
    })
    .await?;
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_title("Export separate static form copy")
        .add_filter("PDF document", &["pdf"])
        .set_file_name(&name)
        .save_file()
        .await
    else {
        return Ok(None);
    };
    let path = pdf_output_path(file.path().to_path_buf())?;
    with_state(app, move |state| {
        state
            .workspace
            .export_flattened_form_copy(document_id, revision, &path)?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await
    .map(Some)
}
