mod config;

use serde::Serialize;

/// What the frontend shows about the running app.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    version: String,
    server_url: String,
}

fn app_info_for(version: &str) -> AppInfo {
    AppInfo {
        version: version.to_string(),
        server_url: config::DEFAULT_SERVER_URL.to_string(),
    }
}

#[tauri::command]
fn app_info(app: tauri::AppHandle) -> AppInfo {
    app_info_for(&app.package_info().version.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![app_info])
        .run(tauri::generate_context!())
        .expect("error while running the host tool");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_info_uses_the_default_server() {
        assert_eq!(
            app_info_for("1.2.3"),
            AppInfo {
                version: "1.2.3".into(),
                server_url: "https://genjiball.us".into(),
            }
        );
    }

    #[test]
    fn app_info_serializes_in_camel_case() {
        let json = serde_json::to_value(app_info_for("1.2.3")).unwrap();
        assert_eq!(json["serverUrl"], "https://genjiball.us");
    }
}
