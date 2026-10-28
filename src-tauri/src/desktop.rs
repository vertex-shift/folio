use std::{path::PathBuf, sync::Mutex};

use tauri::{
    AppHandle, Emitter, Manager, RunEvent, WindowEvent,
    menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu},
};

use crate::{
    document::{DocumentInfo, Edit, Error, Result, atomic_write},
    workspace::{OpenOutcome, Workspace, WorkspaceInfo},
};

#[path = "document_utilities_desktop.rs"]
mod utilities;

#[derive(Default)]
struct DesktopState {
    workspace: Workspace,
    pending_paths: Vec<PathBuf>,
    ui_ready: bool,
    allow_exit: bool,
}

type SharedState = Mutex<DesktopState>;

#[path = "desktop_output.rs"]
mod output_commands;

#[path = "desktop_conversion.rs"]
mod conversion_commands;

#[path = "forms/desktop.rs"]
mod form_commands;

#[path = "desktop_comparison.rs"]
mod comparison_commands;

#[path = "desktop_archive.rs"]
mod archive_commands;

#[path = "desktop_image_edit.rs"]
mod image_commands;

async fn with_state<T: Send + 'static>(
    app: AppHandle,
    action: impl FnOnce(&mut DesktopState) -> Result<T> + Send + 'static,
) -> std::result::Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<SharedState>();
        let mut state = state
            .lock()
            .map_err(|_| Error::Invalid("Document state is unavailable. Restart Folio.".into()))?;
        action(&mut state)
    })
    .await
    .map_err(|error| format!("The document task failed: {error}"))?
    .map_err(|error| error.to_string())
}

#[tauri::command]
async fn bootstrap(app: AppHandle) -> std::result::Result<WorkspaceInfo, String> {
    with_state(app, |state| {
        state.ui_ready = true;
        let paths = std::mem::take(&mut state.pending_paths);
        let opening = state.workspace.request_open_paths(paths);
        let mut snapshot = state.workspace.snapshot()?;
        snapshot.opening.extend(
            opening
                .into_iter()
                .filter(|outcome| matches!(outcome, OpenOutcome::Failed { .. })),
        );
        Ok(snapshot)
    })
    .await
}

#[tauri::command]
async fn open_document(app: AppHandle) -> std::result::Result<Vec<OpenOutcome>, String> {
    let files = rfd::AsyncFileDialog::new()
        .set_title("Open PDFs in new tabs")
        .add_filter("PDF document", &["pdf"])
        .pick_files()
        .await;
    let Some(files) = files else {
        return Ok(Vec::new());
    };
    let paths = files
        .into_iter()
        .map(|file| file.path().to_path_buf())
        .collect();
    with_state(app, move |state| {
        Ok(state.workspace.request_open_paths(paths))
    })
    .await
}

#[tauri::command]
async fn open_document_path(
    app: AppHandle,
    path: String,
) -> std::result::Result<OpenOutcome, String> {
    with_state(app, move |state| {
        state.workspace.request_open_path(PathBuf::from(path))
    })
    .await
}

#[tauri::command]
async fn open_document_paths(
    app: AppHandle,
    paths: Vec<String>,
) -> std::result::Result<Vec<OpenOutcome>, String> {
    with_state(app, move |state| {
        Ok(state
            .workspace
            .request_open_paths(paths.into_iter().map(PathBuf::from).collect()))
    })
    .await
}

#[tauri::command]
async fn activate_document(
    app: AppHandle,
    document_id: u64,
) -> std::result::Result<DocumentInfo, String> {
    with_state(app, move |state| state.workspace.activate(document_id)).await
}

#[tauri::command]
async fn close_document(
    app: AppHandle,
    document_id: u64,
    revision: u64,
    discard_changes: bool,
) -> std::result::Result<WorkspaceInfo, String> {
    with_state(app, move |state| {
        state
            .workspace
            .close_document(document_id, revision, discard_changes)
    })
    .await
}

#[tauri::command]
async fn unlock_document(
    app: AppHandle,
    request_id: u64,
    password: String,
) -> std::result::Result<OpenOutcome, String> {
    with_state(app, move |state| {
        state.workspace.unlock_document(request_id, &password)
    })
    .await
}

#[tauri::command]
async fn cancel_open(app: AppHandle, request_id: u64) -> std::result::Result<bool, String> {
    with_state(app, move |state| {
        Ok(state.workspace.cancel_open(request_id))
    })
    .await
}

#[tauri::command]
async fn open_example(app: AppHandle) -> std::result::Result<DocumentInfo, String> {
    with_state(app, move |state| state.workspace.open_demo()).await
}

#[tauri::command]
async fn create_document(
    app: AppHandle,
    width: f32,
    height: f32,
) -> std::result::Result<DocumentInfo, String> {
    with_state(app, move |state| {
        state.workspace.create_document(width, height)
    })
    .await
}

#[tauri::command]
async fn import_document_pages(
    app: AppHandle,
    document_id: u64,
    revision: u64,
    at: usize,
) -> std::result::Result<Option<OpenOutcome>, String> {
    with_state(app.clone(), move |state| {
        state.workspace.check_revision(document_id, revision)?;
        Ok(())
    })
    .await?;
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_title("Insert pages from a PDF")
        .add_filter("PDF document", &["pdf"])
        .pick_file()
        .await
    else {
        return Ok(None);
    };
    let path = file.path().to_path_buf();
    with_state(app, move |state| {
        state
            .workspace
            .request_import_path(document_id, revision, at, path)
    })
    .await
    .map(Some)
}

#[tauri::command]
async fn extract_document_pages(
    app: AppHandle,
    document_id: u64,
    revision: u64,
    pages: Vec<usize>,
) -> std::result::Result<Option<String>, String> {
    let (bytes, name) = with_state(app.clone(), move |state| {
        let session = state.workspace.ensure_export_ready(document_id, revision)?;
        let stem = std::path::Path::new(&session.name)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        Ok((session.extract_pages(&pages)?, format!("{stem}-pages.pdf")))
    })
    .await?;
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_title("Save extracted pages")
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
            .ensure_output_available(document_id, &path)?;
        let session = state.workspace.ensure_export_ready(document_id, revision)?;
        session.ensure_separate_output(&path)?;
        atomic_write(&path, &bytes)?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await
    .map(Some)
}

fn pdf_output_path(mut path: PathBuf) -> std::result::Result<PathBuf, String> {
    if path.extension().is_none() {
        path.set_extension("pdf");
    } else if !path
        .extension()
        .is_some_and(|value| value.eq_ignore_ascii_case("pdf"))
    {
        return Err("Use a .pdf filename for the saved document.".into());
    }
    Ok(path)
}

#[tauri::command]
async fn document_bytes(
    app: AppHandle,
    document_id: u64,
    revision: u64,
    render: bool,
) -> std::result::Result<tauri::ipc::Response, String> {
    with_state(app, move |state| {
        let session = if render {
            state.workspace.check_revision(document_id, revision)?
        } else {
            state.workspace.ensure_export_ready(document_id, revision)?
        };
        let bytes = if render {
            session.render_bytes()?
        } else {
            state.workspace.ensure_export_ready(document_id, revision)?;
            session.bytes().to_vec()
        };
        Ok(tauri::ipc::Response::new(bytes))
    })
    .await
}

#[tauri::command]
async fn edit_document(
    app: AppHandle,
    document_id: u64,
    revision: u64,
    edit: Edit,
) -> std::result::Result<DocumentInfo, String> {
    with_state(app, move |state| {
        state
            .workspace
            .current_mut(document_id, revision)?
            .apply(edit)
    })
    .await
}

#[tauri::command]
async fn page_text_runs(
    app: AppHandle,
    document_id: u64,
    revision: u64,
    page: usize,
) -> std::result::Result<Vec<crate::text_edit::TextRun>, String> {
    with_state(app, move |state| {
        state
            .workspace
            .check_revision(document_id, revision)?
            .text_runs(page)
    })
    .await
}

#[tauri::command]
async fn inspect_signatures(
    app: AppHandle,
    document_id: u64,
    revision: u64,
) -> std::result::Result<crate::signatures::SignatureReport, String> {
    with_state(app, move |state| {
        state.workspace.signature_report(document_id, revision)
    })
    .await
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SigningIdentityFile {
    name: String,
    path: String,
}

#[tauri::command]
async fn choose_signing_identity(
    app: AppHandle,
    document_id: u64,
    revision: u64,
) -> std::result::Result<Option<SigningIdentityFile>, String> {
    with_state(app.clone(), move |state| {
        state.workspace.check_signing(document_id, revision)?;
        Ok(())
    })
    .await?;
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_title("Choose a local signing identity")
        .add_filter("PKCS#12 signing identity", &["p12", "pfx"])
        .pick_file()
        .await
    else {
        return Ok(None);
    };
    with_state(app, move |state| {
        state.workspace.check_signing(document_id, revision)?;
        Ok(Some(SigningIdentityFile {
            name: file.file_name(),
            path: file.path().to_string_lossy().into_owned(),
        }))
    })
    .await
}

#[tauri::command]
async fn export_signed_copy(
    app: AppHandle,
    document_id: u64,
    revision: u64,
    identity_path: String,
    password: String,
    field_id: Option<String>,
) -> std::result::Result<Option<String>, String> {
    let password = zeroize::Zeroizing::new(password);
    let name = with_state(app.clone(), move |state| {
        let session = state.workspace.check_signing(document_id, revision)?;
        let stem = std::path::Path::new(&session.name)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        Ok(format!("{stem}-signed.pdf"))
    })
    .await?;
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_title("Export a cryptographically signed PDF copy")
        .add_filter("PDF document", &["pdf"])
        .set_file_name(name)
        .save_file()
        .await
    else {
        return Ok(None);
    };
    let path = pdf_output_path(file.path().to_path_buf())?;
    with_state(app, move |state| {
        state.workspace.check_signing(document_id, revision)?;
        let identity = crate::signatures::read_identity_file(std::path::Path::new(&identity_path))?;
        state.workspace.export_signed_copy_in_field(
            document_id,
            revision,
            &identity,
            &password,
            &path,
            field_id.as_deref(),
        )?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
}

#[tauri::command]
async fn choose_signature_anchor(
    app: AppHandle,
    document_id: u64,
    revision: u64,
    trust_revision: u64,
) -> std::result::Result<Option<crate::signatures::TrustStoreInfo>, String> {
    with_state(app.clone(), move |state| {
        state.workspace.check_revision(document_id, revision)?;
        if state.workspace.signature_trust_store()?.revision != trust_revision {
            return Err(crate::document::invalid(
                "The custom CA store changed. Refresh signature checks.",
            ));
        }
        Ok(())
    })
    .await?;
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_title("Add a public root CA to your custom signature store (this run only)")
        .add_filter("Public CA certificate", &["pem", "der", "cer", "crt"])
        .pick_file()
        .await
    else {
        return Ok(None);
    };
    with_state(app, move |state| {
        state.workspace.check_revision(document_id, revision)?;
        let bytes = crate::signatures::read_anchor_file(file.path())?;
        Ok(Some(state.workspace.add_signature_anchor(
            document_id,
            revision,
            trust_revision,
            &bytes,
        )?))
    })
    .await
}

#[tauri::command]
async fn remove_signature_anchor(
    app: AppHandle,
    document_id: u64,
    revision: u64,
    trust_revision: u64,
    fingerprint: String,
) -> std::result::Result<crate::signatures::TrustStoreInfo, String> {
    with_state(app, move |state| {
        state
            .workspace
            .remove_signature_anchor(document_id, revision, trust_revision, &fingerprint)
    })
    .await
}

#[tauri::command]
async fn form_fields(
    app: AppHandle,
    document_id: u64,
    revision: u64,
) -> std::result::Result<crate::forms::FormInfo, String> {
    with_state(app, move |state| {
        state
            .workspace
            .check_revision(document_id, revision)?
            .form_fields()
    })
    .await
}

#[tauri::command]
async fn fill_form_fields(
    app: AppHandle,
    document_id: u64,
    revision: u64,
    changes: Vec<crate::forms::FieldChange>,
) -> std::result::Result<DocumentInfo, String> {
    with_state(app, move |state| {
        state
            .workspace
            .current_mut(document_id, revision)?
            .fill_form_fields(changes)
    })
    .await
}

#[tauri::command]
async fn stage_authoring_image(
    app: AppHandle,
    request: tauri::ipc::Request<'_>,
) -> std::result::Result<crate::authoring::ImageAsset, String> {
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
        return Err("Send the user-selected PNG/JPEG as binary image data.".into());
    };
    if bytes.len() > crate::authoring::MAX_IMAGE_BYTES {
        return Err("Choose a PNG or JPEG no larger than 16 MiB.".into());
    }
    let bytes = bytes.clone();
    with_state(app, move |state| {
        state
            .workspace
            .stage_authoring_image(document_id, revision, &bytes)
    })
    .await
}

#[tauri::command]
async fn insert_authoring_image(
    app: AppHandle,
    document_id: u64,
    revision: u64,
    asset_id: u64,
    placement: crate::authoring::ImagePlacement,
) -> std::result::Result<DocumentInfo, String> {
    with_state(app, move |state| {
        state
            .workspace
            .insert_authoring_image(document_id, revision, asset_id, placement)
    })
    .await
}

#[tauri::command]
async fn cancel_authoring_image(
    app: AppHandle,
    document_id: u64,
    asset_id: u64,
) -> std::result::Result<bool, String> {
    with_state(app, move |state| {
        state
            .workspace
            .cancel_authoring_image(document_id, asset_id)
    })
    .await
}

#[tauri::command]
async fn begin_ocr(
    app: AppHandle,
    document_id: u64,
    revision: u64,
    options: crate::ocr::Options,
) -> std::result::Result<crate::ocr::Plan, String> {
    with_state(app, move |state| {
        state.workspace.begin_ocr(document_id, revision, options)
    })
    .await
}

#[tauri::command]
async fn stage_ocr_page(
    app: AppHandle,
    job_id: u64,
    result: crate::ocr::PageResult,
) -> std::result::Result<(), String> {
    with_state(app, move |state| {
        state.workspace.stage_ocr_page(job_id, result)
    })
    .await
}

#[tauri::command]
async fn commit_ocr(
    app: AppHandle,
    job_id: u64,
) -> std::result::Result<crate::ocr::Outcome, String> {
    with_state(app, move |state| state.workspace.commit_ocr(job_id)).await
}

#[tauri::command]
async fn cancel_ocr(app: AppHandle, job_id: u64) -> std::result::Result<bool, String> {
    with_state(app, move |state| Ok(state.workspace.cancel_ocr(job_id))).await
}

#[tauri::command]
async fn begin_redaction(
    app: AppHandle,
    document_id: u64,
    revision: u64,
    dpi: u16,
    regions: Vec<crate::redaction::Region>,
) -> std::result::Result<crate::redaction::Plan, String> {
    with_state(app, move |state| {
        state
            .workspace
            .begin_redaction(document_id, revision, dpi, &regions)
    })
    .await
}

#[tauri::command]
async fn stage_redaction_page(
    app: AppHandle,
    request: tauri::ipc::Request<'_>,
) -> std::result::Result<(), String> {
    let number = |name: &str| {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| format!("The {name} redaction header is missing or invalid."))
    };
    let job_id = number("x-folio-job-id")?;
    let page = usize::try_from(number("x-folio-page")?)
        .map_err(|_| "The redaction page index is invalid.")?;
    let tauri::ipc::InvokeBody::Raw(png) = request.body() else {
        return Err("A redaction page must be sent as binary PNG data.".into());
    };
    if png.len() > crate::redaction::MAX_PAGE_PNG_BYTES {
        return Err("A rendered page exceeds the 64 MiB PNG limit.".into());
    }
    let png = png.clone();
    with_state(app, move |state| {
        state.workspace.stage_redaction_page(job_id, page, &png)
    })
    .await
}

#[tauri::command]
async fn export_redacted_copy(
    app: AppHandle,
    job_id: u64,
) -> std::result::Result<Option<String>, String> {
    let bytes = with_state(app.clone(), move |state| {
        state.workspace.redaction_bytes(job_id)
    })
    .await?;
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_title("Export a permanently redacted PDF copy")
        .add_filter("PDF document", &["pdf"])
        .set_file_name("Redacted.pdf")
        .save_file()
        .await
    else {
        return Ok(None);
    };
    let path = pdf_output_path(file.path().to_path_buf())?;
    with_state(app, move |state| {
        let session = state.workspace.check_redaction(job_id)?;
        session.ensure_separate_output(&path)?;
        state.workspace.ensure_output_available(session.id, &path)?;
        atomic_write(&path, &bytes)?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await
    .map(Some)
}

#[tauri::command]
async fn cancel_redaction(app: AppHandle, job_id: u64) -> std::result::Result<bool, String> {
    with_state(app, move |state| {
        Ok(state.workspace.cancel_redaction(job_id))
    })
    .await
}

#[tauri::command]
async fn undo_document(
    app: AppHandle,
    document_id: u64,
    revision: u64,
) -> std::result::Result<DocumentInfo, String> {
    with_state(app, move |state| {
        state.workspace.current_mut(document_id, revision)?.undo()
    })
    .await
}

#[tauri::command]
async fn redo_document(
    app: AppHandle,
    document_id: u64,
    revision: u64,
) -> std::result::Result<DocumentInfo, String> {
    with_state(app, move |state| {
        state.workspace.current_mut(document_id, revision)?.redo()
    })
    .await
}

#[tauri::command]
async fn save_document(
    app: AppHandle,
    document_id: u64,
    revision: u64,
    save_as: bool,
) -> std::result::Result<Option<DocumentInfo>, String> {
    let (bytes, suggested_name, directory, previous_path) = with_state(app.clone(), move |state| {
        let session = state.workspace.ensure_export_ready(document_id, revision)?;
        let directory = session
            .saved_path
            .as_ref()
            .or(session.source_path.as_ref())
            .and_then(|path| path.parent())
            .map(|path| path.to_path_buf());
        Ok((
            session.bytes().to_vec(),
            session.suggested_name(),
            directory,
            session.saved_path.clone(),
        ))
    })
    .await?;

    let path = if !save_as && previous_path.is_some() {
        previous_path
    } else {
        let mut dialog = rfd::AsyncFileDialog::new()
            .set_title("Save an edited PDF copy")
            .add_filter("PDF document", &["pdf"])
            .set_file_name(&suggested_name);
        if let Some(directory) = directory {
            dialog = dialog.set_directory(directory);
        }
        dialog
            .save_file()
            .await
            .map(|file| file.path().to_path_buf())
    };
    let Some(path) = path else {
        return Ok(None);
    };
    let path = pdf_output_path(path)?;
    with_state(app, move |state| {
        state
            .workspace
            .ensure_output_available(document_id, &path)?;
        state.workspace.ensure_export_ready(document_id, revision)?;
        let session = state.workspace.current_mut(document_id, revision)?;
        atomic_write(&path, &bytes)?;
        session.record_saved(path, revision);
        session.info()
    })
    .await
    .map(Some)
}

#[tauri::command]
fn close_application(app: AppHandle, discard_changes: bool) -> std::result::Result<(), String> {
    {
        let state = app.state::<SharedState>();
        let mut state = state
            .lock()
            .map_err(|_| "Document state is unavailable. Restart Folio.")?;
        if state.workspace.has_unsaved_changes() && !discard_changes {
            return Err("There are unsaved PDF tabs or pending drafts. Review them or explicitly discard all changes before quitting.".into());
        }
        state.allow_exit = true;
    }
    app.exit(0);
    Ok(())
}

#[tauri::command]
async fn set_document_draft(
    app: AppHandle,
    document_id: u64,
    pending: bool,
) -> std::result::Result<(), String> {
    with_state(app, move |state| {
        state.workspace.set_draft(document_id, pending)
    })
    .await
}

fn protect_exit(app: &AppHandle) -> bool {
    let state = app.state::<SharedState>();
    match state.try_lock() {
        Ok(state) => !state.allow_exit && state.workspace.has_unsaved_changes(),
        Err(_) => true,
    }
}

fn emit(app: &AppHandle, event: &str, value: &str) {
    if let Err(error) = app.emit(event, value) {
        eprintln!("Could not deliver {event}: {error}");
    }
}

fn menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let app_menu = Submenu::with_items(
        app,
        "Folio",
        true,
        &[
            &PredefinedMenuItem::about(
                app,
                Some("About Folio"),
                Some(AboutMetadata {
                    name: Some("Folio".into()),
                    version: Some(env!("CARGO_PKG_VERSION").into()),
                    comments: Some("An independent, local-first PDF workspace.".into()),
                    ..Default::default()
                }),
            )?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;
    let file_menu = Submenu::with_items(
        app,
        "File",
        true,
        &[
            &MenuItem::with_id(app, "new", "New PDF...", true, Some("CmdOrCtrl+N"))?,
            &MenuItem::with_id(
                app,
                "create-files",
                "Create PDF from Files...",
                true,
                Some("CmdOrCtrl+Shift+N"),
            )?,
            &MenuItem::with_id(app, "open", "Open PDF...", true, Some("CmdOrCtrl+O"))?,
            &MenuItem::with_id(app, "example", "Open Example Document", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "save", "Save", true, Some("CmdOrCtrl+S"))?,
            &MenuItem::with_id(
                app,
                "save-copy",
                "Save a Copy...",
                true,
                Some("CmdOrCtrl+Shift+S"),
            )?,
            &MenuItem::with_id(app, "print", "Print...", true, Some("CmdOrCtrl+P"))?,
            &MenuItem::with_id(
                app,
                "export",
                "Export Images or Text...",
                true,
                None::<&str>,
            )?,
            &MenuItem::with_id(app, "optimize", "Optimize PDF Copy...", true, None::<&str>)?,
            &MenuItem::with_id(
                app,
                "preflight",
                "Technical Preflight...",
                true,
                None::<&str>,
            )?,
            &MenuItem::with_id(
                app,
                "archive",
                "Export PDF/A-2b Archival Copy...",
                true,
                None::<&str>,
            )?,
            &MenuItem::with_id(
                app,
                "export-docx",
                "Export Word Document Content...",
                true,
                None::<&str>,
            )?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "close-tab", "Close PDF", true, Some("CmdOrCtrl+W"))?,
        ],
    )?;
    let edit_menu = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &MenuItem::with_id(app, "undo", "Undo", true, Some("CmdOrCtrl+Z"))?,
            &MenuItem::with_id(app, "redo", "Redo", true, Some("CmdOrCtrl+Shift+Z"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "find", "Find in Document", true, Some("CmdOrCtrl+F"))?,
        ],
    )?;
    let window_menu = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
        ],
    )?;
    Menu::with_items(app, &[&app_menu, &file_menu, &edit_menu, &window_menu])
}

pub fn run() {
    let pending_paths = std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .filter(|path| {
            path.extension()
                .is_some_and(|value| value.eq_ignore_ascii_case("pdf"))
        })
        .collect();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Mutex::new(DesktopState {
            pending_paths,
            ..Default::default()
        }))
        .setup(|app| {
            app.set_menu(menu(app.handle())?)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            utilities::document_metadata,
            utilities::document_bookmarks,
            utilities::document_attachments,
            utilities::add_document_attachment,
            utilities::export_document_attachment,
            utilities::export_protection_copy,
            create_document,
            conversion_commands::begin_creation,
            conversion_commands::choose_creation_files,
            conversion_commands::prepare_creation,
            conversion_commands::creation_status,
            conversion_commands::commit_creation,
            conversion_commands::cancel_creation,
            import_document_pages,
            extract_document_pages,
            bootstrap,
            open_document,
            open_document_path,
            open_document_paths,
            activate_document,
            close_document,
            unlock_document,
            cancel_open,
            open_example,
            document_bytes,
            edit_document,
            page_text_runs,
            image_commands::page_image_occurrences,
            image_commands::stage_image_replacement,
            image_commands::edit_image_occurrence,
            image_commands::cancel_image_replacement,
            inspect_signatures,
            choose_signing_identity,
            export_signed_copy,
            choose_signature_anchor,
            remove_signature_anchor,
            form_fields,
            fill_form_fields,
            form_commands::form_designer,
            form_commands::design_form_fields,
            form_commands::inspect_flattened_form_copy,
            form_commands::export_flattened_form_copy,
            stage_authoring_image,
            insert_authoring_image,
            cancel_authoring_image,
            begin_ocr,
            stage_ocr_page,
            commit_ocr,
            cancel_ocr,
            begin_redaction,
            stage_redaction_page,
            export_redacted_copy,
            cancel_redaction,
            undo_document,
            redo_document,
            save_document,
            close_application,
            set_document_draft,
            output_commands::begin_document_output,
            output_commands::output_render_bytes,
            output_commands::stage_document_output,
            output_commands::prepare_optimized_output,
            output_commands::check_document_output,
            output_commands::save_document_output,
            output_commands::print_document_output,
            output_commands::cancel_document_output,
            comparison_commands::begin_comparison,
            comparison_commands::check_comparison,
            comparison_commands::comparison_render_bytes,
            comparison_commands::finish_comparison,
            comparison_commands::save_comparison_report,
            comparison_commands::cancel_comparison,
            archive_commands::inspect_technical,
            archive_commands::check_inspection,
            archive_commands::cancel_inspection,
            archive_commands::save_inspection_report,
            archive_commands::begin_archive,
            archive_commands::check_archive,
            archive_commands::archive_render_bytes,
            archive_commands::stage_archive_page,
            archive_commands::save_archive_copy,
            archive_commands::cancel_archive,
        ])
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            if [
                "new",
                "create-files",
                "open",
                "example",
                "save",
                "save-copy",
                "undo",
                "redo",
                "find",
                "close-tab",
                "print",
                "export",
                "export-docx",
                "optimize",
            ]
            .contains(&id)
            {
                emit(app, "menu-action", id);
            }
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if protect_exit(window.app_handle()) {
                    api.prevent_close();
                    emit(window.app_handle(), "request-close", "");
                } else {
                    window.app_handle().exit(0);
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("Folio could not initialize its desktop window");

    app.run(|app, event| match event {
        RunEvent::ExitRequested { api, .. } if protect_exit(app) => {
            api.prevent_exit();
            emit(app, "request-close", "");
        }
        #[cfg(target_os = "macos")]
        RunEvent::Opened { urls } => {
            let paths: Vec<_> = urls
                .into_iter()
                .filter_map(|url| match url.to_file_path() {
                    Ok(path) => Some(path),
                    Err(_) => {
                        eprintln!("Folio can open local file URLs only.");
                        None
                    }
                })
                .collect();
            let state = app.state::<SharedState>();
            match state.lock() {
                Ok(mut state) if !state.ui_ready => {
                    for path in paths {
                        if !state.pending_paths.contains(&path) {
                            state.pending_paths.push(path);
                        }
                    }
                }
                Ok(state) => {
                    drop(state);
                    let paths: Vec<_> = paths
                        .into_iter()
                        .map(|path| path.to_string_lossy().into_owned())
                        .collect();
                    if let Err(error) = app.emit("open-paths", paths) {
                        eprintln!("Could not deliver PDF opening request: {error}");
                    }
                }
                Err(error) => eprintln!("Could not open the requested PDFs: {error}"),
            }
        }
        _ => {}
    });
}
