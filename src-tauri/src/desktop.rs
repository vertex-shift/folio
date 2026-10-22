use std::{path::PathBuf, sync::Mutex};

use tauri::{
    AppHandle, Emitter, Manager, RunEvent, WindowEvent,
    menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu},
};

use crate::{
    document::{DocumentInfo, Edit, Error, Result, atomic_write},
    workspace::Workspace,
};

#[derive(Default)]
struct DesktopState {
    workspace: Workspace,
    pending_path: Option<PathBuf>,
    ui_ready: bool,
    allow_exit: bool,
}

type SharedState = Mutex<DesktopState>;

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
async fn bootstrap(app: AppHandle) -> std::result::Result<DocumentInfo, String> {
    with_state(app, |state| {
        state.ui_ready = true;
        match state.pending_path.take() {
            Some(path) => state.workspace.open_path(path, false),
            None => state.workspace.bootstrap(),
        }
    })
    .await
}

#[tauri::command]
async fn open_document(
    app: AppHandle,
    discard_changes: bool,
) -> std::result::Result<Option<DocumentInfo>, String> {
    let file = rfd::AsyncFileDialog::new()
        .set_title("Open PDF")
        .add_filter("PDF document", &["pdf"])
        .pick_file()
        .await;
    let Some(file) = file else {
        return Ok(None);
    };
    let path = file.path().to_path_buf();
    with_state(app, move |state| {
        state.workspace.open_path(path, discard_changes)
    })
    .await
    .map(Some)
}

#[tauri::command]
async fn open_document_path(
    app: AppHandle,
    path: String,
    discard_changes: bool,
) -> std::result::Result<DocumentInfo, String> {
    with_state(app, move |state| {
        state
            .workspace
            .open_path(PathBuf::from(path), discard_changes)
    })
    .await
}

#[tauri::command]
async fn open_example(
    app: AppHandle,
    discard_changes: bool,
) -> std::result::Result<DocumentInfo, String> {
    with_state(app, move |state| state.workspace.open_demo(discard_changes)).await
}

#[tauri::command]
async fn document_bytes(
    app: AppHandle,
    document_id: u64,
    revision: u64,
) -> std::result::Result<tauri::ipc::Response, String> {
    with_state(app, move |state| {
        Ok(tauri::ipc::Response::new(
            state
                .workspace
                .check_revision(document_id, revision)?
                .bytes()
                .to_vec(),
        ))
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
        let session = state.workspace.check_revision(document_id, revision)?;
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
    let Some(mut path) = path else {
        return Ok(None);
    };
    if path.extension().is_none() {
        path.set_extension("pdf");
    } else if !path
        .extension()
        .is_some_and(|value| value.eq_ignore_ascii_case("pdf"))
    {
        return Err("Use a .pdf filename for the saved document.".into());
    }
    with_state(app, move |state| {
        let session = state.workspace.current_mut(document_id, revision)?;
        atomic_write(&path, &bytes)?;
        session.record_saved(path, revision);
        session.info()
    })
    .await
    .map(Some)
}

#[tauri::command]
fn close_application(app: AppHandle) -> std::result::Result<(), String> {
    {
        let state = app.state::<SharedState>();
        let mut state = state
            .lock()
            .map_err(|_| "Document state is unavailable. Restart Folio.")?;
        state.allow_exit = true;
    }
    app.exit(0);
    Ok(())
}

fn protect_exit(app: &AppHandle) -> bool {
    let state = app.state::<SharedState>();
    match state.try_lock() {
        Ok(state) => !state.allow_exit && state.workspace.is_dirty(),
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
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
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
    let pending_path = std::env::args_os().skip(1).map(PathBuf::from).find(|path| {
        path.extension()
            .is_some_and(|value| value.eq_ignore_ascii_case("pdf"))
    });
    let app = tauri::Builder::default()
        .manage(Mutex::new(DesktopState {
            pending_path,
            ..Default::default()
        }))
        .setup(|app| {
            app.set_menu(menu(app.handle())?)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            open_document,
            open_document_path,
            open_example,
            document_bytes,
            edit_document,
            undo_document,
            redo_document,
            save_document,
            close_application,
        ])
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            if [
                "open",
                "example",
                "save",
                "save-copy",
                "undo",
                "redo",
                "find",
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
            for url in urls.into_iter().take(1) {
                let Ok(path) = url.to_file_path() else {
                    eprintln!("Folio can open local file URLs only.");
                    continue;
                };
                let state = app.state::<SharedState>();
                match state.lock() {
                    Ok(mut state) if !state.ui_ready => state.pending_path = Some(path),
                    Ok(state) => {
                        drop(state);
                        emit(app, "open-path", &path.to_string_lossy());
                    }
                    Err(error) => eprintln!("Could not open the requested PDF: {error}"),
                }
            }
        }
        _ => {}
    });
}
