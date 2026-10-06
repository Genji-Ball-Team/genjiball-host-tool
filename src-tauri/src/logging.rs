//! The tool's own log, for bug reports: `config::LOG_FILE_NAME`.log in the app's log folder
//! (`%LOCALAPPDATA%\us.genjiball.hosttool\logs`), a new file every `LOG_MAX_BYTES`, the last
//! `LOG_FILES_KEPT` older ones kept. Written through `tauri-plugin-log` (Rust side only: the window
//! has no log permission) from the `log` macros.
//!
//! What's logged: polls that find logs to upload, each upload and the server's answer (its result
//! and status, not the body), retries and backoff, token checks (the result, never the token),
//! settings changes, update checks, ranked code builds, and errors. Never a token.

use log::{Level, LevelFilter, Metadata};
use tauri::plugin::TauriPlugin;
use tauri::Runtime;
use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

use crate::config;

/// This crate's log target prefix: other crates (the HTTP client, Tauri) only log warnings.
const OWN_TARGET: &str = env!("CARGO_CRATE_NAME");

/// A level from `config::LOG_LEVELS` as the `log` crate's filter. Settings are checked when
/// they're saved and read (`settings::normalize_log_level`), so anything else is the default.
pub fn level_filter(level: &str) -> LevelFilter {
    match level {
        "error" => LevelFilter::Error,
        "warn" => LevelFilter::Warn,
        "info" => LevelFilter::Info,
        "debug" => LevelFilter::Debug,
        _ => level_filter(config::DEFAULT_LOG_LEVEL),
    }
}

/// Whether a line is written at all (the level is checked before, by `log::max_level`): this
/// crate's at any level, other crates' warnings and errors only.
fn wanted(metadata: &Metadata) -> bool {
    metadata.target().starts_with(OWN_TARGET) || metadata.level() <= Level::Warn
}

/// The log plugin. Lets through the most detailed level the host can pick; `set_level` then
/// narrows it to the host's.
pub fn plugin<R: Runtime>() -> TauriPlugin<R> {
    let most = config::LOG_LEVELS[config::LOG_LEVELS.len() - 1];
    let builder = tauri_plugin_log::Builder::new()
        .clear_targets()
        .target(Target::new(TargetKind::LogDir {
            file_name: Some(config::LOG_FILE_NAME.into()),
        }))
        .max_file_size(config::LOG_MAX_BYTES.into())
        .rotation_strategy(RotationStrategy::KeepSome(config::LOG_FILES_KEPT))
        .timezone_strategy(TimezoneStrategy::UseLocal)
        .level(level_filter(most))
        .filter(wanted);
    // `npm run dev` shows it in the terminal too.
    #[cfg(debug_assertions)]
    let builder = builder.target(Target::new(TargetKind::Stdout));
    builder.build()
}

/// Logs at `level` (from `config::LOG_LEVELS`) from now on.
pub fn set_level(level: &str) {
    log::set_max_level(level_filter(level));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_level_has_a_filter() {
        let filters: Vec<_> = config::LOG_LEVELS.iter().map(|l| level_filter(l)).collect();
        // Least to most detailed, each its own.
        assert!(filters.windows(2).all(|w| w[0] < w[1]), "{filters:?}");
        assert_eq!(
            level_filter("nope"),
            level_filter(config::DEFAULT_LOG_LEVEL)
        );
    }

    #[test]
    fn other_crates_only_log_warnings() {
        let meta =
            |target: &'static str, level| Metadata::builder().target(target).level(level).build();
        assert!(wanted(&meta(
            "genjiball_host_tool_lib::uploader",
            Level::Debug
        )));
        assert!(!wanted(&meta("hyper_util::client", Level::Info)));
        assert!(wanted(&meta("tauri::manager", Level::Warn)));
    }
}
