//! Live lobby (#6), the loop: every poll it reads the live log (`lobby::Watch`), and lists the
//! host's lobby on the site while a ranked match is played in it, with heartbeats to
//! `PUT /api/host/lobby`, or takes it off with `DELETE` once the match ends, the game closes or the
//! host switches it off (`lobby::Schedule`). Runs for as long as the app does, window open or not,
//! apart from the upload loop: a lobby problem never holds an upload up.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

use crate::lobby::{self, Heartbeat, Live, LogFile, Schedule, Step, Watch};
use crate::server::{self, ListedLobby, LobbyOutcome};
use crate::settings::Settings;
use crate::{config, log_folder, watcher, Store};

/// The event the window listens to for `LobbyStatus`.
pub const STATUS_EVENT: &str = "lobby-status";

/// Why the lobby isn't listed, or isn't up to date.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LobbyProblem {
    /// The settings file can't be used (the window says why).
    Settings,
    NoFolder,
    FolderUnreadable {
        message: String,
    },
    NoToken,
    /// The server turned the token down. No heartbeat until the host changes it.
    TokenRejected {
        revoked: bool,
    },
    /// The host has no home region and didn't pick one.
    NoRegion,
    /// The last heartbeat or close didn't go through (offline, server down). Tried again.
    Failed {
        message: String,
    },
}

/// What the window shows about the live lobby.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LobbyStatus {
    /// The server this is about, as the settings were at the poll.
    pub server_url: String,
    /// Whether the host has the live lobby on.
    pub on: bool,
    /// What's being played in the live log (`Idle` while off: it isn't read then).
    pub live: Live,
    /// The lobby as the site lists it, from the last heartbeat the server took. `None`: not listed.
    pub listed: Option<ListedLobby>,
    pub problem: Option<LobbyProblem>,
}

/// The server and token a lobby is listed with. Memory only: the token never reaches the window.
#[derive(Clone, PartialEq)]
struct Target {
    server_url: String,
    token: String,
}

#[derive(Default)]
pub struct LiveLobby {
    status: Mutex<LobbyStatus>,
    /// Where the lobby is listed now, so quitting can take it off the list.
    listed_on: Mutex<Option<Target>>,
    /// A setting or the token changed: what was held or turned down for the old ones is tried again.
    changed: AtomicBool,
    wake: Notify,
}

impl LiveLobby {
    pub fn status(&self) -> LobbyStatus {
        self.status.lock().unwrap().clone()
    }

    /// A setting or the token changed: the next poll starts now, with them.
    pub fn changed(&self) {
        self.changed.store(true, Ordering::Relaxed);
        self.wake.notify_one();
    }
}

/// Starts the loop. Call once, after `Store` and `LiveLobby` are managed.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut run = Run::default();
        loop {
            let status = run.tick(&app).await;
            let lobby = app.state::<LiveLobby>();
            *lobby.listed_on.lock().unwrap() = run.listed_on();
            let changed = {
                let mut current = lobby.status.lock().unwrap();
                let changed = *current != status;
                *current = status.clone();
                changed
            };
            if changed {
                let _ = app.emit(STATUS_EVENT, status);
            }
            let interval = app.state::<Store>().get().secs(&config::POLL_INTERVAL_SECS);
            let _ = tokio::time::timeout(interval, lobby.wake.notified()).await;
        }
    });
}

/// The tool is quitting: takes a listed lobby off the list, waiting at most
/// `LOBBY_CLOSE_ON_QUIT_SECS`. Best effort: the server drops it after its TTL anyway.
pub fn close_on_quit(app: &AppHandle) {
    let Some(lobby) = app.try_state::<LiveLobby>() else {
        return;
    };
    let Some(target) = lobby.listed_on.lock().unwrap().take() else {
        return;
    };
    let timeout = Duration::from_secs(config::LOBBY_CLOSE_ON_QUIT_SECS);
    let _ = tauri::async_runtime::block_on(server::close_lobby(
        &target.server_url,
        &target.token,
        timeout,
    ));
}

/// `wait` after `now`; a wait too far off to count to is the lobby's TTL.
fn later(now: Instant, wait: Duration) -> Instant {
    now.checked_add(wait)
        .unwrap_or(now + Duration::from_secs(config::LOBBY_TTL_SECS))
}

#[derive(Default)]
struct Run {
    watch: Watch,
    schedule: Schedule,
    /// Where the last heartbeat the server took went.
    target: Option<Target>,
    /// What the site lists, from the last heartbeat the server took.
    listed: Option<ListedLobby>,
    /// The server and token that were turned down (and whether revoked): no heartbeat with them
    /// until a setting or the token changes.
    rejected: Option<(Target, bool)>,
    /// Why the last heartbeat or close didn't go through, until one does.
    failed: Option<LobbyProblem>,
}

impl Run {
    /// Where the lobby is listed, as far as the tool knows.
    fn listed_on(&self) -> Option<Target> {
        self.target.clone().filter(|_| self.schedule.is_listed())
    }

    async fn tick(&mut self, app: &AppHandle) -> LobbyStatus {
        let store = app.state::<Store>();
        if app
            .state::<LiveLobby>()
            .changed
            .swap(false, Ordering::Relaxed)
        {
            self.schedule.release();
            self.rejected = None;
            self.failed = None;
        }
        // First: a settings file fixed by hand is read again here.
        let settings_error = store.load_error();
        let settings = store.get();
        let server_url = settings.server_url().to_string();
        let timeout = settings.secs(&config::REQUEST_TIMEOUT_SECS);
        let mut status = LobbyStatus {
            server_url: server_url.clone(),
            on: settings.live_lobby_on(),
            ..LobbyStatus::default()
        };
        let token = match settings_error {
            Some(_) => Err(Some(LobbyProblem::Settings)),
            None => store
                .tokens
                .get(&server_url)
                .map_err(|message| Some(LobbyProblem::Failed { message })),
        };
        let wanted = token.and_then(|token| {
            let token = token.ok_or(Some(LobbyProblem::NoToken))?;
            self.wanted(&settings, Target { server_url, token }, &mut status)
        });
        let (want, target) = match wanted {
            Ok((target, beat)) => (Some(beat), Some(target)),
            Err(problem) => {
                status.problem = problem;
                (None, None)
            }
        };

        let wants_listing = want.is_some();
        let now = Instant::now();
        // Listed with another server or token: off that list first, as well as it goes.
        if let (Some(listed_on), Some(target)) = (self.listed_on(), &target) {
            if listed_on != *target {
                let _ = server::close_lobby(&listed_on.server_url, &listed_on.token, timeout).await;
                self.schedule.forget();
                self.target = None;
            }
        }
        match self.schedule.step(want.as_ref(), now) {
            Step::Wait => {}
            Step::Heartbeat => {
                if let (Some(target), Some(beat)) = (target, want) {
                    let outcome = server::lobby_heartbeat(
                        &target.server_url,
                        &target.token,
                        beat.region.as_deref(),
                        beat.players,
                        beat.name.as_deref(),
                        timeout,
                    )
                    .await;
                    let refresh_every = settings.secs(&config::STATUS_REFRESH_SECS);
                    self.heartbeat_answered(outcome, target, beat, now, refresh_every);
                }
            }
            Step::Close => match self.target.clone() {
                Some(listed_on) => {
                    let outcome =
                        server::close_lobby(&listed_on.server_url, &listed_on.token, timeout).await;
                    self.close_answered(outcome, now);
                }
                None => self.schedule.forget(),
            },
        }
        // Nothing listed and nothing to list: an old failure no longer matters.
        if !self.schedule.is_listed() && !wants_listing {
            self.failed = None;
        }
        status.listed = self.listed_on().and(self.listed.clone());
        if status.problem.is_none() {
            status.problem = self.failed.clone();
        }
        status
    }

    /// The heartbeat to send to `target` now, or why there's none (`None`: there's just no ranked
    /// match being played, or the host switched it off). Reads the live log into `status`.
    fn wanted(
        &mut self,
        settings: &Settings,
        target: Target,
        status: &mut LobbyStatus,
    ) -> Result<(Target, Heartbeat), Option<LobbyProblem>> {
        if !settings.live_lobby_on() {
            return Err(None);
        }
        if let Some((rejected, revoked)) = &self.rejected {
            if *rejected == target {
                return Err(Some(LobbyProblem::TokenRejected { revoked: *revoked }));
            }
        }
        let folder = log_folder::current(settings.log_folder.as_deref())
            .filter(|f| f.exists)
            .ok_or(Some(LobbyProblem::NoFolder))?;
        let newest = watcher::newest_log(&folder.path).map_err(|e| {
            Some(LobbyProblem::FolderUnreadable {
                message: e.to_string(),
            })
        })?;
        let file = newest.as_ref().and_then(|path| {
            let meta = std::fs::metadata(path).ok()?;
            let age = meta
                .modified()
                .ok()
                .and_then(|m| m.elapsed().ok())
                .unwrap_or_default();
            Some((
                path,
                path.file_name()?.to_str()?.to_string(),
                meta.len(),
                age,
            ))
        });
        let quiet = settings.secs(&config::QUIET_SECS);
        status.live = self.watch.look(
            file.as_ref().map(|(_, name, size, age)| LogFile {
                name,
                size: *size,
                age: *age,
            }),
            Instant::now(),
            quiet,
            || {
                let (path, ..) = file.as_ref()?;
                std::fs::read(path)
                    .ok()
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            },
        );
        match status.live {
            Live::Playing { players } => Ok((
                target,
                Heartbeat {
                    players,
                    name: settings.lobby_name.clone(),
                    region: settings.region.clone(),
                },
            )),
            Live::Idle | Live::Unranked => Err(None),
        }
    }

    /// Records what came of a heartbeat of `beat` to `target`, sent at `now`. A host without a
    /// region is asked again every `no_region_wait` (an admin may set it), or when a setting changes.
    fn heartbeat_answered(
        &mut self,
        outcome: LobbyOutcome,
        target: Target,
        beat: Heartbeat,
        now: Instant,
        no_region_wait: Duration,
    ) {
        let min_gap = Duration::from_secs(config::LOBBY_HEARTBEAT_MIN_SECS);
        match outcome {
            LobbyOutcome::Listed(answer) => {
                let (every, ttl) = lobby::timing(answer.heartbeat_seconds, answer.ttl_seconds);
                self.schedule.listed(beat, now, every, ttl);
                self.target = Some(target);
                self.listed = Some(answer.lobby);
                self.failed = None;
            }
            LobbyOutcome::RateLimited { after } => {
                // Nothing was written: not an error, just later.
                self.schedule.hold(later(now, after.unwrap_or(min_gap)));
            }
            LobbyOutcome::NoRegion => {
                self.schedule.hold(later(now, no_region_wait));
                self.failed = Some(LobbyProblem::NoRegion);
            }
            LobbyOutcome::TokenRejected { revoked } => {
                // A revoked host's lobby isn't listed, and an unknown token has none.
                self.schedule.forget();
                self.target = None;
                self.rejected = Some((target, revoked));
                self.failed = None;
            }
            LobbyOutcome::Failed { message } => {
                self.schedule.hold(later(now, min_gap));
                self.failed = Some(LobbyProblem::Failed { message });
            }
            LobbyOutcome::Closed => {
                self.schedule.hold(later(now, min_gap));
                self.failed = Some(LobbyProblem::Failed {
                    message: "The server's answer wasn't what the host tool expected".into(),
                });
            }
        }
    }

    /// Records what came of a close sent at `now`.
    fn close_answered(&mut self, outcome: LobbyOutcome, now: Instant) {
        let min_gap = Duration::from_secs(config::LOBBY_HEARTBEAT_MIN_SECS);
        match outcome {
            LobbyOutcome::Closed => {
                self.schedule.closed(now);
                self.listed = None;
                self.failed = None;
            }
            // Not listed with that token any more either way.
            LobbyOutcome::TokenRejected { .. } => {
                self.schedule.forget();
                self.listed = None;
            }
            LobbyOutcome::RateLimited { after } => {
                self.schedule.hold(later(now, after.unwrap_or(min_gap)));
            }
            LobbyOutcome::Failed { message } => {
                self.schedule.hold(later(now, min_gap));
                self.failed = Some(LobbyProblem::Failed { message });
            }
            LobbyOutcome::Listed(_) | LobbyOutcome::NoRegion => {
                self.schedule.hold(later(now, min_gap));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REFRESH: Duration = Duration::from_secs(config::STATUS_REFRESH_SECS.default);
    const MIN_GAP: Duration = Duration::from_secs(config::LOBBY_HEARTBEAT_MIN_SECS);

    fn target(token: &str) -> Target {
        Target {
            server_url: "https://genjiball.us".into(),
            token: token.into(),
        }
    }

    fn beat() -> Heartbeat {
        Heartbeat {
            players: 6,
            name: None,
            region: None,
        }
    }

    fn listed() -> LobbyOutcome {
        LobbyOutcome::Listed(server::LobbyAnswer {
            lobby: ListedLobby {
                region: "eu".into(),
                name: None,
                players: 6,
            },
            heartbeat_seconds: Some(config::LOBBY_HEARTBEAT_SECS),
            ttl_seconds: Some(config::LOBBY_TTL_SECS),
        })
    }

    #[test]
    fn a_taken_heartbeat_lists_the_lobby_until_its_closed() {
        let mut run = Run::default();
        let now = Instant::now();
        run.heartbeat_answered(listed(), target("t"), beat(), now, REFRESH);
        assert!(run.listed_on() == Some(target("t")));
        assert_eq!(run.listed.as_ref().map(|l| l.region.as_str()), Some("eu"));
        assert_eq!(run.schedule.step(None, now), Step::Close);
        run.close_answered(LobbyOutcome::Closed, now);
        assert!(run.listed_on().is_none());
        assert_eq!(run.schedule.step(None, now), Step::Wait);
    }

    #[test]
    fn too_soon_waits_without_an_error() {
        let mut run = Run::default();
        let now = Instant::now();
        let after = Duration::from_secs(12);
        run.heartbeat_answered(
            LobbyOutcome::RateLimited { after: Some(after) },
            target("t"),
            beat(),
            now,
            REFRESH,
        );
        assert_eq!(run.failed, None);
        assert_eq!(
            run.schedule.step(Some(&beat()), now + after / 2),
            Step::Wait
        );
        assert_eq!(
            run.schedule.step(Some(&beat()), now + after),
            Step::Heartbeat
        );
    }

    #[test]
    fn a_turned_down_token_stops_heartbeats_with_it() {
        let mut run = Run::default();
        let now = Instant::now();
        run.heartbeat_answered(listed(), target("t"), beat(), now, REFRESH);
        run.heartbeat_answered(
            LobbyOutcome::TokenRejected { revoked: true },
            target("t"),
            beat(),
            now + MIN_GAP,
            REFRESH,
        );
        assert!(run.listed_on().is_none());
        assert!(matches!(&run.rejected, Some((t, true)) if *t == target("t")));
        let mut status = LobbyStatus::default();
        assert_eq!(
            run.wanted(&Settings::default(), target("t"), &mut status)
                .err(),
            Some(Some(LobbyProblem::TokenRejected { revoked: true }))
        );
    }

    #[test]
    fn a_host_without_a_region_is_asked_again_later() {
        let mut run = Run::default();
        let now = Instant::now();
        run.heartbeat_answered(LobbyOutcome::NoRegion, target("t"), beat(), now, REFRESH);
        assert_eq!(run.failed, Some(LobbyProblem::NoRegion));
        assert_eq!(run.schedule.step(Some(&beat()), now + MIN_GAP), Step::Wait);
        assert_eq!(
            run.schedule.step(Some(&beat()), now + REFRESH),
            Step::Heartbeat
        );
    }

    #[test]
    fn a_failed_close_is_tried_again() {
        let mut run = Run::default();
        let now = Instant::now();
        run.heartbeat_answered(listed(), target("t"), beat(), now, REFRESH);
        run.close_answered(
            LobbyOutcome::Failed {
                message: "offline".into(),
            },
            now,
        );
        assert!(run.listed_on().is_some());
        assert!(matches!(run.failed, Some(LobbyProblem::Failed { .. })));
        assert_eq!(run.schedule.step(None, now + MIN_GAP), Step::Close);
    }

    #[test]
    fn switched_off_reads_nothing() {
        let mut run = Run::default();
        let mut settings = Settings::default();
        settings.set_live_lobby(false);
        let mut status = LobbyStatus::default();
        assert_eq!(
            run.wanted(&settings, target("t"), &mut status).err(),
            Some(None)
        );
        assert_eq!(status.live, Live::Idle);
    }

    #[test]
    fn heartbeats_while_a_match_is_played_in_the_newest_log() {
        let dir = tempfile::tempdir().unwrap();
        let example = include_str!("../tests/fixtures/ranked-log-example.txt");
        let playing = &example[..example.find("[00:01:54] MATCH_END").unwrap()];
        std::fs::write(dir.path().join("Log-2026-10-02-20-00-00.txt"), example).unwrap();
        std::fs::write(dir.path().join("Log-2026-10-02-20-15-33.txt"), playing).unwrap();
        let settings = Settings {
            log_folder: Some(dir.path().to_path_buf()),
            region: Some("na".into()),
            lobby_name: Some("Late night".into()),
            ..Settings::default()
        };
        let mut run = Run::default();
        let mut status = LobbyStatus::default();
        let (to, sent) = run.wanted(&settings, target("t"), &mut status).unwrap();
        assert!(to == target("t"));
        assert_eq!(
            sent,
            Heartbeat {
                players: 5,
                name: Some("Late night".into()),
                region: Some("na".into()),
            }
        );
        assert_eq!(status.live, Live::Playing { players: 5 });

        // The match ends.
        std::fs::write(dir.path().join("Log-2026-10-02-20-15-33.txt"), example).unwrap();
        assert_eq!(
            run.wanted(&settings, target("t"), &mut status).err(),
            Some(None)
        );
        assert_eq!(status.live, Live::Idle);
    }

    #[test]
    fn serializes_for_the_frontend() {
        let status = LobbyStatus {
            server_url: "https://genjiball.us".into(),
            on: true,
            live: Live::Playing { players: 6 },
            listed: Some(ListedLobby {
                region: "eu".into(),
                name: None,
                players: 6,
            }),
            problem: Some(LobbyProblem::TokenRejected { revoked: false }),
        };
        assert_eq!(
            serde_json::to_value(status).unwrap(),
            serde_json::json!({
                "serverUrl": "https://genjiball.us",
                "on": true,
                "live": { "kind": "playing", "players": 6 },
                "listed": { "region": "eu", "name": null, "players": 6 },
                "problem": { "kind": "tokenRejected", "revoked": false },
            })
        );
    }
}
