use super::with_state;
use crate::{
    document::DocumentInfo,
    paragraph::{ParagraphChange, ParagraphPage, ParagraphPreview},
};
use tauri::AppHandle;

#[tauri::command]
pub async fn page_paragraph_blocks(
    app: AppHandle,
    document_id: u64,
    revision: u64,
    page: usize,
) -> Result<ParagraphPage, String> {
    with_state(app, move |state| {
        state
            .workspace
            .check_revision(document_id, revision)?
            .paragraph_blocks(page)
    })
    .await
}

#[tauri::command]
pub async fn preview_paragraph(
    app: AppHandle,
    document_id: u64,
    revision: u64,
    change: ParagraphChange,
) -> Result<ParagraphPreview, String> {
    with_state(app, move |state| {
        state
            .workspace
            .check_revision(document_id, revision)?
            .paragraph_preview(&change)
    })
    .await
}

#[tauri::command]
pub async fn edit_paragraph(
    app: AppHandle,
    document_id: u64,
    revision: u64,
    change: ParagraphChange,
) -> Result<DocumentInfo, String> {
    with_state(app, move |state| {
        state
            .workspace
            .edit_paragraph(document_id, revision, change)
    })
    .await
}
