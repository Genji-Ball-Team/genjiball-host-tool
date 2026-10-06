//! The overlay's feed (#49–#55): everything the overlay window and the stream page show, read
//! fresh at each of their polls (`config::OVERLAY_POLL_MS`). Rust hands over the live log's
//! complete lines and the tool's own state; the window parses the log with the server's parser,
//! like the Match view (`src/overlay/model.ts`). Ratings come from the cache in `ratings.rs`, and
//! what it lacks is asked in the background.
//!
//! Nothing here touches the game: it reads the Workshop log folder and asks the ranked server, as
//! the uploader does.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::history::Entry;
use crate::match_log::{self, Known, LogText};
use crate::overlay_window::OverlayWindow;
use crate::ratings::{Job, PlayerRating, Ratings};
use crate::server::{self, MatchResult};
use crate::tourneys::Tourneys;
use crate::uploader::{Problem, Uploader};
use crate::{config, log_folder, log_scan, Store};

/// What the overlay keeps while the tool runs.
pub struct OverlayState {
    /// When the tool started, RFC 3339 in UTC: the session's matches are those uploaded since.
    started_at: String,
    /// When "Copy ranked code" last built the code, RFC 3339. Memory only: after a restart the
    /// overlay doesn't know.
    code_built_at: Mutex<Option<String>>,
    /// The match keys in each log file, by name, with the size they were read at.
    copies: Mutex<HashMap<String, (u64, Vec<String>)>>,
    pub ratings: Ratings,
}

impl Default for OverlayState {
    fn default() -> Self {
        OverlayState {
            started_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
            code_built_at: Mutex::default(),
            copies: Mutex::default(),
            ratings: Ratings::default(),
        }
    }
}

impl OverlayState {
    /// The ranked code was just built.
    pub fn code_built(&self) {
        *self.code_built_at.lock().unwrap() =
            Some(Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true));
    }
}

/// What the overlay asks for, from what it read in the log last time.
#[derive(Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FeedRequest {
    /// The live log as the overlay has it: its text isn't sent again until it grows.
    pub known: Option<Known>,
    /// The players in the lobby now, by their name in the log: the roster's ratings.
    pub names: Vec<String>,
    /// The key of the live log's last match: its log copies and its result.
    pub match_key: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Feed {
    /// For the stream page, not the overlay window.
    pub stream: bool,
    /// The widgets on, by key, in `config::OVERLAY_WIDGETS` order.
    pub widgets: Vec<&'static str>,
    pub opacity: u16,
    pub scale: u16,
    pub layout: BTreeMap<String, [f64; 2]>,
    /// The host is placing the widgets: they show sample data where there's none.
    pub editing: bool,
    pub hotkeys: Vec<HotkeyView>,
    pub poll_ms: u64,
    /// The live log, `None` while there's none. Its `text` is `None` when unchanged.
    pub log: Option<LogText>,
    /// When it last grew, RFC 3339.
    pub log_written_at: Option<String>,
    pub log_error: Option<String>,
    /// How long a log must stop growing before it counts as done, in seconds.
    pub quiet_secs: u64,
    pub host: HostView,
    pub ratings: Vec<PlayerRating>,
    /// The region the ratings are from: the one uploads go as.
    pub region: Option<String>,
    pub tourneys: Vec<TourneyView>,
    /// The live log's last match on the site, once it's public there.
    pub result: Option<MatchResult>,
    pub session: SessionView,
    /// Files holding the live match (`None` with its widget off).
    pub log_copies: Option<usize>,
    pub ranked_code: RankedCodeView,
    pub kill_feed_shown: usize,
    pub now: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyView {
    pub action: &'static str,
    pub label: &'static str,
    pub keys: Option<String>,
}

/// The upload status, without the history.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostView {
    pub problem: Option<Problem>,
    pub waiting: usize,
    pub retrying: Option<String>,
    pub dry_run: bool,
    pub afk: bool,
    /// The newest upload, or file waiting to be.
    pub last: Option<Entry>,
}

/// A tourney lobby the host is assigned to, whose key is known: the overlay finds a tourney
/// match's lobby by it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TourneyView {
    pub lobby_key: String,
    pub tourney: String,
    pub label: String,
    pub round_limit: u32,
    pub needs_screenshot: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub since: String,
    /// Matches uploaded since, public on the site or not.
    pub matches: usize,
    /// Those public on the site, newest first.
    pub results: Vec<MatchResult>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RankedCodeView {
    pub built_at: Option<String>,
    pub stale_secs: u64,
}

/// The feed now, for the overlay window or the stream page.
pub fn feed(app: &AppHandle, request: &FeedRequest, stream: bool) -> Feed {
    let store = app.state::<Store>();
    let uploader = app.state::<Uploader>();
    let overlay = app.state::<OverlayState>();
    let settings = store.get();
    let server_url = settings.server_url().to_string();
    let widgets = settings.overlay.widgets_on(stream);
    let folder = log_folder::current(settings.log_folder.as_deref()).filter(|f| f.exists);

    let (log, log_written_at, log_error) = match &folder {
        None => (
            None,
            None,
            Some("The Workshop log folder isn't there yet".to_string()),
        ),
        Some(folder) => match match_log::read_live(&folder.path, request.known.as_ref()) {
            Ok(log) => {
                let written = match_log::live_file(&folder.path)
                    .ok()
                    .flatten()
                    .map(|f| f.written_at);
                (log, written, None)
            }
            // Locked for a moment while the game writes it: the next poll reads it.
            Err(e) => (None, None, Some(e)),
        },
    };

    let status = uploader.status();
    let region = status.region.clone().or(settings.region.clone());
    let scope = (server_url.clone(), region.clone());
    let known = uploader.known_matches(&server_url);
    let id_of = |key: &str| {
        known
            .iter()
            .find(|k| k.match_key == key)
            .and_then(|k| k.match_id)
    };
    let current_id = request.match_key.as_deref().and_then(id_of);
    let since = &overlay.started_at;
    let session: Vec<_> = known
        .iter()
        .filter(|k| k.first_at.as_str() >= since.as_str())
        .collect();
    let mut wanted: Vec<i64> = current_id.into_iter().collect();
    if widgets.contains(&"session") {
        wanted.extend(
            session
                .iter()
                .filter(|k| server::is_public(&k.status))
                .filter_map(|k| k.match_id)
                .take(config::SESSION_MATCHES_MAX),
        );
    }
    let names = if widgets.contains(&"roster") {
        request.names.clone()
    } else {
        Vec::new()
    };
    let jobs = overlay
        .ratings
        .jobs(&scope, &names, &wanted, Instant::now());
    run_jobs(app, scope.clone(), jobs);

    let log_copies = match (&folder, &request.match_key) {
        (Some(folder), Some(key)) if widgets.contains(&"logCopies") => {
            Some(count_copies(&folder.path, key, &overlay.copies))
        }
        _ => None,
    };
    let tourneys = app
        .state::<Tourneys>()
        .status()
        .lobbies
        .into_iter()
        .filter_map(|l| {
            Some(TourneyView {
                lobby_key: l.lobby_key?,
                tourney: l.lobby.tourney.name,
                label: l.lobby.label,
                round_limit: l.lobby.round_limit,
                needs_screenshot: l.needs_screenshot,
            })
        })
        .collect();

    let code_built_at = overlay.code_built_at.lock().unwrap().clone();
    Feed {
        stream,
        widgets,
        opacity: settings.overlay.opacity(),
        scale: settings.overlay.scale(),
        layout: settings.overlay.layout.clone(),
        editing: !stream && app.state::<OverlayWindow>().editing(),
        hotkeys: config::OVERLAY_HOTKEYS
            .iter()
            .map(|h| HotkeyView {
                action: h.action,
                label: h.label,
                keys: settings.overlay.hotkey(h.action),
            })
            .collect(),
        poll_ms: config::OVERLAY_POLL_MS,
        log,
        log_written_at,
        log_error,
        quiet_secs: settings.get(&config::QUIET_SECS),
        host: HostView {
            problem: status.problem.clone(),
            waiting: status.waiting,
            retrying: status.retrying.clone(),
            dry_run: status.dry_run,
            afk: status.afk.on,
            last: status.history.entries.first().cloned(),
        },
        ratings: overlay.ratings.players(&scope, &names),
        region,
        tourneys,
        result: current_id.and_then(|id| overlay.ratings.match_result(id)),
        session: SessionView {
            since: since.clone(),
            matches: session.len(),
            results: wanted
                .iter()
                .skip(usize::from(current_id.is_some()))
                .filter_map(|&id| overlay.ratings.match_result(id))
                .collect(),
        },
        log_copies,
        ranked_code: RankedCodeView {
            built_at: code_built_at,
            stale_secs: config::RANKED_CODE_STALE_SECS,
        },
        kill_feed_shown: config::KILL_FEED_SHOWN,
        now: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
    }
}

/// Sends the cache's requests in the background, each with the answer stored when it comes.
fn run_jobs(app: &AppHandle, scope: (String, Option<String>), jobs: Vec<Job>) {
    for job in jobs {
        let app = app.clone();
        let scope = scope.clone();
        tauri::async_runtime::spawn(async move {
            let timeout = app
                .state::<Store>()
                .get()
                .secs(&config::REQUEST_TIMEOUT_SECS);
            let (server_url, region) = (&scope.0, scope.1.as_deref());
            let state = app.state::<OverlayState>();
            let ratings = &state.ratings;
            let failed = |e: String| {
                log::debug!("Overlay: {job:?} failed: {e}");
                ratings.failed(&scope, &job, Instant::now());
            };
            match &job {
                Job::Leaderboard => {
                    let mut players = Vec::new();
                    for page in 1..=config::RANKS_LEADERBOARD_PAGES {
                        match server::leaderboard_page(server_url, region, page, timeout).await {
                            Ok(read) => {
                                players.extend(read.players);
                                if !read.has_more {
                                    break;
                                }
                            }
                            Err(e) => return failed(e),
                        }
                    }
                    ratings.board_read(&scope, players, Instant::now());
                }
                Job::Search(name) => {
                    match server::search_players(server_url, region, name, timeout).await {
                        Ok(found) => ratings.searched(&scope, name, found, Instant::now()),
                        Err(e) => failed(e),
                    }
                }
                Job::Match(id) => match server::match_result(server_url, *id, timeout).await {
                    Ok(result) => ratings.match_read(&scope, *id, result, Instant::now()),
                    Err(e) => failed(e),
                },
            }
        });
    }
}

/// How many of the newest `LOG_COPIES_SCANNED` logs in `folder` hold the match `key`: Overwatch
/// starts a new file, repeating the log so far, each time the host moves to or from spectator.
/// A file is read again only once its size changed (`cache`).
pub fn count_copies(
    folder: &Path,
    key: &str,
    cache: &Mutex<HashMap<String, (u64, Vec<String>)>>,
) -> usize {
    let Ok(entries) = fs::read_dir(folder) else {
        return 0;
    };
    let mut names: Vec<(String, u64)> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            let meta = e.metadata().ok()?;
            (log_scan::is_log_file(&name) && meta.is_file()).then_some((name, meta.len()))
        })
        .collect();
    // Overwatch names them by when they start.
    names.sort_by(|a, b| b.0.cmp(&a.0));
    names.truncate(config::LOG_COPIES_SCANNED);
    let mut cache = cache.lock().unwrap();
    cache.retain(|name, _| names.iter().any(|(n, _)| n == name));
    names
        .iter()
        .filter(|(name, size)| {
            let fresh = cache.get(name).is_some_and(|(s, _)| s == size);
            if !fresh {
                // Locked for a moment while the game writes it: the next poll reads it.
                let Ok(bytes) = fs::read(folder.join(name)) else {
                    return false;
                };
                let keys = log_scan::scan(&String::from_utf8_lossy(&bytes)).match_keys;
                cache.insert(name.clone(), (*size, keys));
            }
            cache
                .get(name)
                .is_some_and(|(_, keys)| keys.iter().any(|k| k == key))
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(key: &str) -> String {
        format!("[00:00:01] GBR|1.00|2|1.3.3R|{key}\n[00:00:02] MATCH_START|2.00|workshop-island-night|Default|0|\n")
    }

    #[test]
    fn counts_the_files_a_match_is_split_over() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Mutex::default();
        fs::write(dir.path().join("Log-2026-10-06-20-00-00.txt"), log("111")).unwrap();
        fs::write(dir.path().join("Log-2026-10-06-20-10-00.txt"), log("222")).unwrap();
        // The host moved to spectator: a new file repeats the match.
        fs::write(dir.path().join("Log-2026-10-06-20-15-00.txt"), log("222")).unwrap();
        fs::write(dir.path().join("notes.txt"), log("222")).unwrap();
        assert_eq!(count_copies(dir.path(), "222", &cache), 2);
        assert_eq!(count_copies(dir.path(), "111", &cache), 1);
        assert_eq!(count_copies(dir.path(), "333", &cache), 0);

        // A file that grew is read again.
        fs::write(
            dir.path().join("Log-2026-10-06-20-00-00.txt"),
            log("111") + &log("333"),
        )
        .unwrap();
        assert_eq!(count_copies(dir.path(), "333", &cache), 1);
        assert_eq!(count_copies(&dir.path().join("missing"), "333", &cache), 0);
    }
}
