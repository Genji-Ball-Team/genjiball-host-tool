//! Every tunable and its default. Code reads these, never a literal.
//! The settings screen (#2, #11) will let a host override them; these stay the defaults.

/// The ranked server uploads go to. Overridable for local testing or the test server.
pub const DEFAULT_SERVER_URL: &str = "https://genjiball.us";
