mod config;
mod credentials;
mod dpapi;
mod log_folder;
mod server;
mod settings;

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{Manager, State};

use credentials::Tokens;
use log_folder::LogFolder;
use server::TokenCheck;
use settings::Settings;

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
async fn check_saved_token(store: State<'_, Store>) -> Result<TokenCheck, String> {
    let server_url = store.get().server_url().to_string();
    let token = store
        .tokens
        .get(&server_url)?
        .ok_or("No host token saved for this server")?;
    Ok(server::check_token(&server_url, &token).await)
}

/// Checks a token the host entered, and saves it unless the server turned it down. A server that
/// can't be reached doesn't stop the save: the host may be offline, and uploads will retry.
#[tauri::command]
async fn save_token(token: String, store: State<'_, Store>) -> Result<TokenCheck, String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("Paste the host token an admin gave you".into());
    }
    let server_url = store.get().server_url().to_string();
    let check = server::check_token(&server_url, token).await;
    if !check.is_rejected() {
        store.tokens.set(&server_url, token)?;
    }
    Ok(check)
}

#[tauri::command]
fn forget_token(store: State<Store>) -> Result<(), String> {
    store.tokens.delete(store.get().server_url())
}

/// Empty goes back to the default server.
#[tauri::command]
fn set_server_url(
    url: String,
    app: tauri::AppHandle,
    store: State<Store>,
) -> Result<AppState, String> {
    let url = settings::normalize_server_url(&url)?;
    store.update(|s| s.server_url = url)?;
    app_state(&app, &store)
}

/// `None` goes back to the detected folder.
#[tauri::command]
fn set_log_folder(
    path: Option<PathBuf>,
    app: tauri::AppHandle,
    store: State<Store>,
) -> Result<AppState, String> {
    store.update(|s| s.log_folder = path)?;
    app_state(&app, &store)
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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            check_saved_token,
            save_token,
            forget_token,
            set_server_url,
            set_log_folder
        ])
        .run(tauri::generate_context!())
        .expect("error while running the host tool");
}
