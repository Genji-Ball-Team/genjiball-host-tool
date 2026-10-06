//! Ratings for the overlay (#51, #53), from the public API: the lobby's players' standing in the
//! region in use, and a finished match's rating changes. Answers come from the cache at once; what
//! the cache lacks is asked in the background (`Ratings::jobs`, run by `overlay.rs`), so a slow
//! server never holds the overlay up.
//!
//! Light on the server's database (genjiball-ranked `docs/database.md`, "Free tier"): the
//! leaderboard is read a few pages at a time (cached at the edge), and a name that isn't on it is
//! searched once per `RANKS_UNKNOWN_SECS`, since each search reads every alias.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::config;
use crate::server::{FoundPlayer, MatchResult, Standing};

/// A player of the lobby, as the roster shows them.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerRating {
    /// The name asked about: their name in the log.
    pub name: String,
    pub state: RatingState,
    /// Their standing, while `found`.
    pub standing: Option<Standing>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RatingState {
    /// Not asked yet, or being asked.
    Pending,
    Found,
    /// The server has no player by that name (or old name) rated in the region: a new player.
    Unknown,
}

/// A request the cache needs sent.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Job {
    /// The first `RANKS_LEADERBOARD_PAGES` pages of the leaderboard.
    Leaderboard,
    /// `/api/players?search=` for a name not on the leaderboard, by its name key: the server
    /// searches ignoring case, so `Ghost` and `ghost` are one search.
    Search(String),
    /// `/api/matches/:id`.
    Match(i64),
}

/// Whose ratings the cache holds: they're dropped when the server or region changes.
type Scope = (String, Option<String>);

#[derive(Default)]
pub struct Ratings {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    scope: Option<Scope>,
    /// By name key: everyone on the leaderboard pages read, and when.
    board: Option<(Instant, HashMap<String, Standing>)>,
    /// Names searched, by name key: their standing (`None`: unknown), and when.
    searched: HashMap<String, (Instant, Option<Standing>)>,
    /// When a request last failed, by job: tried again after `MATCH_RATINGS_RETRY_SECS`.
    failed: HashMap<Job, Instant>,
    /// Matches by id: the result (`None`: not public yet), and when it was read.
    matches: HashMap<i64, (Instant, Option<MatchResult>)>,
    running: HashSet<Job>,
}

/// Names map to players ignoring case, as on the server.
fn key(name: &str) -> String {
    name.to_lowercase()
}

impl Ratings {
    /// What's known of `names` in `scope` now.
    pub fn players(&self, scope: &Scope, names: &[String]) -> Vec<PlayerRating> {
        let mut inner = self.inner.lock().unwrap();
        inner.rescope(scope);
        names
            .iter()
            .map(|name| {
                let k = key(name);
                let on_board = inner.board.as_ref().and_then(|(_, b)| b.get(&k));
                let searched = inner.searched.get(&k).map(|(_, s)| s);
                let (state, standing) = match (on_board, searched) {
                    (Some(s), _) | (None, Some(Some(s))) => (RatingState::Found, Some(s.clone())),
                    (None, Some(None)) => (RatingState::Unknown, None),
                    (None, None) => (RatingState::Pending, None),
                };
                PlayerRating {
                    name: name.clone(),
                    state,
                    standing,
                }
            })
            .collect()
    }

    /// A match's result as last read, `None` while it isn't (or isn't public yet).
    pub fn match_result(&self, id: i64) -> Option<MatchResult> {
        let inner = self.inner.lock().unwrap();
        inner.matches.get(&id).and_then(|(_, m)| m.clone())
    }

    /// The requests to send for `names` and `matches` in `scope` at `now`, marked as running: each
    /// is sent once at a time, then `done` with its answer.
    pub fn jobs(&self, scope: &Scope, names: &[String], matches: &[i64], now: Instant) -> Vec<Job> {
        let mut inner = self.inner.lock().unwrap();
        inner.rescope(scope);
        let mut jobs = Vec::new();
        let fresh = |at: Instant, secs: u64| now.duration_since(at) < Duration::from_secs(secs);
        let board_fresh = inner
            .board
            .as_ref()
            .is_some_and(|(at, _)| fresh(*at, config::RANKS_CACHE_SECS));
        if !names.is_empty() && !board_fresh {
            jobs.push(Job::Leaderboard);
        }
        // Searched only once the leaderboard's read: most lobbies are on it.
        if let Some((_, board)) = &inner.board {
            for name in names {
                let k = key(name);
                let searched = inner.searched.get(&k).is_some_and(|(at, found)| {
                    let keep = match found {
                        Some(_) => config::RANKS_CACHE_SECS,
                        None => config::RANKS_UNKNOWN_SECS,
                    };
                    fresh(*at, keep)
                });
                let searchable = name.trim().chars().count() >= config::PLAYER_SEARCH_MIN_CHARS;
                if !board.contains_key(&k) && !searched && searchable {
                    jobs.push(Job::Search(k));
                }
            }
        }
        for &id in matches {
            let due = match inner.matches.get(&id) {
                // Rated: it won't change.
                Some((_, Some(m))) if m.rated() => false,
                Some((at, _)) => !fresh(*at, config::MATCH_RATINGS_RETRY_SECS),
                None => true,
            };
            if due {
                jobs.push(Job::Match(id));
            }
        }
        // Two players with one name, anywhere in the lobby: one search.
        let mut seen = HashSet::new();
        jobs.retain(|job| seen.insert(job.clone()));
        jobs.retain(|job| {
            let backing_off = inner
                .failed
                .get(job)
                .is_some_and(|at| fresh(*at, config::MATCH_RATINGS_RETRY_SECS));
            !backing_off && !inner.running.contains(job)
        });
        inner.running.extend(jobs.iter().cloned());
        jobs
    }

    /// The leaderboard pages read for `scope`.
    pub fn board_read(&self, scope: &Scope, players: Vec<Standing>, now: Instant) {
        self.done(scope, &Job::Leaderboard, |inner| {
            let board = players.into_iter().map(|s| (key(&s.name), s)).collect();
            inner.board = Some((now, board));
        });
    }

    /// What a search for `name` found: the player with that name, or that old name.
    pub fn searched(&self, scope: &Scope, name: &str, found: Vec<FoundPlayer>, now: Instant) {
        let k = key(name);
        let player = found
            .into_iter()
            .find(|f| {
                key(&f.standing.name) == k || f.matched_alias.as_deref().map(key) == Some(k.clone())
            })
            .map(|f| f.standing);
        self.done(scope, &Job::Search(name.to_string()), |inner| {
            inner.searched.insert(k.clone(), (now, player));
        });
    }

    pub fn match_read(&self, scope: &Scope, id: i64, result: Option<MatchResult>, now: Instant) {
        self.done(scope, &Job::Match(id), |inner| {
            inner.matches.insert(id, (now, result));
        });
    }

    /// A request failed: tried again after a while.
    pub fn failed(&self, scope: &Scope, job: &Job, now: Instant) {
        self.done(scope, job, |inner| {
            inner.failed.insert(job.clone(), now);
        });
    }

    fn done(&self, scope: &Scope, job: &Job, store: impl FnOnce(&mut Inner)) {
        let mut inner = self.inner.lock().unwrap();
        inner.running.remove(job);
        // The settings changed while it was asked: it's about others.
        if inner.scope.as_ref() == Some(scope) {
            inner.failed.remove(job);
            store(&mut inner);
        }
    }
}

impl Inner {
    fn rescope(&mut self, scope: &Scope) {
        if self.scope.as_ref() != Some(scope) {
            *self = Inner {
                scope: Some(scope.clone()),
                ..Inner::default()
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::MatchPlayer;

    fn scope() -> Scope {
        ("https://genjiball.us".into(), Some("eu".into()))
    }

    fn standing(name: &str, rating: f64) -> Standing {
        Standing {
            id: 1,
            name: name.into(),
            rank: Some(4),
            rating: Some(rating),
            tier: None,
        }
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn reads_the_leaderboard_then_searches_only_the_names_not_on_it() {
        let ratings = Ratings::default();
        let now = Instant::now();
        let lobby = names(&["Kenzo", "nova", "x"]);
        assert_eq!(ratings.jobs(&scope(), &lobby, &[], now), [Job::Leaderboard]);
        // Already being asked.
        assert!(ratings.jobs(&scope(), &lobby, &[], now).is_empty());
        assert!(ratings
            .players(&scope(), &lobby)
            .iter()
            .all(|p| p.state == RatingState::Pending));

        ratings.board_read(&scope(), vec![standing("kenzo", 1700.0)], now);
        // "x" is too short to search: the server would refuse it.
        assert_eq!(
            ratings.jobs(&scope(), &lobby, &[], now),
            [Job::Search("nova".into())]
        );
        let found = FoundPlayer {
            standing: standing("Nova", 1400.0),
            matched_alias: None,
        };
        ratings.searched(&scope(), "nova", vec![found], now);
        let players = ratings.players(&scope(), &lobby);
        assert_eq!(players[0].state, RatingState::Found);
        assert_eq!(players[1].standing.as_ref().unwrap().rating, Some(1400.0));
        assert_eq!(players[2].state, RatingState::Pending);
        assert!(ratings.jobs(&scope(), &lobby, &[], now).is_empty());

        // The cache runs out.
        let later = now + Duration::from_secs(config::RANKS_CACHE_SECS + 1);
        let jobs = ratings.jobs(&scope(), &lobby, &[], later);
        assert!(jobs.contains(&Job::Leaderboard) && jobs.contains(&Job::Search("nova".into())));
    }

    #[test]
    fn searches_a_name_once_whatever_its_case_or_place() {
        let ratings = Ratings::default();
        let now = Instant::now();
        let lobby = names(&["Ghost", "Nova", "ghost"]);
        ratings.jobs(&scope(), &lobby, &[], now);
        ratings.board_read(&scope(), vec![standing("Nova", 1500.0)], now);
        assert_eq!(
            ratings.jobs(&scope(), &lobby, &[], now),
            [Job::Search("ghost".into())]
        );
        let found = FoundPlayer {
            standing: standing("Ghost", 1600.0),
            matched_alias: None,
        };
        ratings.searched(&scope(), "ghost", vec![found], now);
        let players = ratings.players(&scope(), &lobby);
        assert_eq!(players[0].state, RatingState::Found);
        assert_eq!(players[2].state, RatingState::Found);
    }

    #[test]
    fn a_name_the_server_doesnt_know_is_new_and_kept_longer() {
        let ratings = Ratings::default();
        let now = Instant::now();
        let lobby = names(&["Fresh"]);
        ratings.jobs(&scope(), &lobby, &[], now);
        ratings.board_read(&scope(), vec![], now);
        ratings.jobs(&scope(), &lobby, &[], now);
        // Only someone else's name holds it.
        let other = FoundPlayer {
            standing: standing("Freshman", 1500.0),
            matched_alias: None,
        };
        ratings.searched(&scope(), "Fresh", vec![other], now);
        assert_eq!(
            ratings.players(&scope(), &lobby)[0].state,
            RatingState::Unknown
        );
        let later = now + Duration::from_secs(config::RANKS_CACHE_SECS + 1);
        assert_eq!(
            ratings.jobs(&scope(), &lobby, &[], later),
            [Job::Leaderboard]
        );
    }

    #[test]
    fn an_old_name_finds_the_player() {
        let ratings = Ratings::default();
        let now = Instant::now();
        let found = FoundPlayer {
            standing: standing("Kenzo", 1800.0),
            matched_alias: Some("OldKenzo".into()),
        };
        ratings.players(&scope(), &[]);
        ratings.searched(&scope(), "oldkenzo", vec![found], now);
        let players = ratings.players(&scope(), &names(&["OldKenzo"]));
        assert_eq!(players[0].standing.as_ref().unwrap().name, "Kenzo");
    }

    #[test]
    fn another_region_starts_afresh_and_drops_late_answers() {
        let ratings = Ratings::default();
        let now = Instant::now();
        let lobby = names(&["Kenzo"]);
        ratings.jobs(&scope(), &lobby, &[], now);
        let na = ("https://genjiball.us".to_string(), Some("na".to_string()));
        assert_eq!(ratings.jobs(&na, &lobby, &[], now), [Job::Leaderboard]);
        // The EU answer comes in late.
        ratings.board_read(&scope(), vec![standing("Kenzo", 1700.0)], now);
        assert_eq!(ratings.players(&na, &lobby)[0].state, RatingState::Pending);
    }

    #[test]
    fn a_match_is_asked_again_until_its_rated() {
        let ratings = Ratings::default();
        let now = Instant::now();
        assert_eq!(ratings.jobs(&scope(), &[], &[12], now), [Job::Match(12)]);
        ratings.match_read(&scope(), 12, None, now);
        assert!(ratings.jobs(&scope(), &[], &[12], now).is_empty());
        let later = now + Duration::from_secs(config::MATCH_RATINGS_RETRY_SECS + 1);
        assert_eq!(ratings.jobs(&scope(), &[], &[12], later), [Job::Match(12)]);
        let player = |after| MatchPlayer {
            name: "Kenzo".into(),
            round_wins: Some(1),
            kills: Some(2),
            place: Some(1),
            rating_before: Some(1500.0),
            rating_after: after,
        };
        let result = |after| MatchResult {
            id: 12,
            played_at: None,
            rounds: 3,
            rated_rounds: 3,
            players: vec![player(after)],
        };
        ratings.match_read(&scope(), 12, Some(result(None)), later);
        assert!(!ratings.match_result(12).unwrap().rated());
        let much_later = later + Duration::from_secs(config::MATCH_RATINGS_RETRY_SECS + 1);
        assert_eq!(
            ratings.jobs(&scope(), &[], &[12], much_later),
            [Job::Match(12)]
        );
        ratings.match_read(&scope(), 12, Some(result(Some(1520.0))), much_later);
        let forever = much_later + Duration::from_secs(24 * 3600);
        assert!(ratings.jobs(&scope(), &[], &[12], forever).is_empty());
    }

    #[test]
    fn a_failed_request_waits_before_its_tried_again() {
        let ratings = Ratings::default();
        let now = Instant::now();
        let lobby = names(&["Kenzo"]);
        ratings.jobs(&scope(), &lobby, &[], now);
        ratings.failed(&scope(), &Job::Leaderboard, now);
        assert!(ratings.jobs(&scope(), &lobby, &[], now).is_empty());
        let later = now + Duration::from_secs(config::MATCH_RATINGS_RETRY_SECS + 1);
        assert_eq!(
            ratings.jobs(&scope(), &lobby, &[], later),
            [Job::Leaderboard]
        );
    }
}
