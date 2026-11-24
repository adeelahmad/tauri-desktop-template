//! Auto-update from GitHub releases (Tauri updater).
//!
//! The release workflow signs the update bundles with `TAURI_SIGNING_PRIVATE_KEY` and
//! publishes `latest.json`; the app checks the endpoint in `tauri.conf.json`
//! (`plugins.updater`), verifies the signature with the public key there, installs and
//! restarts. Without a public key the check reports that updates are not set up.
//!
//! Commands: `check_update` and `install_update` (feature `updater`).
//!
//! # Example
//!
//! `check_update` returns:
//!
//! ```
//! let info = serde_json::json!({"configured": true, "available": true, "current": "1.0.0", "version": "1.1.0", "notes": "Faster search."});
//! assert_eq!(info["available"], true);
//! ```

use serde::Serialize;
use tauri::Emitter;
use tauri_plugin_updater::UpdaterExt;

/// What `check_update` found, for the UI.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    configured: bool,
    available: bool,
    current: String,
    version: Option<String>,
    notes: Option<String>,
}

fn configured(app: &tauri::AppHandle) -> bool {
    app.config()
        .plugins
        .0
        .get("updater")
        .and_then(|u| u.get("pubkey"))
        .and_then(|k| k.as_str())
        .is_some_and(|k| !k.trim().is_empty())
}

/// Tauri command: is there a newer release? Never installs.
#[tauri::command]
pub async fn check_update(app: tauri::AppHandle) -> Result<UpdateInfo, String> {
    let current = app.package_info().version.to_string();
    if !configured(&app) {
        return Ok(UpdateInfo { configured: false, available: false, current, version: None, notes: None });
    }
    let update = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
    Ok(UpdateInfo {
        configured: true,
        available: update.is_some(),
        current,
        version: update.as_ref().map(|u| u.version.clone()),
        notes: update.and_then(|u| u.body),
    })
}

/// Download (with `update-progress` events: bytes so far, total), install, restart.
#[tauri::command]
pub async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    let Some(update) = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())? else {
        return Err("Already up to date.".into());
    };
    let mut done = 0usize;
    let handle = app.clone();
    update
        .download_and_install(
            move |chunk, total| {
                done += chunk;
                let _ = handle.emit("update-progress", (done, total));
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    // The worker (and its database) must restart on the new binary too.
    #[cfg(feature = "ipc-worker")]
    crate::ipc::shutdown_worker();
    #[cfg(all(feature = "postgres", not(feature = "ipc-worker")))]
    desktop_core::pgstore::shutdown();
    app.restart()
}
