use super::with_state;
use crate::{
    authoring::ImageAsset,
    document::DocumentInfo,
    image_edit::{ImageChange, ImagePage},
};
use tauri::AppHandle;

#[tauri::command]
pub async fn page_image_occurrences(
    app: AppHandle,
    document_id: u64,
    revision: u64,
    page: usize,
) -> Result<ImagePage, String> {
    with_state(app, move |state| {
        state
            .workspace
            .check_revision(document_id, revision)?
            .image_occurrences(page)
    })
    .await
}

#[tauri::command]
pub async fn stage_image_replacement(
    app: AppHandle,
    request: tauri::ipc::Request<'_>,
) -> Result<ImageAsset, String> {
    let number = |name: &str| {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| format!("The {name} image header is missing or invalid."))
    };
    let document_id = number("x-document-id")?;
    let revision = number("x-revision")?;
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err(
            "Send the user-selected PNG/JPEG as binary image data, not a filesystem path.".into(),
        );
    };
    if bytes.len() > crate::authoring::MAX_IMAGE_BYTES {
        return Err("Choose a PNG or JPEG no larger than 16 MiB.".into());
    }
    let bytes = bytes.clone();
    with_state(app, move |state| {
        state
            .workspace
            .stage_image_replacement(document_id, revision, &bytes)
    })
    .await
}

#[tauri::command]
pub async fn edit_image_occurrence(
    app: AppHandle,
    document_id: u64,
    revision: u64,
    change: ImageChange,
) -> Result<DocumentInfo, String> {
    with_state(app, move |state| {
        state
            .workspace
            .edit_image_occurrence(document_id, revision, change)
    })
    .await
}

#[tauri::command]
pub async fn cancel_image_replacement(
    app: AppHandle,
    document_id: u64,
    asset_id: u64,
) -> Result<bool, String> {
    with_state(app, move |state| {
        state
            .workspace
            .cancel_image_replacement(document_id, asset_id)
    })
    .await
}
