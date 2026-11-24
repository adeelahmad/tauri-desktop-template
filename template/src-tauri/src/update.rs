//! Auto-update from GitHub releases (Tauri updater).
//!
//! The release workflow signs the update bundles with `TAURI_SIGNING_PRIVATE_KEY` and
//! publishes `latest.json`; the app checks the endpoint in `tauri.conf.json`
//! (`plugins.updater`), verifies the signature with the public key there, installs and
//! restarts. Without a public key the check reports that updates are not set up.
//!
//! Commands: `check_update` and `install_update` (feature `updater`){% if release_channels %};
//! `update_channel_status` and `rollback_update` (feature `release-channels`).
//!
//! With `release-channels` the endpoints' `{channel}` token is filled in with the channel
//! picked in Settings → Updates, a staged release (`"rollout": 25` in `latest.json`) is only
//! offered to installs whose bucket is below the percentage, and installing an update first
//! records the running version (and keeps its bundle on macOS and for an AppImage) so that
//! `rollback_update` can go back. See `desktop_core::release` for what each OS allows{% endif %}.
//!
//! # Example
//!
//! `check_update` returns:
//!
//! ```
//! let info = serde_json::json!({"configured": true, "available": true, "current": "1.0.0", "version": "1.1.0", "notes": "Faster search."});
//! assert_eq!(info["available"], true);
//! ```

use crate::error::CommandError;
use serde::Serialize;
use tauri::Emitter;
use tauri_plugin_updater::UpdaterExt;
{% if release_channels %}#[cfg(feature = "release-channels")]
use desktop_core::release::{self, Channel, RollbackRecord};
{% endif %}
/// What `check_update` found, for the UI.
#[derive(Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    configured: bool,
    available: bool,
    current: String,
    version: Option<String>,
    notes: Option<String>,
    /// The release is staged and this install's turn has not come yet (`release-channels`).
    staged: bool,
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
{% if release_channels %}
/// The GitHub repository releases come from (rollback fetches older releases there).
#[cfg(feature = "release-channels")]
const REPO: &str = "{{github_repo}}";

/// The base data folder (not a profile's): the install id and the rollback record belong to
/// the installed app, whichever profile is open.
#[cfg(feature = "release-channels")]
fn data_dir() -> std::path::PathBuf {
    crate::paths::base_dirs().1
}

#[cfg(feature = "release-channels")]
fn channel(app: &tauri::AppHandle) -> Channel {
    crate::load_settings(app).config.updates.channel
}

/// The configured endpoints for `channel`.
#[cfg(feature = "release-channels")]
fn endpoints(app: &tauri::AppHandle, channel: Channel) -> Result<Vec<tauri::Url>, String> {
    let raw: Vec<String> = app
        .config()
        .plugins
        .0
        .get("updater")
        .and_then(|u| u.get("endpoints"))
        .and_then(|e| e.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    release::channel_endpoints(&raw, channel).iter().map(|u| u.parse().map_err(|e| format!("updater endpoint {u}: {e}"))).collect()
}

/// This install's rollout bucket (0 to 99).
#[cfg(feature = "release-channels")]
fn bucket() -> Result<u8, String> {
    release::install_id(&data_dir()).map(|id| release::bucket(&id)).map_err(|e| format!("install id: {e}"))
}

/// The newest release on the picked channel, and whether this install may take it now.
#[cfg(feature = "release-channels")]
async fn find(app: &tauri::AppHandle) -> Result<(Option<tauri_plugin_updater::Update>, bool), String> {
    let updater = app.updater_builder().endpoints(endpoints(app, channel(app))?).map_err(|e| e.to_string())?.build().map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        return Ok((None, false));
    };
    if release::admitted(bucket()?, release::rollout_of(&update.raw_json)) {
        Ok((Some(update), false))
    } else {
        Ok((None, true))
    }
}

/// Before an update replaces the app: remember this version, and keep its bundle where the
/// platform allows (a failure to copy only means the rollback goes online).
#[cfg(feature = "release-channels")]
fn prepare_rollback(app: &tauri::AppHandle) {
    let dir = data_dir();
    let appimage = std::env::var_os("APPIMAGE").map(std::path::PathBuf::from);
    let backup = std::env::current_exe()
        .ok()
        .and_then(|exe| release::bundle_to_keep(&exe, appimage.as_deref(), std::env::consts::OS))
        .and_then(|bundle| release::keep_bundle(&bundle, &dir).map_err(|e| log::warn!("rollback: could not keep {}: {e}", bundle.display())).ok());
    let record = RollbackRecord { previous: app.package_info().version.to_string(), backup };
    if let Err(e) = release::save_record(&dir, &record) {
        log::warn!("rollback: could not save the record: {e}");
    }
}
{% endif %}
/// Tauri command: is there a newer release? Never installs.
#[tauri::command]
pub async fn check_update(app: tauri::AppHandle) -> Result<UpdateInfo, CommandError> {
    let current = app.package_info().version.to_string();
    if !configured(&app) {
        return Ok(UpdateInfo { configured: false, available: false, current, version: None, notes: None, staged: false });
    }
    {% if release_channels %}#[cfg(feature = "release-channels")]
    let (update, staged) = find(&app).await?;
    #[cfg(not(feature = "release-channels"))]
    {% endif %}let (update, staged) = (app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?, false);
    Ok(UpdateInfo {
        configured: true,
        available: update.is_some(),
        current,
        version: update.as_ref().map(|u| u.version.clone()),
        notes: update.and_then(|u| u.body),
        staged,
    })
}

/// Download (with `update-progress` events: bytes so far, total), install, restart.
#[tauri::command]
pub async fn install_update(app: tauri::AppHandle) -> Result<(), CommandError> {
    {% if release_channels %}#[cfg(feature = "release-channels")]
    let update = find(&app).await?.0;
    #[cfg(not(feature = "release-channels"))]
    {% endif %}let update = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
    let Some(update) = update else {
        return Err("Already up to date.".into());
    };
    {% if release_channels %}#[cfg(feature = "release-channels")]
    prepare_rollback(&app);
    {% endif %}Ok(install(&app, update).await?)
}

async fn install(app: &tauri::AppHandle, update: tauri_plugin_updater::Update) -> Result<(), String> {
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
    restart(app)
}

/// Restart on the new binary; the worker (and its database) must restart on it too.
fn restart(app: &tauri::AppHandle) -> ! {
    #[cfg(feature = "ipc-worker")]
    crate::ipc::shutdown_worker();
    #[cfg(all(feature = "postgres", not(feature = "ipc-worker")))]
    desktop_core::pgstore::shutdown();
    app.restart()
}
{% if release_channels %}
/// What `update_channel_status` reports, for Settings → Updates.
#[cfg(feature = "release-channels")]
#[derive(Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ChannelStatus {
    channel: Channel,
    /// This install's rollout bucket (0 to 99): a release rolled out to N% reaches it when
    /// the bucket is below N.
    bucket: Option<u8>,
    /// The version a rollback goes back to.
    previous: Option<String>,
    /// Whether a copy of that version's bundle is kept (rollback works offline).
    kept: bool,
    /// Whether rollback is possible on this platform and install at all.
    rollback: bool,
}

/// Tauri command: the channel, this install's bucket and what a rollback would do.
#[cfg(feature = "release-channels")]
#[tauri::command]
pub fn update_channel_status(app: tauri::AppHandle) -> ChannelStatus {
    let record = release::load_record(&data_dir());
    let kept = record.as_ref().and_then(|r| r.backup.as_deref()).is_some_and(std::path::Path::exists);
    // A .deb install is owned by the package manager: neither updates nor rollbacks apply.
    let deb = cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_none();
    ChannelStatus {
        channel: channel(&app),
        bucket: bucket().ok(),
        previous: record.map(|r| r.previous),
        kept,
        rollback: !deb && configured(&app),
    }
}

/// Tauri command: go back to the version that ran before the last update, then restart.
/// Restores the kept bundle where there is one (macOS, AppImage); otherwise downloads that
/// version's signed update bundle from its GitHub release and installs it (Windows).
#[cfg(feature = "release-channels")]
#[tauri::command]
pub async fn rollback_update(app: tauri::AppHandle) -> Result<(), CommandError> {
    let dir = data_dir();
    let record = release::load_record(&dir).ok_or("No earlier version is recorded: the app has not been updated from here.")?;
    if let Some(backup) = record.backup.as_deref().filter(|b| b.exists()) {
        let appimage = std::env::var_os("APPIMAGE").map(std::path::PathBuf::from);
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        if let Some(target) = release::bundle_to_keep(&exe, appimage.as_deref(), std::env::consts::OS) {
            release::restore_bundle(backup, &target).map_err(|e| format!("Could not put back {}: {e}", record.previous))?;
            let _ = release::clear(&dir);
            restart(&app);
        }
    }
    if !configured(&app) {
        return Err("Updates are not set up (no public key in tauri.conf.json).".into());
    }
    let url: tauri::Url = release::release_manifest_url(REPO, &record.previous).parse().map_err(|e| format!("{e}"))?;
    let previous = record.previous.clone();
    let update = app
        .updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        // Any other version counts as "newer", so the older release installs.
        .version_comparator(move |current, remote| remote.version.to_string() == previous && remote.version != current)
        .build()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| format!("Could not reach the release of {}: {e}", record.previous))?
        .ok_or_else(|| format!("The release of {} has no update bundle for this platform.", record.previous))?;
    let _ = release::clear(&dir);
    Ok(install(&app, update).await?)
}
{% endif %}