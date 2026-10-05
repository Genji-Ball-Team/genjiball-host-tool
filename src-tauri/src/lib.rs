mod config;
mod credentials;
mod dpapi;
mod history;
mod log_folder;
mod log_scan;
mod ranked_code;
mod release;
mod server;
mod settings;
mod uploader;
mod uploads;
mod watcher;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, State, WindowEvent};

use credentials::Tokens;
use history::Page;
use log_folder::LogFolder;
use release::ReleaseCache;
use server::TokenCheck;
use settings::Settings;
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
    settings_error: Option<String>,
    /// Every `config::TUNABLES`, in order, with the host's value.
    advanced: Vec<AdvancedSetting>,
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
    if matches!(check, TokenCheck::Ok { .. }) {
        // The server knows it now (an admin fixed it, say): stop holding uploads for it.
        uploader.token_ok();
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
) -> Result<TokenCheck, String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("Paste the host token an admin gave you".into());
    }
    let settings = store.get();
    let server_url = settings.server_url().to_string();
    let timeout = settings.secs(&config::REQUEST_TIMEOUT_SECS);
    let check = server::check_token(&server_url, token, timeout).await;
    if !check.is_rejected() {
        store.tokens.set(&server_url, token)?;
        // Uploads to this server start with the logs written from now on. A token saved again,
        // even the same one, is tried again.
        uploader.start(&server_url);
        uploader.token_ok();
        uploader.changed();
    }
    Ok(check)
}

#[tauri::command]
fn forget_token(store: State<Store>, uploader: State<Uploader>) -> Result<(), String> {
    store.tokens.delete(store.get().server_url())?;
    uploader.changed();
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
) -> Result<AppState, String> {
    let url = settings::normalize_server_url(&url)?;
    store.update(|s| s.server_url = url)?;
    uploader.changed();
    app_state(&app, &store)
}

/// `None` goes back to the detected folder.
#[tauri::command]
fn set_log_folder(
    path: Option<PathBuf>,
    app: tauri::AppHandle,
    store: State<Store>,
    uploader: State<Uploader>,
) -> Result<AppState, String> {
    store.update(|s| s.log_folder = path)?;
    uploader.changed();
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
) -> Result<AppState, String> {
    let values = settings::normalize_advanced(&values)?;
    store.update(|s| s.replace_advanced(values))?;
    // The next poll starts now, with the new values.
    uploader.wake();
    app_state(&app, &store)
}

/// The ranked Workshop code, for the window to copy.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RankedCode {
    code: String,
    /// The server the rank tags came from: the window drops a code built for a server the host
    /// has since switched away from.
    server_url: String,
    /// The GenjiBall-CE release it's built from (`1.3.3R`).
    release: String,
    /// When the server worked the rank tags out (ISO 8601).
    tags_updated_at: String,
    names: usize,
    /// Names the Workshop can't show, left out.
    skipped_names: usize,
    /// How long the window may keep a code it couldn't copy for the next click, before building
    /// a new one: as long as the release found is reused.
    keep_secs: u64,
}

/// The latest ranked release's code with the current server's rank tags in it.
#[tauri::command]
async fn build_ranked_code(
    store: State<'_, Store>,
    releases: State<'_, ReleaseCache>,
) -> Result<RankedCode, String> {
    let settings = store.get();
    let server_url = settings.server_url().to_string();
    let keep = settings.secs(&config::RELEASE_CACHE_SECS);
    let timeout = settings.secs(&config::REQUEST_TIMEOUT_SECS);
    let (release, base) = releases.latest(keep, timeout).await?;
    let tags = server::rank_tags(&server_url, timeout).await?;
    let filled = ranked_code::fill(&base, &tags)?;
    Ok(RankedCode {
        code: filled.code,
        server_url,
        release,
        tags_updated_at: tags.updated_at,
        names: filled.names,
        skipped_names: filled.skipped,
        keep_secs: keep.as_secs(),
    })
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // First, so a second launch stops here: it shows this window (maybe in the tray) rather
        // than running a second uploader on the same files.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_window(app)
        }))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            app.manage(Store::open(&dir));
            app.manage(Uploader::new(dir.join(config::UPLOADS_FILE)));
            app.manage(ReleaseCache::default());
            uploader::start(app.handle().clone());
            tray(app)?;
            Ok(())
        })
        // Closing the window keeps the tool uploading from the tray; Quit there stops it.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
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
            get_upload_status,
            build_ranked_code,
            get_upload_history,
            retry_upload,
            open_match
        ])
        .run(tauri::generate_context!())
        .expect("error while running the host tool");
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
