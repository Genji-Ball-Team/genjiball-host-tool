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
