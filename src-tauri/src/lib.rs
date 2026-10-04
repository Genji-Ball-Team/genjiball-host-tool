mod config;
mod credentials;
mod dpapi;
mod history;
mod log_folder;
mod log_scan;
mod server;
mod settings;
mod uploader;
mod uploads;
mod watcher;

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, State, WindowEvent};

use credentials::Tokens;
use history::Page;
use log_folder::LogFolder;
use server::TokenCheck;
use settings::Settings;
use uploader::{UploadStatus, Uploader};

/// The settings file and what's in it. Commands change both together.
struct Store {
    path: PathBuf,
    settings: Mutex<Settings>,
    /// Why the file couldn't be read at startup, if it couldn't. The defaults are used until the
    /// host changes a setting, which writes a fresh file.
    load_error: Mutex<Option<String>>,
    tokens: Tokens,
}

impl Store {
    fn get(&self) -> Settings {
        self.settings.lock().unwrap().clone()
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
}

fn app_state(app: &tauri::AppHandle, store: &Store) -> Result<AppState, String> {
    let settings = store.get();
    Ok(AppState {
        version: app.package_info().version.to_string(),
        server_url: settings.server_url().to_string(),
        default_server_url: config::DEFAULT_SERVER_URL,
        has_token: store.tokens.get(settings.server_url())?.is_some(),
        log_folder: log_folder::current(settings.log_folder.as_deref()),
        settings_error: store.load_error.lock().unwrap().clone(),
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
    let server_url = store.get().server_url().to_string();
    let token = store
        .tokens
        .get(&server_url)?
        .ok_or("No host token saved for this server")?;
    // The listed matches' status too: "Check again" after an admin accepted one.
    uploader.refresh();
    Ok(server::check_token(&server_url, &token).await)
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
    let server_url = store.get().server_url().to_string();
    let check = server::check_token(&server_url, token).await;
    if !check.is_rejected() {
        store.tokens.set(&server_url, token)?;
        // Uploads to this server start with the logs written from now on.
        uploader.start(&server_url);
    }
    Ok(check)
}

#[tauri::command]
fn forget_token(store: State<Store>, uploader: State<Uploader>) -> Result<(), String> {
    store.tokens.delete(store.get().server_url())?;
    uploader.wake();
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
    uploader.wake();
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
    uploader.wake();
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            let path = dir.join(config::SETTINGS_FILE);
            let (settings, load_error) = match settings::load(&path) {
                Ok(settings) => (settings, None),
                Err(e) => (Settings::default(), Some(e)),
            };
            app.manage(Store {
                path,
                settings: Mutex::new(settings),
                load_error: Mutex::new(load_error),
                tokens: Tokens::new(dir.join(config::TOKENS_FALLBACK_FILE)),
            });
            app.manage(Uploader::new(dir.join(config::UPLOADS_FILE)));
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
            get_upload_status,
            get_upload_history,
            retry_upload,
            open_match
        ])
        .run(tauri::generate_context!())
        .expect("error while running the host tool");
}
