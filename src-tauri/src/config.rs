//! Every tunable and its default. Code reads these, never a literal.
//! The ones a host can change (`Tunable`, listed in `TUNABLES`) are under Advanced in the window;
//! the settings file holds only what the host changed, so a new default reaches every host.

/// A number a host can change under Advanced, in seconds: its default and the range the window
/// takes. `key` is its name in `settings.json` and between Rust and the window.
#[derive(Debug, PartialEq)]
pub struct Tunable {
    pub key: &'static str,
    pub label: &'static str,
    /// What it does, for the window.
    pub help: &'static str,
    pub default: u64,
    pub min: u64,
    pub max: u64,
}

/// The ranked server uploads go to. Overridable for local testing or the test server.
pub const DEFAULT_SERVER_URL: &str = "https://genjiball.us";

/// A region ranked keeps apart (its own ratings, leaderboard and rank tags): `id` is what the
/// server takes (`X-Region`, `?region=`), `label` what the window shows.
#[derive(Debug, PartialEq, serde::Serialize)]
pub struct Region {
    pub id: &'static str,
    pub label: &'static str,
}

/// The regions the host picks from: the server's `regions` (genjiball-ranked `src/config.ts`).
pub const REGIONS: [Region; 2] = [
    Region {
        id: "eu",
        label: "Europe",
    },
    Region {
        id: "na",
        label: "North America",
    },
];

/// How long a request to the ranked server (or GitHub) may take before it counts as unreachable.
pub const REQUEST_TIMEOUT_SECS: Tunable = Tunable {
    key: "requestTimeoutSecs",
    label: "Request timeout",
    help: "How long the server or GitHub may take to answer before the tool gives up and retries.",
    default: 15,
    min: 1,
    max: 300,
};

/// Where Overwatch writes Workshop logs, under the user's Documents folder.
pub const WORKSHOP_LOG_SUBFOLDER: [&str; 2] = ["Overwatch", "Workshop"];

/// The settings file, in the app's config folder (`%APPDATA%\us.genjiball.hosttool`).
pub const SETTINGS_FILE: &str = "settings.json";

/// The name host tokens are stored under in the OS credential store (one entry per server URL).
pub const CREDENTIAL_SERVICE: &str = "genjiball-host-tool";

/// Where the host token goes, encrypted for this Windows user, when the credential store won't
/// take it. In the app's config folder, next to the settings.
pub const TOKENS_FALLBACK_FILE: &str = "tokens.json";

/// The record of what was uploaded (per server, per log file), in the app's config folder. It's
/// what keeps a restart from uploading every file again, and what the window lists.
pub const UPLOADS_FILE: &str = "uploads.json";

/// Workshop log files are `Log-<date>-<time>.txt`; anything else in the folder is left alone.
pub const LOG_FILE_PREFIX: &str = "Log-";
pub const LOG_FILE_SUFFIX: &str = ".txt";

/// How often the log folder is checked for new and growing files. Each check only reads file
/// sizes; a file is read again only when it changed.
pub const POLL_INTERVAL_SECS: Tunable = Tunable {
    key: "pollIntervalSecs",
    label: "Log folder check",
    help: "How often the log folder is checked for new and growing logs.",
    default: 5,
    min: 1,
    max: 60,
};

/// How long a ranked log must stop growing before it's uploaded without a `MATCH_END`: the host
/// moved to spectator (Overwatch carries on in a new file), closed the lobby or crashed.
pub const QUIET_SECS: Tunable = Tunable {
    key: "quietSecs",
    label: "Quiet time",
    help: "How long a log must stop growing before it's uploaded without the match's end: you moved to spectator, closed the lobby or the game crashed.",
    default: 60,
    min: 10,
    max: 3600,
};

/// The server refuses bigger uploads (`maxUploadBytes` in genjiball-ranked).
pub const MAX_UPLOAD_BYTES: u64 = 512 * 1024;

/// The wait before retrying a failed upload (server down, offline), doubled after each failure
/// of the same file up to `RETRY_MAX_SECS`. A `Retry-After` from the server wins, however long; on
/// a `429` it holds every upload to that server.
pub const RETRY_FIRST_SECS: Tunable = Tunable {
    key: "retryFirstSecs",
    label: "First retry",
    help: "How long a failed upload waits before it's tried again. The wait doubles after each failure.",
    default: 30,
    min: 5,
    max: 3600,
};
pub const RETRY_MAX_SECS: Tunable = Tunable {
    key: "retryMaxSecs",
    label: "Longest retry wait",
    help: "The wait between retries stops doubling here. A wait the server asks for wins, however long.",
    default: 30 * 60,
    min: 5,
    max: 24 * 3600,
};

/// How many uploads the window lists on a page of the upload history, newest first. Older ones are
/// on the next pages.
pub const UPLOADS_PAGE_SIZE: usize = 8;

/// How often the status of the newest matches (an admin accepting one in review, say) and the
/// host's trust are asked of the server again.
pub const STATUS_REFRESH_SECS: Tunable = Tunable {
    key: "statusRefreshSecs",
    label: "Status refresh",
    help: "How often the server is asked again about your trust and your newest matches (an admin accepting one, say).",
    default: 120,
    min: 30,
    max: 3600,
};

/// The server answers at most this many match keys at once (`hostMatchKeysMax` in genjiball-ranked):
/// the newest this many matches get their status refreshed.
pub const MAX_STATUS_KEYS: usize = 50;

/// The GitHub API the ranked code's base release is looked up on.
pub const GITHUB_API_URL: &str = "https://api.github.com";

/// The repo (`owner/name`) whose releases hold the game's Workshop code.
pub const RELEASE_REPO: &str = "Genji-Ball-Team/GenjiBall-CE";

/// Only releases whose tag ends in this are ranked builds (`1.3.3R`); the latest one is the base
/// of the ranked code.
pub const RELEASE_TAG_SUFFIX: &str = "R";

/// The release asset holding the Workshop code is `<prefix><tag><suffix>`: `genjiball-v1.3.3R.txt`,
/// as GenjiBall-CE's release workflow (`.github/workflows/release.yml`) names it.
pub const RELEASE_ASSET_PREFIX: &str = "genjiball-v";
pub const RELEASE_ASSET_SUFFIX: &str = ".txt";

/// How many of the newest releases are searched for a ranked one (GitHub allows at most 100).
pub const RELEASES_SEARCHED: u32 = 100;

/// How long the ranked release found on GitHub is reused before asking GitHub again. Keeps clicks
/// on "Copy ranked code" well inside GitHub's 60 requests an hour without a token.
pub const RELEASE_CACHE_SECS: Tunable = Tunable {
    key: "releaseCacheSecs",
    label: "Release check",
    help: "How long the ranked release found on GitHub is reused before GitHub is asked again. GitHub allows 60 requests an hour.",
    default: 10 * 60,
    min: 60,
    max: 24 * 3600,
};

/// A match's page on the ranked site, under the server URL, with its id (`/match?id=12`). Only
/// `accepted` and `void` matches are public; the site answers any other with "not found".
pub const MATCH_PAGE_PATH: &str = "/match";

/// Every `Tunable`, in the order the window lists them.
pub const TUNABLES: [&Tunable; 7] = [
    &POLL_INTERVAL_SECS,
    &QUIET_SECS,
    &REQUEST_TIMEOUT_SECS,
    &RETRY_FIRST_SECS,
    &RETRY_MAX_SECS,
    &STATUS_REFRESH_SECS,
    &RELEASE_CACHE_SECS,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tunables_are_sane() {
        for (i, t) in TUNABLES.iter().enumerate() {
            assert!(t.min <= t.default && t.default <= t.max, "{}", t.key);
            assert!(t.min > 0, "{}", t.key);
            assert!(
                TUNABLES[..i].iter().all(|other| other.key != t.key),
                "{} twice",
                t.key
            );
        }
        for (i, r) in REGIONS.iter().enumerate() {
            assert!(
                REGIONS[..i].iter().all(|other| other.id != r.id),
                "{}",
                r.id
            );
        }
    }
}
