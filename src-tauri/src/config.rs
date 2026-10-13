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

/// The Workshop data centers (the code's `lobby` settings, `Data Center Preference`) a region's
/// ranked and tourney codes can put the lobby on, by the name the code uses (OverPy's
/// `dataCenterPreference` values, in English like the rest of the release's code). The first is the
/// region's default; the host can pick another, or `BEST_AVAILABLE`.
#[derive(Debug, PartialEq, serde::Serialize)]
pub struct RegionDataCenters {
    pub region: &'static str,
    pub names: &'static [&'static str],
}

pub const DATA_CENTERS: [RegionDataCenters; 2] = [
    RegionDataCenters {
        region: "eu",
        names: &[
            "Netherlands",
            "Germany",
            "Germany 2",
            "France",
            "Ireland",
            "Finland 2",
        ],
    },
    RegionDataCenters {
        region: "na",
        names: &[
            "USA - Central",
            "USA - East",
            "USA - East 2",
            "USA - West",
            "USA - West 2",
            "USA - Northwest",
            "USA - Southwest",
        ],
    },
];

/// The data center that leaves the server to the game (the best ping for the host): the code
/// gets no `Data Center Preference` line.
pub const BEST_AVAILABLE: &str = "Best Available";

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

/// Host AFK (`afk.rs`): whether it's on, and the rounds that weren't rated for the host, per match.
/// In the app's config folder, so they last across restarts.
pub const AFK_FILE: &str = "afk.json";

/// How long the AFK rounds of a match are kept after the last one, for its uploads. A match is
/// usually
/// uploaded minutes after it ends; this leaves room for a host who stays offline for weeks.
pub const AFK_KEEP_SECS: u64 = 30 * 24 * 3600;

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

/// How often the window's match view reads the log again, while the window is open and the view is
/// shown (#16): the live match follows the game this closely. Only a log that grew is sent again.
pub const MATCH_VIEW_POLL_SECS: Tunable = Tunable {
    key: "matchViewPollSecs",
    label: "Match view refresh",
    help: "How often the match view reads the log again while this window shows it.",
    default: 2,
    min: 1,
    max: 60,
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
/// of the ranked code, unless the host pinned one (`releaseTag` in the settings, which must end in
/// this too).
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

/// How many of the region's best players the ranked code tags with their place and rating, and
/// the game lists on the right (its "top 10 list": GenjiBall-CE `docs/rank-tags.md` allows 60).
pub const TOP_TAGGED: usize = 10;

/// A match's page on the ranked site, under the server URL, with its id (`/match?id=12`). Only
/// `accepted` and `void` matches are public; the site answers any other with "not found".
pub const MATCH_PAGE_PATH: &str = "/match";

/// Live lobby (#6): whether the tool lists the host's lobby on the site while a ranked match is
/// being played, for a host who hasn't switched it (`Settings::live_lobby`).
pub const LIVE_LOBBY_ON_BY_DEFAULT: bool = true;

/// How often a listed lobby is refreshed until the server says (`heartbeatSeconds` in its answer,
/// which wins: genjiball-ranked `lobbyHeartbeatSeconds`).
pub const LOBBY_HEARTBEAT_SECS: u64 = 60;

/// The server refuses a heartbeat this soon after the last heartbeat or close
/// (genjiball-ranked `lobbyHeartbeatMinSeconds`). Also the shortest heartbeat interval the tool
/// takes from an answer, and the wait after a request that failed.
pub const LOBBY_HEARTBEAT_MIN_SECS: u64 = 30;

/// How long a lobby stays listed without a heartbeat until the server says (`ttlSeconds` in its
/// answer: genjiball-ranked `lobbyTtlSeconds`). Past it, the tool knows it's off the list.
pub const LOBBY_TTL_SECS: u64 = 180;

/// The longest lobby name the server takes, in characters (genjiball-ranked `lobbyNameMaxLength`).
pub const LOBBY_NAME_MAX_CHARS: usize = 64;

/// The most players a heartbeat may say (genjiball-ranked `lobbyPlayersMax`).
pub const LOBBY_PLAYERS_MAX: u32 = 12;

/// How long quitting the tool waits for the server to take a listed lobby off the list. Past it
/// the tool quits anyway, and the lobby drops off after `LOBBY_TTL_SECS`.
pub const LOBBY_CLOSE_ON_QUIT_SECS: u64 = 3;

/// How often the tourney lobbies the host is assigned to (`GET /api/host/tourneys`) are asked of
/// the server while nothing is due (#8). The tool also asks when a code window opens, shortly
/// before a start, after a tourney match is uploaded and on "Check again". The server answers
/// host routes uncached, so each ask is a database read: keep this long.
pub const TOURNEY_POLL_SECS: Tunable = Tunable {
    key: "tourneyPollSecs",
    label: "Tourney check",
    help: "How often the server is asked about the tourneys you host. It's also asked when a code becomes available, before a start and after a tourney match is uploaded.",
    default: 5 * 60,
    min: 60,
    max: 3600,
};

/// How long before a tourney starts the host gets a notification that it's about to.
pub const TOURNEY_START_NOTICE_SECS: u64 = 15 * 60;

/// An upload with a tourney match whose lobby the tool doesn't know yet asks the server for the
/// host's lobbies first, to upload it as the lobby's region; at most this often per server, so
/// copies of the same match don't each ask.
pub const TOURNEY_LOOKUP_MIN_SECS: u64 = 30;

/// The longest tourney name and lobby label the game shows on its HUD line, in characters
/// (GenjiBall-CE `docs/tourney-rule.md`: "under 40 characters each"). Longer ones are cut.
pub const TOURNEY_TEXT_MAX_CHARS: usize = 39;

/// Where Overwatch saves screenshots, under the user's Documents folder: the default screenshots
/// folder the verify screenshot is offered from (#10).
pub const SCREENSHOT_SUBFOLDER: [&str; 3] = ["Overwatch", "ScreenShots", "Overwatch"];

/// The image files offered from the screenshots folder, by extension (lower case). The server
/// takes PNG, JPEG and WebP, by their content.
pub const SCREENSHOT_EXTENSIONS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

/// The biggest verify screenshot the server takes (genjiball-ranked `screenshotMaxBytes`).
pub const SCREENSHOT_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// How often the window looks for a new screenshot in the screenshots folder while it asks for a
/// lobby's verify screenshot. Only reads the folder's file list.
pub const SCREENSHOT_POLL_SECS: u64 = 3;

/// How often the tool looks for a new version of itself, after the check at startup. The endpoint
/// and the signing key are in `tauri.conf.json`, under `plugins.updater`.
pub const UPDATE_CHECK_SECS: Tunable = Tunable {
    key: "updateCheckSecs",
    label: "Update check",
    help: "How often the tool looks for a new version of itself. It also looks when it starts.",
    default: 6 * 3600,
    min: 3600,
    max: 7 * 24 * 3600,
};

/// Whether the tool looks for updates by itself (at startup and every `UPDATE_CHECK_SECS`), for a
/// host who hasn't switched it (`Settings::auto_update_check`). "Check for updates" works either way.
pub const AUTO_UPDATE_CHECK_BY_DEFAULT: bool = true;

/// The update channels the host picks from under Advanced → Updates. `stable` reads the updater
/// endpoint in `tauri.conf.json` (`latest.json` on the latest release, which is never a
/// pre-release); `prerelease` the `UPDATE_MANIFEST` of the newest release, pre-releases too.
pub const UPDATE_CHANNELS: [&str; 2] = ["stable", "prerelease"];
pub const DEFAULT_UPDATE_CHANNEL: &str = "stable";

/// The repo the tool's own releases are published in, by the release workflow.
pub const APP_REPO: &str = "Genji-Ball-Team/genjiball-host-tool";

/// The release asset the updater reads: the version, its installer and the installer's signature.
pub const UPDATE_MANIFEST: &str = "latest.json";

/// How many of the newest releases the `prerelease` channel searches for one with an
/// `UPDATE_MANIFEST` (a release still being built has none yet).
pub const UPDATE_RELEASES_SEARCHED: u32 = 10;

/// How often the update loop looks at the clock to see whether a check is due, so a changed
/// `UPDATE_CHECK_SECS` counts within this long.
pub const UPDATE_TICK_SECS: u64 = 60;

/// The tool's own log (`logging.rs`), `<name>.log` in the app's log folder
/// (`%LOCALAPPDATA%\us.genjiball.hosttool\logs`). Older ones are `<name>_<date>.log` next to it.
pub const LOG_FILE_NAME: &str = "host-tool";

/// The tool's log starts a new file once it's this big.
pub const LOG_MAX_BYTES: u64 = 1024 * 1024;

/// How many older log files are kept besides the current one. With `LOG_MAX_BYTES`, the log folder
/// never holds much more than 5 MB.
pub const LOG_FILES_KEPT: usize = 4;
// The log plugin keeps `LOG_FILES_KEPT - 1` when it starts a new file.
const _: () = assert!(LOG_FILES_KEPT > 0 && LOG_MAX_BYTES > 0);

/// The log levels the host picks from under Advanced → Debug, least to most detailed. Each logs
/// what the ones before it do, and more.
pub const LOG_LEVELS: [&str; 4] = ["error", "warn", "info", "debug"];

/// The log level when the host hasn't picked one: uploads, checks and errors, not every poll.
pub const DEFAULT_LOG_LEVEL: &str = "info";

/// A diagnostics export (`diagnostics.rs`) holds at most this much of the tool's own log, newest
/// first: the current file, then older ones while they fit, the last cut to its end.
pub const DIAGNOSTICS_LOG_BYTES: u64 = 2 * 1024 * 1024;

/// The file name a diagnostics export is offered under, before the date: `<prefix>-<date>.json`.
pub const DIAGNOSTICS_FILE_PREFIX: &str = "genjiball-host-tool-diagnostics";

/// Dry run (Advanced → Debug): the uploader picks the logs to upload as usual but sends nothing,
/// and the debug panel shows what it would have sent. For a host who hasn't switched it
/// (`Settings::dry_run`).
pub const DRY_RUN_BY_DEFAULT: bool = false;

/// How many of the latest uploads and dry runs the debug panel lists, newest first. Kept in memory
/// only: a restart starts the list afresh.
pub const DEBUG_UPLOADS_KEPT: usize = 20;

/// The debug panel shows at most this much of a server's answer to an upload.
pub const DEBUG_ANSWER_BYTES: usize = 16 * 1024;

/// How many of the live log's last lines (its newest events) the debug panel shows.
pub const DEBUG_EVENTS_SHOWN: usize = 40;

/// The overlay (#49): an experimental window over Overwatch, for a host who hasn't switched it.
/// Off until the host turns it on under Settings → Overlay.
pub const OVERLAY_ON_BY_DEFAULT: bool = false;

/// A widget the overlay and the stream page (#55) can show. `key` is its name in `settings.json`
/// and between Rust and the windows; `group` the heading it's listed under in Settings.
#[derive(Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayWidget {
    pub key: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    /// What it shows, for Settings.
    pub help: &'static str,
    /// On in the overlay, and on the stream page, for a host who hasn't switched it.
    pub overlay: bool,
    pub stream: bool,
}

/// Every widget, in the order Settings lists them. Host status (#50), lobby (#51), match (#52),
/// after the round (#53) and tourney (#54).
pub const OVERLAY_WIDGETS: [OverlayWidget; 13] = [
    OverlayWidget {
        key: "logging",
        label: "Logging",
        group: "Host status",
        help: "Whether a ranked match is being recorded, and why it won't count.",
        overlay: true,
        stream: false,
    },
    OverlayWidget {
        key: "uploads",
        label: "Uploads",
        group: "Host status",
        help: "The last upload's result, and anything holding uploads up.",
        overlay: true,
        stream: false,
    },
    OverlayWidget {
        key: "afk",
        label: "AFK",
        group: "Host status",
        help: "A badge while AFK is on, so you never forget it.",
        overlay: true,
        stream: false,
    },
    OverlayWidget {
        key: "logCopies",
        label: "Log copies",
        group: "Host status",
        help: "How many files the match is split over: each move to or from spectator starts one.",
        overlay: false,
        stream: false,
    },
    OverlayWidget {
        key: "rankedCode",
        label: "Ranked code",
        group: "Host status",
        help: "How long ago you copied the ranked code, and when its rank tags may be stale.",
        overlay: false,
        stream: false,
    },
    OverlayWidget {
        key: "roster",
        label: "Lobby roster",
        group: "Lobby",
        help: "The players in the lobby with their tier, rating and place, new players and doubled names.",
        overlay: true,
        stream: true,
    },
    OverlayWidget {
        key: "eliminations",
        label: "Eliminations",
        group: "Match",
        help: "Who went out this round, in order, and who sent the ball.",
        overlay: false,
        stream: true,
    },
    OverlayWidget {
        key: "standings",
        label: "Standings",
        group: "Match",
        help: "Round wins and kills in this match, ranked as the site ranks them.",
        overlay: true,
        stream: true,
    },
    OverlayWidget {
        key: "killFeed",
        label: "Kill feed",
        group: "Match",
        help: "The latest kills, with the deflects that led to them.",
        overlay: false,
        stream: true,
    },
    OverlayWidget {
        key: "roundResult",
        label: "Round result",
        group: "After the round",
        help: "The finishing order of the round that just ended, until the next one starts.",
        overlay: true,
        stream: true,
    },
    OverlayWidget {
        key: "matchSummary",
        label: "Match summary",
        group: "After the round",
        help: "Places, wins and kills when the match ends, and rating changes once the server has rated it.",
        overlay: true,
        stream: true,
    },
    OverlayWidget {
        key: "session",
        label: "Session",
        group: "After the round",
        help: "Matches, rounds and players since the tool started, and your own rating change.",
        overlay: false,
        stream: false,
    },
    OverlayWidget {
        key: "tourney",
        label: "Tourney",
        group: "Tourney",
        help: "In a tourney match: the round of the limit, live standings and the verify screenshot reminder.",
        overlay: true,
        stream: true,
    },
];

/// A setting of the overlay the host picks a whole number for: its default and range.
#[derive(Debug, PartialEq, serde::Serialize)]
pub struct OverlayRange {
    pub default: u16,
    pub min: u16,
    pub max: u16,
}

/// How opaque the widgets are, in percent.
pub const OVERLAY_OPACITY: OverlayRange = OverlayRange {
    default: 92,
    min: 30,
    max: 100,
};

/// How big the widgets are, in percent of their normal size.
pub const OVERLAY_SCALE: OverlayRange = OverlayRange {
    default: 100,
    min: 70,
    max: 160,
};

/// How big one widget can be made in edit mode, as a share of its normal size (times the
/// overlay's size), and the step a right-click "Bigger" or a Ctrl+wheel notch takes.
pub const OVERLAY_WIDGET_SIZE_MIN: f64 = 0.5;
pub const OVERLAY_WIDGET_SIZE_MAX: f64 = 2.5;
pub const OVERLAY_WIDGET_SIZE_STEP: f64 = 0.1;

/// Whether the overlay shows only while Overwatch is the window in front, for a host who hasn't
/// switched it. Off, it also shows over the desktop and other windows.
pub const OVERLAY_ONLY_WITH_GAME_BY_DEFAULT: bool = true;

/// What the overlay's global hotkeys do, and their default keys (Tauri accelerator syntax). A
/// hotkey is taken by Windows before the game sees it, so the defaults are combinations Overwatch
/// doesn't use. The host can change each, or clear it.
#[derive(Debug, PartialEq, serde::Serialize)]
pub struct OverlayHotkey {
    pub action: &'static str,
    pub label: &'static str,
    pub default: &'static str,
}

pub const OVERLAY_HOTKEYS: [OverlayHotkey; 3] = [
    OverlayHotkey {
        action: "toggle",
        label: "Show or hide the overlay",
        default: "Ctrl+Alt+O",
    },
    OverlayHotkey {
        action: "edit",
        label: "Edit the layout",
        default: "Ctrl+Alt+L",
    },
    OverlayHotkey {
        action: "afk",
        label: "AFK on or off",
        default: "Ctrl+Alt+A",
    },
];

/// How often the overlay looks for the Overwatch window, in milliseconds: whether it's in front
/// and where it is. Only asks Windows about the window in front (`overlay_window.rs`).
pub const OVERLAY_TRACK_MS: u64 = 400;

/// How many of those checks pass between placing the overlay again while it shows, even if
/// nothing moved, so it stays over windows that went on top since.
pub const OVERLAY_REPLACE_TRACKS: u32 = 8;

/// How long after the last Ctrl+wheel notch a widget's new size is saved, in milliseconds: the
/// size shows at once, and one save follows a turn of the wheel.
pub const OVERLAY_SIZE_SAVE_MS: u64 = 400;

/// How the overlay knows the Overwatch window: its window class, else its title. Read from the
/// window only, never from the game's process.
pub const GAME_WINDOW_CLASS: &str = "TankWindowClass";
pub const GAME_WINDOW_TITLE: &str = "Overwatch";

/// How often the overlay and the stream page read the live log and the tool's state again, in
/// milliseconds. The log is only sent again when it grew.
pub const OVERLAY_POLL_MS: u64 = 1000;

/// How many of the newest kills the kill feed shows.
pub const KILL_FEED_SHOWN: usize = 5;

/// How long the ranked code may have been copied before the overlay says its rank tags may be
/// stale: ratings move with every match.
pub const RANKED_CODE_STALE_SECS: u64 = 6 * 3600;

/// How long the roster keeps a player's rating before asking the server again.
pub const RANKS_CACHE_SECS: u64 = 10 * 60;

/// How many pages of the region's leaderboard (`leaderboardPageSize`, 50 a page) the roster reads
/// for ratings, at most. A name not on them is searched (`/api/players?search=`) on its own.
pub const RANKS_LEADERBOARD_PAGES: u32 = 4;

/// How long a name the server doesn't know is kept as unknown before it's searched again. Long:
/// each search reads every alias (genjiball-ranked `docs/database.md`, "Free tier").
pub const RANKS_UNKNOWN_SECS: u64 = 30 * 60;

/// The fewest characters the server searches a name for (genjiball-ranked
/// `playerSearchMinLength`). Shorter names are only found on the leaderboard.
pub const PLAYER_SEARCH_MIN_CHARS: usize = 2;

/// How often a finished match's ratings are asked of the server again until it has rated them
/// (`/api/matches/:id`). A complete match is rated at upload, so it's usually the first answer.
pub const MATCH_RATINGS_RETRY_SECS: u64 = 30;

/// How many of a session's matches the overlay asks the server about, newest first.
pub const SESSION_MATCHES_MAX: usize = 30;

/// How many of the newest logs are read for copies of the live match (the same `matchKey`). Each
/// is read again only once it grew.
pub const LOG_COPIES_SCANNED: usize = 30;

/// The stream page (#55): the overlay's widgets as a page on this PC, for an OBS browser source.
/// Off until the host turns it on.
pub const STREAM_ON_BY_DEFAULT: bool = false;

/// The port the stream page is served on, on `127.0.0.1` only, unless the host picks another.
pub const STREAM_PORT: OverlayRange = OverlayRange {
    default: 47_623,
    min: 1024,
    max: 65_535,
};

/// How often the stream page's server looks for a new connection, and whether it was turned
/// off, in milliseconds.
pub const STREAM_ACCEPT_POLL_MS: u64 = 100;

/// The biggest request head the stream page reads. It only answers `GET`s without a body.
pub const STREAM_REQUEST_MAX_BYTES: usize = 8 * 1024;

/// How long the stream page waits for a request before it hangs up.
pub const STREAM_READ_TIMEOUT_SECS: u64 = 5;

/// Every `Tunable`, in the order the window lists them.
pub const TUNABLES: [&Tunable; 10] = [
    &POLL_INTERVAL_SECS,
    &QUIET_SECS,
    &MATCH_VIEW_POLL_SECS,
    &REQUEST_TIMEOUT_SECS,
    &RETRY_FIRST_SECS,
    &RETRY_MAX_SECS,
    &STATUS_REFRESH_SECS,
    &TOURNEY_POLL_SECS,
    &RELEASE_CACHE_SECS,
    &UPDATE_CHECK_SECS,
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
        assert!(LOG_LEVELS.contains(&DEFAULT_LOG_LEVEL));
        assert!(UPDATE_CHANNELS.contains(&DEFAULT_UPDATE_CHANNEL));
        for (i, r) in REGIONS.iter().enumerate() {
            assert!(
                REGIONS[..i].iter().all(|other| other.id != r.id),
                "{}",
                r.id
            );
            let centers = DATA_CENTERS.iter().find(|d| d.region == r.id);
            assert!(centers.is_some_and(|d| !d.names.is_empty()), "{}", r.id);
        }
        assert_eq!(DATA_CENTERS.len(), REGIONS.len());
    }

    #[test]
    fn overlay_settings_are_sane() {
        for (i, w) in OVERLAY_WIDGETS.iter().enumerate() {
            assert!(
                OVERLAY_WIDGETS[..i].iter().all(|o| o.key != w.key),
                "{} twice",
                w.key
            );
        }
        for (i, h) in OVERLAY_HOTKEYS.iter().enumerate() {
            assert!(OVERLAY_HOTKEYS[..i].iter().all(|o| o.action != h.action));
            assert!(OVERLAY_HOTKEYS[..i].iter().all(|o| o.default != h.default));
        }
        for r in [&OVERLAY_OPACITY, &OVERLAY_SCALE, &STREAM_PORT] {
            assert!(r.min <= r.default && r.default <= r.max);
        }
    }
}
