use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::updates::{Lifecycle, Status};

pub(super) struct Package {
    update: Update,
    bytes: Vec<u8>,
}

pub(super) type State = Mutex<Lifecycle<Package>>;

fn publish(app: &AppHandle) {
    match app.state::<State>().lock() {
        Ok(state) => {
            if let Err(error) = app.emit("folio-update", &state.status) {
                eprintln!("Could not deliver update status: {error}");
            }
        }
        Err(error) => eprintln!("Update state is unavailable: {error}"),
    }
}

#[tauri::command]
pub(super) fn update_status(app: AppHandle) -> Result<Status, String> {
    app.state::<State>()
        .lock()
        .map(|state| state.status.clone())
        .map_err(|_| "Update state is unavailable. Restart Folio.".into())
}

#[tauri::command]
pub(super) fn check_for_updates(app: AppHandle) -> Result<(), String> {
    let started = app
        .state::<State>()
        .lock()
        .map_err(|_| "Update state is unavailable. Restart Folio.")?
        .begin_check();
    if started {
        publish(&app);
        tauri::async_runtime::spawn(async move {
            let result = prepare(&app).await;
            match app.state::<State>().lock() {
                Ok(mut state) => match result {
                    Ok(package) => state.finish(package),
                    Err(error) => {
                        eprintln!("Folio update check/download failed: {error}");
                        state.fail(format!(
                            "Could not prepare an update: {error}. Your current app is unchanged. Check again when connected."
                        ));
                    }
                },
                Err(error) => eprintln!("Update state is unavailable: {error}"),
            }
            publish(&app);
        });
    }
    Ok(())
}

async fn prepare(app: &AppHandle) -> Result<Option<Package>, String> {
    let updater = app
        .updater_builder()
        .timeout(Duration::from_secs(30))
        .restart_after_install(false)
        .build()
        .map_err(|error| error.to_string())?;
    let Some(mut update) = updater.check().await.map_err(|error| error.to_string())? else {
        return Ok(None);
    };
    if update.download_url.scheme() != "https"
        || update.download_url.host_str() != Some("github.com")
        || !update
            .download_url
            .path()
            .starts_with("/vertex-shift/folio/releases/download/")
    {
        return Err("The release contains an unexpected download location".into());
    }
    app.state::<State>()
        .lock()
        .map_err(|_| "Update state is unavailable.")?
        .downloading(update.version.clone());
    publish(app);
    update.timeout = Some(Duration::from_secs(15 * 60));
    let too_large = Arc::new(tokio::sync::Notify::new());
    let limit_signal = too_large.clone();
    let mut received = 0u64;
    let mut last_progress = Instant::now();
    let download = update.download(
        |chunk, total| {
            received = received.saturating_add(chunk as u64);
            if received > 512 * 1024 * 1024 || total.is_some_and(|size| size > 512 * 1024 * 1024) {
                limit_signal.notify_one();
            }
            if last_progress.elapsed() >= Duration::from_millis(500) {
                if let Ok(mut state) = app.state::<State>().lock() {
                    state.progress(received, total);
                }
                publish(app);
                last_progress = Instant::now();
            }
        },
        || {},
    );
    let bytes = tokio::select! {
        biased;
        _ = too_large.notified() => return Err("The update exceeds the 512 MiB download limit".into()),
        result = tokio::time::timeout(Duration::from_secs(15 * 60), download) => {
            result.map_err(|_| "The update download timed out")?.map_err(|error| error.to_string())?
        }
    };
    if bytes.len() > 512 * 1024 * 1024 {
        return Err("The update exceeds the 512 MiB download limit".into());
    }
    // download() verifies both the signature and its version binding before returning.
    Ok(Some(Package { update, bytes }))
}

pub(super) fn initialize(app: &AppHandle) {
    let unavailable = if cfg!(debug_assertions) {
        Some("Automatic installation is disabled in development builds.")
    } else if cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_none() {
        Some(
            "This Linux installation is managed by your package manager. Use an AppImage for automatic updates.",
        )
    } else if !(cfg!(all(target_os = "windows", target_arch = "x86_64"))
        || cfg!(all(target_os = "macos", target_arch = "aarch64"))
        || cfg!(all(target_os = "linux", target_arch = "x86_64")))
    {
        Some("Automatic updates are not published for this architecture.")
    } else {
        None
    };
    if let Some(message) = unavailable {
        if let Ok(mut state) = app.state::<State>().lock() {
            state.unavailable(message);
        }
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(10)).await;
        loop {
            if let Err(error) = check_for_updates(app.clone()) {
                eprintln!("Could not start update check: {error}");
            }
            tokio::time::sleep(Duration::from_secs(6 * 60 * 60)).await;
        }
    });
}

pub(super) async fn install_on_close(app: &AppHandle) -> Result<(), String> {
    let package = app
        .state::<State>()
        .lock()
        .map_err(|_| "Update state is unavailable. Restart Folio.")?
        .begin_close();
    if let Some(package) = package {
        publish(app);
        let result =
            tauri::async_runtime::spawn_blocking(move || package.update.install(&package.bytes))
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result.map_err(|error| error.to_string()));
        if let Err(error) = result {
            let message = format!(
                "The update could not be installed: {error}. Folio is still open. Quit again to skip this update, or check again to retry."
            );
            if let Ok(mut state) = app.state::<State>().lock() {
                state.close_failed(message.clone());
            }
            publish(app);
            return Err(message);
        }
    }
    Ok(())
}
