mod afk;
mod config;
mod credentials;
mod debug;
mod diagnostics;
mod dpapi;
mod history;
mod live_lobby;
mod lobby;
mod log_folder;
mod log_scan;
mod logging;
mod match_log;
mod ranked_code;
mod release;
mod server;
mod settings;
mod tourney;
mod tourneys;
mod updates;
mod uploader;
mod uploads;
mod watcher;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use afk::AfkStatus;
use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, RunEvent, State, WindowEvent};
use tauri_plugin_window_state::{AppHandleExt, StateFlags};

use credentials::Tokens;
use history::Page;
use live_lobby::{LiveLobby, LobbyStatus};
use log_folder::LogFolder;
use release::ReleaseCache;
use server::TokenCheck;
use settings::Settings;
use tourneys::{Tourneys, TourneysStatus};
use updates::{UpdateStatus, Updates};
use uploader::{UploadStatus, Uploader};

/// The settings file and what's in it. Commands change both together.
struct Store {
    path: PathBuf,
    settings: Mutex<Settings>,
    /// Why the file can't be used, while it can't. The window shows the defaults until the host
    /// changes a setting (which writes a fresh file) or fixes the file, but nothing is uploaded
    /// with them: they'd be the live server and the detected folder, not what the host chose.
    load_error: Mutex<Option<String>>,
    tokens: Tokens,
}

impl Store {
    /// The settings and tokens in `dir`, the app's config folder.
    fn open(dir: &Path) -> Store {
        let path = dir.join(config::SETTINGS_FILE);
        let (settings, load_error) = match settings::load(&path) {
            Ok(settings) => (settings, None),
            Err(e) => (Settings::default(), Some(e)),
        };
        Store {
            path,
            settings: Mutex::new(settings),
            load_error: Mutex::new(load_error),
            tokens: Tokens::new(dir.join(config::TOKENS_FALLBACK_FILE)),
        }
    }

    fn get(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    /// `load_error`, after reading the file again: a file fixed by hand counts as soon as it's saved.
    fn load_error(&self) -> Option<String> {
        let mut settings = self.settings.lock().unwrap();
        let mut error = self.load_error.lock().unwrap();
        if error.is_some() {
            match settings::load(&self.path) {
                Ok(loaded) => {
                    *settings = loaded;
                    *error = None;
                }
                Err(e) => *error = Some(e),
            }
        }
        error.clone()
    }

    fn update(&self, change: impl FnOnce(&mut Settings)) -> Result<(), String> {
        let mut settings = self.settings.lock().unwrap();
        let mut next = settings.clone();
        change(&mut next);
        settings::save(&self.path, &next)?;
        *settings = next;
        *self.load_error.lock().unwrap() = None;
        Ok(())
    }
}

/// What the window shows. Never the token itself.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppState {
    version: String,
    server_url: String,
    default_server_url: &'static str,
    has_token: bool,
    log_folder: Option<LogFolder>,
    /// Where the verify screenshot of a tourney lobby is offered from (#10).
    screenshot_folder: Option<LogFolder>,
    /// The biggest verify screenshot the server takes, in bytes.
    screenshot_max_bytes: u64,
    /// How often the window looks for a new screenshot while it asks for one, in seconds.
    screenshot_poll_secs: u64,
    /// The region the host picked, `None` for their home region.
    region: Option<String>,
    regions: &'static [config::Region],
    /// Whether the live lobby is on (#6), and the name it's listed under (`None`: none).
    live_lobby: bool,
    lobby_name: Option<String>,
    lobby_name_max: usize,
    settings_error: Option<String>,
    /// Every `config::TUNABLES`, in order, with the host's value.
    advanced: Vec<AdvancedSetting>,
    /// `config::MATCH_VIEW_POLL_SECS` as the host set it.
    match_view_poll_secs: u64,
    /// The log level the host picked, `None` for `default_log_level`.
    log_level: Option<String>,
    log_levels: &'static [&'static str],
    default_log_level: &'static str,
    /// The GenjiBall-CE release the ranked code is built from, `None` for the latest ranked one.
    release_tag: Option<String>,
    release_tag_suffix: &'static str,
    /// Whether uploads are a dry run (Advanced, Debug).
    dry_run: bool,
    /// Whether the tool looks for updates by itself, and the channel it reads.
    auto_update_check: bool,
    update_channel: String,
    update_channels: &'static [&'static str],
    default_update_channel: &'static str,
}

/// A tunable under Advanced, as the window shows it. All in seconds.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AdvancedSetting {
    key: &'static str,
    label: &'static str,
    help: &'static str,
    default: u64,
    min: u64,
    max: u64,
    /// The host's value, `None` at the default.
    value: Option<u64>,
}

fn app_state(app: &tauri::AppHandle, store: &Store) -> Result<AppState, String> {
    let settings_error = store.load_error();
    let settings = store.get();
    Ok(AppState {
        version: app.package_info().version.to_string(),
        server_url: settings.server_url().to_string(),
        default_server_url: config::DEFAULT_SERVER_URL,
        has_token: store.tokens.get(settings.server_url())?.is_some(),
        log_folder: log_folder::current(settings.log_folder.as_deref()),
        screenshot_folder: log_folder::screenshots(settings.screenshot_folder.as_deref()),
        screenshot_max_bytes: config::SCREENSHOT_MAX_BYTES,
        screenshot_poll_secs: config::SCREENSHOT_POLL_SECS,
        region: settings.region.clone(),
        regions: &config::REGIONS,
        live_lobby: settings.live_lobby_on(),
        lobby_name: settings.lobby_name.clone(),
        lobby_name_max: config::LOBBY_NAME_MAX_CHARS,
        settings_error,
        advanced: config::TUNABLES
            .iter()
            .map(|t| AdvancedSetting {
                key: t.key,
                label: t.label,
                help: t.help,
                default: t.default,
                min: t.min,
                max: t.max,
                value: settings.advanced.get(t.key).copied(),
            })
            .collect(),
        match_view_poll_secs: settings.get(&config::MATCH_VIEW_POLL_SECS),
        log_level: settings.log_level.clone(),
        log_levels: &config::LOG_LEVELS,
        default_log_level: config::DEFAULT_LOG_LEVEL,
        release_tag: settings.release_tag.clone(),
        release_tag_suffix: config::RELEASE_TAG_SUFFIX,
        dry_run: settings.dry_run_on(),
        auto_update_check: settings.auto_update_check_on(),
        update_channel: settings.update_channel().to_string(),
        update_channels: &config::UPDATE_CHANNELS,
        default_update_channel: config::DEFAULT_UPDATE_CHANNEL,
    })
}

#[tauri::command]
fn get_state(app: tauri::AppHandle, store: State<Store>) -> Result<AppState, String> {
    app_state(&app, &store)
}

/// Checks the token saved for the current server.
#[tauri::command]
async fn check_saved_token(
    store: State<'_, Store>,
    uploader: State<'_, Uploader>,
    lobby: State<'_, LiveLobby>,
) -> Result<TokenCheck, String> {
    let settings = store.get();
    let server_url = settings.server_url().to_string();
    let token = store
        .tokens
        .get(&server_url)?
        .ok_or("No host token saved for this server")?;
    // The listed matches' status too: "Check again" after an admin accepted one.
    uploader.refresh();
    let timeout = settings.secs(&config::REQUEST_TIMEOUT_SECS);
    let check = server::check_token(&server_url, &token, timeout).await;
    log::info!("Token check for {server_url}: {}", check.summary());
    if matches!(check, TokenCheck::Ok { .. }) {
        // The server knows it now (an admin fixed it, say): stop holding uploads for it.
        uploader.token_ok();
        lobby.changed();
    }
    Ok(check)
}

/// Checks a token the host entered, and saves it unless the server turned it down. A server that
/// can't be reached doesn't stop the save: the host may be offline, and uploads will retry.
#[tauri::command]
async fn save_token(
    token: String,
    store: State<'_, Store>,
    uploader: State<'_, Uploader>,
    lobby: State<'_, LiveLobby>,
    tourneys: State<'_, Tourneys>,
) -> Result<TokenCheck, String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("Paste the host token an admin gave you".into());
    }
    let settings = store.get();
    let server_url = settings.server_url().to_string();
    let timeout = settings.secs(&config::REQUEST_TIMEOUT_SECS);
    let check = server::check_token(&server_url, token, timeout).await;
    log::info!("New token for {server_url}: {}", check.summary());
    if !check.is_rejected() {
        store.tokens.set(&server_url, token).inspect_err(|e| {
            log::error!("Couldn't save the token for {server_url}: {e}");
        })?;
        log::info!("Token saved for {server_url}");
        // Uploads to this server start with the logs written from now on. A token saved again,
        // even the same one, is tried again.
        uploader.start(&server_url);
        uploader.token_ok();
        uploader.changed();
        lobby.changed();
        tourneys.refresh();
    }
    Ok(check)
}

#[tauri::command]
fn forget_token(
    store: State<Store>,
    uploader: State<Uploader>,
    lobby: State<LiveLobby>,
    tourneys: State<Tourneys>,
) -> Result<(), String> {
    let server_url = store.get().server_url().to_string();
    store.tokens.delete(&server_url)?;
    log::info!("Token for {server_url} forgotten");
    uploader.changed();
    lobby.changed();
    tourneys.refresh();
    Ok(())
}

#[tauri::command]
fn get_upload_status(uploader: State<Uploader>) -> UploadStatus {
    uploader.status()
}

/// A page of the upload history to the current server, from 0 (the newest).
#[tauri::command]
fn get_upload_history(page: usize, store: State<Store>, uploader: State<Uploader>) -> Page {
    let settings = store.get();
    let folder = log_folder::current(settings.log_folder.as_deref()).filter(|f| f.exists);
    uploader.history(
        settings.server_url(),
        folder.as_ref().map(|f| f.path.as_path()),
        page,
    )
}

/// Turns host AFK on or off: from then on, rounds that start aren't rated for the host (`afk.rs`).
/// The live log is read now, so a round already started (even one not read yet) still counts.
#[tauri::command]
fn set_afk(on: bool, store: State<Store>, uploader: State<Uploader>) -> Result<AfkStatus, String> {
    let settings = store.get();
    let started = match log_folder::current(settings.log_folder.as_deref()).filter(|f| f.exists) {
        Some(folder) => live_round_starts(&folder.path)?,
        None => Vec::new(),
    };
    Ok(uploader.set_afk(on, &started))
}

/// The rounds started in the live log (the newest in `folder`), now.
fn live_round_starts(folder: &Path) -> Result<Vec<log_scan::RoundStart>, String> {
    let live =
        watcher::newest_log(folder).map_err(|e| format!("Couldn't read the log folder: {e}"))?;
    let Some(path) = live else {
        return Ok(Vec::new());
    };
    // Locked for a moment while the game writes it: the host clicks again.
    let bytes = std::fs::read(&path)
        .map_err(|e| format!("Couldn't read the match log ({e}). Try again"))?;
    Ok(log_scan::round_starts(&String::from_utf8_lossy(&bytes)))
}

/// The log folder in use, if it's there.
fn current_log_folder(store: &Store) -> Result<PathBuf, String> {
    log_folder::current(store.get().log_folder.as_deref())
        .filter(|f| f.exists)
        .map(|f| f.path)
        .ok_or_else(|| "The Workshop log folder isn't there yet".to_string())
}

/// A log's complete lines for the match view, by file name in the log folder in use (`match_log`).
/// No text when the window already has it at that size (`known`).
#[tauri::command]
fn read_match_log(
    file: String,
    known: Option<match_log::Known>,
    store: State<Store>,
) -> Result<match_log::LogText, String> {
    match_log::read(&current_log_folder(&store)?, &file, known.as_ref())
}

/// The live log (the newest in the log folder) for the match view's current match; `None` while
/// there's no log.
#[tauri::command]
fn read_live_log(
    known: Option<match_log::Known>,
    store: State<Store>,
) -> Result<Option<match_log::LogText>, String> {
    match_log::read_live(&current_log_folder(&store)?, known.as_ref())
}

/// Tries a failed upload again now, through the upload queue.
#[tauri::command]
fn retry_upload(file: String, uploader: State<Uploader>) {
    uploader.retry(file);
}

/// Opens the page of one of the host's public matches on the current server's site, in the host's
/// browser. The window only gives the id: the URL is built here, so it can't open anything else.
#[tauri::command]
fn open_match(match_id: i64, store: State<Store>, uploader: State<Uploader>) -> Result<(), String> {
    let server_url = store.get().server_url().to_string();
    if !uploader.has_public_match(&server_url, match_id) {
        return Err("That match isn't on the site: only accepted and voided matches are".into());
    }
    let url = server::match_page_url(&server_url, match_id)?;
    log::debug!("Opening {url}");
    tauri_plugin_opener::open_url(url, None::<&str>)
        .map_err(|e| format!("Couldn't open the browser: {e}"))
}

/// Empty goes back to the default server.
#[tauri::command]
fn set_server_url(
    url: String,
    app: tauri::AppHandle,
    store: State<Store>,
    uploader: State<Uploader>,
    lobby: State<LiveLobby>,
    tourneys: State<Tourneys>,
) -> Result<AppState, String> {
    let url = settings::normalize_server_url(&url)?;
    store.update(|s| s.server_url = url)?;
    log::info!("Server set to {}", store.get().server_url());
    uploader.changed();
    lobby.changed();
    tourneys.refresh();
    app_state(&app, &store)
}

/// `None` goes back to the detected folder.
#[tauri::command]
fn set_log_folder(
    path: Option<PathBuf>,
    app: tauri::AppHandle,
    store: State<Store>,
    uploader: State<Uploader>,
    lobby: State<LiveLobby>,
) -> Result<AppState, String> {
    store.update(|s| s.log_folder = path)?;
    log::info!(
        "Log folder set to {:?}",
        log_folder::current(store.get().log_folder.as_deref()).map(|f| f.path)
    );
    uploader.changed();
    lobby.changed();
    app_state(&app, &store)
}

/// The region the host hosts in now, `None` for their home region. Uploads from the next one on
/// go as it, and the ranked code gets its rank tags.
#[tauri::command]
fn set_region(
    region: Option<String>,
    app: tauri::AppHandle,
    store: State<Store>,
    uploader: State<Uploader>,
    lobby: State<LiveLobby>,
) -> Result<AppState, String> {
    if let Some(region) = &region {
        settings::check_region(region)?;
    }
    store.update(|s| s.region = region.clone())?;
    log::info!(
        "Region set to {}",
        region.as_deref().unwrap_or("the home region")
    );
    uploader.changed();
    lobby.changed();
    app_state(&app, &store)
}

/// The Advanced values the host entered, by key. A key left out goes back to its default, so `{}`
/// resets them all. Nothing is saved unless every value is in its range.
#[tauri::command]
fn set_advanced(
    values: BTreeMap<String, u64>,
    app: tauri::AppHandle,
    store: State<Store>,
    uploader: State<Uploader>,
    lobby: State<LiveLobby>,
) -> Result<AppState, String> {
    let values = settings::normalize_advanced(&values)?;
    store.update(|s| s.replace_advanced(values.clone()))?;
    log::info!("Advanced values set to {values:?} (others at their default)");
    // The next poll starts now, with the new values.
    uploader.wake();
    lobby.changed();
    app_state(&app, &store)
}

/// Switches the live lobby on or off and sets the name it's listed under (empty: none). The next
/// heartbeat says the new name; switched off, a listed lobby is taken off the list now.
#[tauri::command]
fn set_live_lobby(
    on: bool,
    name: String,
    app: tauri::AppHandle,
    store: State<Store>,
    lobby: State<LiveLobby>,
) -> Result<AppState, String> {
    let name = settings::normalize_lobby_name(&name)?;
    store.update(|s| {
        s.set_live_lobby(on);
        s.lobby_name = name;
    })?;
    lobby.changed();
    app_state(&app, &store)
}

/// Whether the host's lobby is listed on the site now, and why not.
#[tauri::command]
fn get_lobby_status(lobby: State<LiveLobby>) -> LobbyStatus {
    lobby.status()
}

/// How much the tool logs, from now on. `None` goes back to the default.
#[tauri::command]
fn set_log_level(
    level: Option<String>,
    app: tauri::AppHandle,
    store: State<Store>,
) -> Result<AppState, String> {
    let level = settings::normalize_log_level(level.as_deref())?;
    store.update(|s| s.log_level = level)?;
    let level = store.get().log_level().to_string();
    logging::set_level(&level);
    log::info!("Log level set to {level}");
    app_state(&app, &store)
}

/// The GenjiBall-CE release the ranked code is built from (`1.3.3R`). Empty: the latest ranked one.
#[tauri::command]
fn set_release_tag(
    tag: String,
    app: tauri::AppHandle,
    store: State<Store>,
) -> Result<AppState, String> {
    let tag = settings::normalize_release_tag(&tag)?;
    store.update(|s| s.release_tag = tag.clone())?;
    log::info!(
        "Ranked code release set to {}",
        tag.as_deref().unwrap_or("the latest")
    );
    app_state(&app, &store)
}

/// Switches the dry run on or off. A poll under way stops sending, and the next one starts now:
/// switched off, the files the dry run went through are uploaded then.
#[tauri::command]
fn set_dry_run(
    on: bool,
    app: tauri::AppHandle,
    store: State<Store>,
    uploader: State<Uploader>,
) -> Result<AppState, String> {
    store.update(|s| s.set_dry_run(on))?;
    log::info!("Dry run {}", if on { "on" } else { "off" });
    uploader.changed();
    app_state(&app, &store)
}

/// What the debug panel shows (Advanced, Debug).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DebugInfo {
    /// The ranked files waiting to be uploaded, as of the last poll.
    queue: Vec<watcher::Queued>,
    /// The latest uploads and dry runs, newest first.
    uploads: Vec<debug::Attempt>,
    /// The live log's newest events, `None` while there's no log.
    events: Option<debug::LiveEvents>,
    /// Why the live log couldn't be read.
    events_error: Option<String>,
}

#[tauri::command]
fn get_debug(store: State<Store>, uploader: State<Uploader>) -> DebugInfo {
    let settings = store.get();
    let folder = log_folder::current(settings.log_folder.as_deref()).filter(|f| f.exists);
    let folder = folder.as_ref().map(|f| f.path.as_path());
    let (events, events_error) =
        match folder.map(|f| debug::live_events(f, config::DEBUG_EVENTS_SHOWN)) {
            None => (
                None,
                Some("The Workshop log folder isn't there yet".to_string()),
            ),
            Some(Ok(events)) => (events, None),
            // Locked for a moment while the game writes it: the next refresh reads it.
            Some(Err(e)) => (None, Some(format!("Couldn't read the newest log: {e}"))),
        };
    DebugInfo {
        queue: uploader.queue(settings.server_url(), folder),
        uploads: uploader.recent(),
        events,
        events_error,
    }
}

/// Whether the tool looks for updates by itself, and the channel it reads (`None`: the default).
/// A new channel is checked at once, and the update the other one offered is dropped: it may not
/// be on this one.
#[tauri::command]
fn set_updates(
    auto_check: bool,
    channel: Option<String>,
    app: tauri::AppHandle,
    store: State<Store>,
    updates: State<Updates>,
) -> Result<AppState, String> {
    let channel = settings::normalize_update_channel(channel.as_deref())?;
    let before = store.get().update_channel().to_string();
    store.update(|s| {
        s.set_auto_update_check(auto_check);
        s.update_channel = channel;
    })?;
    let settings = store.get();
    log::info!(
        "Update checks {}, channel {}",
        if auto_check { "on" } else { "off" },
        settings.update_channel()
    );
    if settings.update_channel() != before {
        let _ = app.emit("update-status", updates.switched());
        let app = app.clone();
        tauri::async_runtime::spawn(async move { updates::check(&app).await });
    }
    app_state(&app, &store)
}

/// Opens the tool's log folder in Explorer, through the opener's Rust API.
#[tauri::command]
fn open_log_folder(app: tauri::AppHandle) -> Result<(), String> {
    let dir = app
        .path()
        .app_log_dir()
        .map_err(|e| format!("Couldn't find the log folder: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    tauri_plugin_opener::open_path(&dir, None::<&str>)
        .map_err(|e| format!("Couldn't open {}: {e}", dir.display()))
}

/// Writes a diagnostics export (`diagnostics.rs`) where the host picks, offered in Downloads.
/// The path it was saved to, `None` if the host cancelled.
#[tauri::command]
async fn export_diagnostics(
    app: tauri::AppHandle,
    store: State<'_, Store>,
    uploader: State<'_, Uploader>,
    updates: State<'_, Updates>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let settings = store.get();
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let log_dir = app.path().app_log_dir().ok();
    let workshop = log_folder::current(settings.log_folder.as_deref()).filter(|f| f.exists);
    // Blanked out wherever they turn up. Tokens never go in these files; this makes sure.
    let mut secrets = Vec::new();
    for server_url in [settings.server_url(), config::DEFAULT_SERVER_URL] {
        if let Ok(Some(token)) = store.tokens.get(server_url) {
            secrets.push(token);
        }
    }
    let version = app.package_info().version.to_string();
    let sources = diagnostics::Sources {
        version: &version,
        webview: tauri::webview_version().ok(),
        config_dir: &config_dir,
        log_dir: log_dir.as_deref(),
        workshop_folder: workshop.as_ref().map(|f| f.path.as_path()),
        upload_status: serde_json::to_value(uploader.status()).unwrap_or_default(),
        update_status: serde_json::to_value(updates.status()).unwrap_or_default(),
        recent_uploads: serde_json::to_value(uploader.recent()).unwrap_or_default(),
    };
    let now = chrono::Local::now();
    let text = diagnostics::collect(&sources, &secrets, now);

    let (sent, chosen) = tokio::sync::oneshot::channel();
    let mut dialog = app
        .dialog()
        .file()
        .set_title("Export diagnostics")
        .set_file_name(format!(
            "{}-{}.json",
            config::DIAGNOSTICS_FILE_PREFIX,
            now.format("%Y-%m-%d-%H-%M")
        ))
        .add_filter("JSON", &["json"]);
    if let Ok(downloads) = app.path().download_dir() {
        dialog = dialog.set_directory(downloads);
    }
    dialog.save_file(move |path| {
        let _ = sent.send(path);
    });
    let Some(path) = chosen.await.ok().flatten() else {
        return Ok(None);
    };
    let path = path
        .into_path()
        .map_err(|e| format!("Can't save there: {e}"))?;
    std::fs::write(&path, text).map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
    log::info!("Diagnostics exported to {}", path.display());
    Ok(Some(path.display().to_string()))
}

/// The update the last check found, and whether the last check worked.
#[tauri::command]
fn get_update_status(updates: State<Updates>) -> UpdateStatus {
    updates.status()
}

/// Looks for an update now ("Check for updates"). A failed check is in the status, not an error.
#[tauri::command]
async fn check_for_update(app: tauri::AppHandle) -> UpdateStatus {
    updates::check(&app).await
}

/// Installs the update found and restarts the tool. Only returns when that failed.
#[tauri::command]
async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    updates::install(&app).await
}

/// The ranked Workshop code, for the window to copy.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RankedCode {
    code: String,
    /// The server the rank tags came from: the window drops a code built for a server the host
    /// has since switched away from.
    server_url: String,
    /// The region the host picked when it was built (`None`: the home region), likewise.
    chosen_region: Option<String>,
    /// The region the rank tags were asked for: the one picked, else the home region (`None`
    /// without a token: the server's first region). The window drops a kept code for another.
    requested_region: Option<String>,
    /// The region whose rank tags are in it, as the server said. `None` from a server without
    /// regions.
    region: Option<String>,
    /// The GenjiBall-CE release it's built from (`1.3.3R`).
    release: String,
    /// When the leaderboard was read for the tags (ISO 8601).
    tags_updated_at: String,
    /// Top players tagged with their place and rating.
    top: usize,
    /// Names tagged with their rank tier.
    names: usize,
    /// Names the Workshop can't show, left out.
    skipped_names: usize,
    /// How long the window may keep a code it couldn't copy for the next click, before building
    /// a new one: as long as the release found is reused.
    keep_secs: u64,
}

/// The latest ranked release's code with the top players of the current server and region tagged
/// in it.
#[tauri::command]
async fn build_ranked_code(
    store: State<'_, Store>,
    releases: State<'_, ReleaseCache>,
) -> Result<RankedCode, String> {
    let built = ranked_code(&store, &releases).await;
    match &built {
        Ok(code) => log::info!(
            "Built the ranked code from release {} for {} (region {}): top {}, {} more names, {} left out",
            code.release,
            code.server_url,
            code.region.as_deref().unwrap_or("?"),
            code.top,
            code.names,
            code.skipped_names
        ),
        Err(e) => log::warn!("Couldn't build the ranked code: {e}"),
    }
    built
}

async fn ranked_code(store: &Store, releases: &ReleaseCache) -> Result<RankedCode, String> {
    let settings = store.get();
    let server_url = settings.server_url().to_string();
    let keep = settings.secs(&config::RELEASE_CACHE_SECS);
    let timeout = settings.secs(&config::REQUEST_TIMEOUT_SECS);
    let region = match &settings.region {
        Some(region) => Some(region.clone()),
        None => home_region(store, &server_url, timeout).await?,
    };
    let (release, base) = releases
        .get(settings.release_tag.as_deref(), keep, timeout)
        .await?;
    let tiers = server::rank_tags(&server_url, region.as_deref(), timeout).await?;
    let leaderboard = server::leaderboard(&server_url, region.as_deref(), timeout).await?;
    let now = chrono::Local::now();
    let built = ranked_code::top_tags(&tiers, &leaderboard, &now.format("%Y-%m-%d").to_string());
    let filled = ranked_code::fill(&base, &built.tags)?;
    Ok(RankedCode {
        code: filled.code,
        server_url,
        chosen_region: settings.region,
        requested_region: region,
        region: leaderboard.region,
        release,
        tags_updated_at: now.to_rfc3339(),
        top: built.top,
        names: filled.names - built.top,
        skipped_names: built.skipped + filled.skipped,
        keep_secs: keep.as_secs(),
    })
}

/// The home region of the token saved for `server_url`, asked now: the token may have changed
/// since the uploader last asked. `None` without a token (the server's first region then, which
/// its answer names). A check that fails is an error, not that first region: it may be the wrong
/// one.
async fn home_region(
    store: &Store,
    server_url: &str,
    timeout: std::time::Duration,
) -> Result<Option<String>, String> {
    let Some(token) = store.tokens.get(server_url)? else {
        return Ok(None);
    };
    match server::check_token(server_url, &token, timeout).await {
        TokenCheck::Ok { host } => host.region.map(Some).ok_or_else(|| {
            "You have no home region: pick the region you host in under Ranked server".to_string()
        }),
        TokenCheck::Unknown | TokenCheck::Revoked => {
            Err("The server turned your host token down, so your home region isn't known. Pick the region you host in under Ranked server".into())
        }
        TokenCheck::Unreachable { message } => Err(format!(
            "Couldn't ask the server for your home region ({message}). Pick the region you host in under Ranked server"
        )),
    }
}

/// The tourney lobbies the host is assigned to, as last read (#8).
#[tauri::command]
fn get_tourneys(tourneys: State<Tourneys>) -> TourneysStatus {
    tourneys.status()
}

/// Asks the server for the host's tourney lobbies now ("Check again").
#[tauri::command]
async fn check_tourneys(
    app: tauri::AppHandle,
    tourneys: State<'_, Tourneys>,
) -> Result<TourneysStatus, String> {
    tourneys::check(&app).await;
    Ok(tourneys.status())
}

/// A tourney lobby's Workshop code, for the window to copy (#9).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TourneyCode {
    code: String,
    /// The server it's for: the window drops a code built for a server the host switched from.
    server_url: String,
    lobby_id: i64,
    /// The `TOURNEY - generated` rule's values, as written.
    values: tourney::Written,
    /// The lobby's region: its rank tags are in the code, and its match is uploaded as it.
    region: String,
    /// The GenjiBall-CE release it's built from (`1.3.3R`).
    release: String,
    /// Top players tagged with their place and rating, names tagged with their rank tier, and
    /// names left out (the Workshop can't show them).
    top: usize,
    names: usize,
    skipped_names: usize,
}

/// The ranked code for lobby `lobby_id`'s region, with the lobby's `TOURNEY - generated` rule
/// turned on. Only while the server gives the lobby's code values (its code window).
#[tauri::command]
async fn build_tourney_code(
    lobby_id: i64,
    store: State<'_, Store>,
    releases: State<'_, ReleaseCache>,
    uploader: State<'_, Uploader>,
) -> Result<TourneyCode, String> {
    let built = tourney_code(lobby_id, &store, &releases, &uploader).await;
    match &built {
        Ok(code) => log::info!(
            "Built the tourney code for lobby {} ({}, {}, key {}, {} rounds) from release {} for {} (region {})",
            code.lobby_id,
            code.values.name,
            code.values.label,
            code.values.lobby_key,
            code.values.round_limit,
            code.release,
            code.server_url,
            code.region
        ),
        Err(e) => log::warn!("Couldn't build the tourney code for lobby {lobby_id}: {e}"),
    }
    built
}

async fn tourney_code(
    lobby_id: i64,
    store: &Store,
    releases: &ReleaseCache,
    uploader: &Uploader,
) -> Result<TourneyCode, String> {
    let settings = store.get();
    let server_url = settings.server_url().to_string();
    let keep = settings.secs(&config::RELEASE_CACHE_SECS);
    let timeout = settings.secs(&config::REQUEST_TIMEOUT_SECS);
    let token = store
        .tokens
        .get(&server_url)?
        .ok_or("No host token saved for this server")?;
    // Asked now: the code values are only there while the window is open, and an admin may have
    // changed the lobby since the list was read.
    let list = server::host_tourneys(&server_url, &token, timeout)
        .await
        .map_err(|e| match e {
            server::TourneysError::TokenRejected { .. } => {
                "The server turned your host token down".to_string()
            }
            server::TourneysError::Failed { message } => message,
        })?;
    uploader.learn_lobbies(&server_url, &list.lobbies);
    let lobby = list
        .lobbies
        .iter()
        .find(|l| l.id == lobby_id)
        .ok_or("This lobby isn't assigned to you any more")?;
    let Some(values) = &lobby.code else {
        let opens = lobby
            .code_from
            .as_deref()
            .and_then(tourney::parse_time)
            .filter(|_| !tourney::is_done(lobby));
        return Err(match opens {
            Some(at) => format!(
                "The code for {} isn't available yet: it opens {}",
                lobby.label,
                at.with_timezone(&chrono::Local).format("%a %e %b at %H:%M")
            ),
            None => format!(
                "The code for {} isn't available any more: the lobby is done",
                lobby.label
            ),
        });
    };
    let (release, base) = releases
        .get(settings.release_tag.as_deref(), keep, timeout)
        .await?;
    let region = Some(lobby.region.as_str());
    let tiers = server::rank_tags(&server_url, region, timeout).await?;
    let leaderboard = server::leaderboard(&server_url, region, timeout).await?;
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let built = ranked_code::top_tags(&tiers, &leaderboard, &date);
    let filled = ranked_code::fill(&base, &built.tags)?;
    let (code, values) = tourney::fill(&filled.code, values)?;
    Ok(TourneyCode {
        code,
        server_url,
        lobby_id,
        values,
        region: lobby.region.clone(),
        release,
        top: built.top,
        names: filled.names - built.top,
        skipped_names: built.skipped + filled.skipped,
    })
}

/// The screenshots folder in use, if it's there.
fn screenshot_folder(store: &Store) -> Result<PathBuf, String> {
    log_folder::screenshots(store.get().screenshot_folder.as_deref())
        .filter(|f| f.exists)
        .map(|f| f.path)
        .ok_or_else(|| {
            "The screenshots folder isn't there. Choose the folder your screenshots go to".into()
        })
}

/// The newest image in the screenshots folder, `None` when it has none.
#[tauri::command]
fn newest_screenshot(store: State<Store>) -> Result<Option<tourney::ScreenshotFile>, String> {
    tourney::newest_screenshot(&screenshot_folder(&store)?)
        .map_err(|e| format!("Couldn't read the screenshots folder: {e}"))
}

/// An image the host picked, dropped or was offered, for its preview: its bytes, only if it's an
/// image the server takes (`tourney::read_screenshot`).
#[tauri::command]
fn read_screenshot(path: PathBuf) -> Result<tauri::ipc::Response, String> {
    let (bytes, _) = tourney::read_screenshot(&path)?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Uploads the image in the request's body as the verify screenshot of the lobby in its
/// `lobby-id` header, replacing the one there, then reads the lobbies again.
#[tauri::command]
async fn upload_screenshot(
    request: tauri::ipc::Request<'_>,
    app: tauri::AppHandle,
    store: State<'_, Store>,
    tourneys: State<'_, Tourneys>,
) -> Result<TourneysStatus, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("No image was sent".into());
    };
    let lobby_id = request
        .headers()
        .get("lobby-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<i64>().ok())
        .ok_or("No lobby was given")?;
    let kind = tourney::check_screenshot(bytes)?;
    change_screenshot(
        &app,
        &store,
        &tourneys,
        lobby_id,
        Some((bytes.clone(), kind)),
    )
    .await
}

/// Deletes the verify screenshot of lobby `lobby_id`, then reads the lobbies again.
#[tauri::command]
async fn delete_screenshot(
    lobby_id: i64,
    app: tauri::AppHandle,
    store: State<'_, Store>,
    tourneys: State<'_, Tourneys>,
) -> Result<TourneysStatus, String> {
    change_screenshot(&app, &store, &tourneys, lobby_id, None).await
}

async fn change_screenshot(
    app: &tauri::AppHandle,
    store: &Store,
    tourneys: &Tourneys,
    lobby_id: i64,
    image: Option<(Vec<u8>, &str)>,
) -> Result<TourneysStatus, String> {
    let settings = store.get();
    let server_url = settings.server_url().to_string();
    let timeout = settings.secs(&config::REQUEST_TIMEOUT_SECS);
    let token = store
        .tokens
        .get(&server_url)?
        .ok_or("No host token saved for this server")?;
    let what = match &image {
        Some((bytes, kind)) => format!("Uploading a {kind} of {} bytes", bytes.len()),
        None => "Deleting".into(),
    };
    log::info!("{what} as the verify screenshot of lobby {lobby_id} on {server_url}");
    let changed = server::put_screenshot(&server_url, &token, lobby_id, image, timeout).await;
    match &changed {
        Ok(_) => log::info!("Verify screenshot of lobby {lobby_id}: done"),
        Err(e) => log::warn!("Verify screenshot of lobby {lobby_id}: {e}"),
    }
    // The list as the server has it now, whatever came of it.
    tourneys::check(app).await;
    changed.map(|_| tourneys.status())
}

/// `None` goes back to the detected screenshots folder.
#[tauri::command]
fn set_screenshot_folder(
    path: Option<PathBuf>,
    app: tauri::AppHandle,
    store: State<Store>,
) -> Result<AppState, String> {
    store.update(|s| s.screenshot_folder = path)?;
    log::info!(
        "Screenshots folder set to {:?}",
        log_folder::screenshots(store.get().screenshot_folder.as_deref()).map(|f| f.path)
    );
    app_state(&app, &store)
}

fn show_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// The tray icon: click it to open the window, or quit from its menu.
fn tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let mut tray = TrayIconBuilder::new()
        .tooltip(app.package_info().name.clone())
        .menu(&Menu::with_items(app, &[&open, &quit])?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// What the window keeps between starts: its size, place and whether it's maximised.
const WINDOW_STATE: StateFlags = StateFlags::SIZE
    .union(StateFlags::POSITION)
    .union(StateFlags::MAXIMIZED);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // First, so a second launch stops here: it shows this window (maybe in the tray) rather
        // than running a second uploader on the same files.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_window(app)
        }))
        // After the single instance check, so a second launch never touches the log file.
        .plugin(logging::plugin())
        .plugin(tauri_plugin_dialog::init())
        // Only its Rust API is used (`updates.rs`): the window has no updater permission.
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Only its Rust API is used (`tourneys.rs`): the window has no notification permission.
        .plugin(tauri_plugin_notification::init())
        // The window opens where and as big as it was left. Rust only: no window permission.
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(WINDOW_STATE)
                .build(),
        )
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            let store = Store::open(&dir);
            logging::set_level(store.get().log_level());
            log::info!("Host tool v{} started", app.package_info().version);
            if let Some(e) = store.load_error() {
                log::error!("{e}");
            }
            app.manage(store);
            app.manage(Uploader::new(dir.join(config::UPLOADS_FILE)));
            app.manage(ReleaseCache::default());
            app.manage(Updates::default());
            app.manage(LiveLobby::default());
            app.manage(Tourneys::default());
            uploader::start(app.handle().clone());
            live_lobby::start(app.handle().clone());
            tourneys::start(app.handle().clone());
            updates::start(app.handle().clone());
            tray(app)?;
            Ok(())
        })
        // Closing the window keeps the tool uploading from the tray; Quit there stops it.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                // Saved now too, not only on quit: an update's restart skips the quit.
                let _ = window.app_handle().save_window_state(WINDOW_STATE);
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            check_saved_token,
            save_token,
            forget_token,
            set_server_url,
            set_log_folder,
            set_advanced,
            set_region,
            set_log_level,
            set_release_tag,
            set_dry_run,
            get_debug,
            set_updates,
            open_log_folder,
            export_diagnostics,
            get_upload_status,
            build_ranked_code,
            get_upload_history,
            retry_upload,
            set_afk,
            set_live_lobby,
            get_lobby_status,
            open_match,
            read_match_log,
            read_live_log,
            get_tourneys,
            check_tourneys,
            build_tourney_code,
            newest_screenshot,
            read_screenshot,
            upload_screenshot,
            delete_screenshot,
            set_screenshot_folder,
            get_update_status,
            check_for_update,
            install_update
        ])
        .build(tauri::generate_context!())
        .expect("error while running the host tool")
        .run(|app, event| {
            // Quit from the tray: take a listed lobby off the site rather than wait for its TTL.
            if let RunEvent::Exit = event {
                live_lobby::close_on_quit(app);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn a_settings_error_lasts_until_the_file_is_fixed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(config::SETTINGS_FILE);
        fs::write(&path, r#"{ "serverUrl": "http://192.168.1.20:8787" }"#).unwrap();
        let store = Store::open(dir.path());
        assert!(store.load_error().is_some());
        assert!(store.load_error().is_some(), "still");
        // The defaults stand in meanwhile.
        assert_eq!(store.get(), Settings::default());

        fs::write(&path, r#"{ "serverUrl": "http://localhost:8787" }"#).unwrap();
        assert_eq!(store.load_error(), None);
        assert_eq!(store.get().server_url(), "http://localhost:8787");
    }

    #[test]
    fn changing_a_setting_writes_a_fresh_file_and_clears_the_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(config::SETTINGS_FILE);
        fs::write(&path, "{ broken").unwrap();
        let store = Store::open(dir.path());
        assert!(store.load_error().is_some());
        store
            .update(|s| s.server_url = Some("http://localhost:8787".into()))
            .unwrap();
        assert_eq!(store.load_error(), None);
        assert_eq!(settings::load(&path).unwrap(), store.get());
    }
}
