//! Every tunable and its default. Code reads these, never a literal.
//! The settings screen (#2, #11) lets a host override some of them; these stay the defaults.

/// The ranked server uploads go to. Overridable for local testing or the test server.
pub const DEFAULT_SERVER_URL: &str = "https://genjiball.us";

/// How long a request to the ranked server may take before it counts as unreachable.
pub const REQUEST_TIMEOUT_SECS: u64 = 15;

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
pub const POLL_INTERVAL_SECS: u64 = 5;

/// How long a ranked log must stop growing before it's uploaded without a `MATCH_END`: the host
/// moved to spectator (Overwatch carries on in a new file), closed the lobby or crashed.
pub const QUIET_SECS: u64 = 60;

/// Log files last written longer ago than this are ignored, so a first run doesn't upload a
/// folder full of old matches.
pub const MAX_LOG_AGE_DAYS: u64 = 14;

/// The server refuses bigger uploads (`maxUploadBytes` in genjiball-ranked).
pub const MAX_UPLOAD_BYTES: u64 = 512 * 1024;

/// The wait before retrying a failed upload (server down, offline), doubled after each failure
/// of the same file up to `RETRY_MAX_SECS`. A `Retry-After` from the server wins, however long; on
/// a `429` it holds every upload to that server.
pub const RETRY_FIRST_SECS: u64 = 30;
pub const RETRY_MAX_SECS: u64 = 30 * 60;

/// How many uploads the window lists on a page of the upload history, newest first. Older ones are
/// on the next pages.
pub const UPLOADS_PAGE_SIZE: usize = 8;

/// How often the status of the newest matches (an admin accepting one in review, say) and the
/// host's trust are asked of the server again.
pub const STATUS_REFRESH_SECS: u64 = 120;

/// The server answers at most this many match keys at once (`hostMatchKeysMax` in genjiball-ranked):
/// the newest this many matches get their status refreshed.
pub const MAX_STATUS_KEYS: usize = 50;

/// A match's page on the ranked site, under the server URL, with its id (`/match?id=12`). Only
/// `accepted` and `void` matches are public; the site answers any other with "not found".
pub const MATCH_PAGE_PATH: &str = "/match";
