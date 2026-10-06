//! Tourneys (#8, #9, #10), the pure parts: the tourney code's `TOURNEY - generated` rule
//! (GenjiBall-CE `docs/tourney-rule.md`), when to tell the host about a lobby, which region a log
//! with a tourney match is uploaded as, and the verify screenshot's checks. The loop that asks the
//! server is `tourneys.rs`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;

use crate::config;
use crate::log_scan::Scan;
use crate::ranked_code::{custom_string, replace_rule, RuleError};
use crate::server::{TourneyCodeValues, TourneyLobby};

/// The rule's first line. It's in a release with tourneys exactly once.
pub const RULE_START: &str = r#"rule ("TOURNEY - generated") {"#;

/// The index of `rankedState` the rule sets (`RankedField.TOURNEY` in GenjiBall-CE): fixed.
const RANKED_STATE_INDEX: u32 = 16;

/// The values as they went into the rule: what the window shows the host it built.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Written {
    pub lobby_key: String,
    pub round_limit: u32,
    pub name: String,
    pub label: String,
}

/// A name or label as the game can show it: no `{` or `}` (the Workshop reads them as
/// placeholders), no line breaks, at most `config::TOURNEY_TEXT_MAX_CHARS` characters.
pub fn hud_text(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, '{' | '}'))
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .chars()
        .take(config::TOURNEY_TEXT_MAX_CHARS)
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// The rule's action for `values`: `Set Global Variable At Index(rankedState, 16, Array(True,
/// lobbyKey, roundLimit, name, label));`, and the values as written. An error for a lobby key that
/// isn't digits (the game logs it as is, and the server matches it as text).
pub fn action(values: &TourneyCodeValues) -> Result<(String, Written), String> {
    let key = &values.lobby_key;
    if key.is_empty() || !key.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!(
            "The server's lobby key ({key}) isn't digits: the host tool can't put it in the code"
        ));
    }
    let written = Written {
        lobby_key: key.clone(),
        round_limit: values.round_limit,
        name: hud_text(&values.name),
        label: hud_text(&values.label),
    };
    let action = format!(
        "Set Global Variable At Index(rankedState, {RANKED_STATE_INDEX}, Array(True, {}, {}, {}, {}));",
        custom_string(&written.lobby_key),
        written.round_limit,
        custom_string(&written.name),
        custom_string(&written.label),
    );
    Ok((action, written))
}

/// `code` (a ranked code: its `RANKS - generated` rule may be filled already) with its
/// `TOURNEY - generated` rule turned on with `values`. Everything else stays as it was.
pub fn fill(code: &str, values: &TourneyCodeValues) -> Result<(String, Written), String> {
    let (action, written) = action(values)?;
    let filled = replace_rule(code, RULE_START, &action).map_err(|e| match e {
        RuleError::Missing => {
            "The TOURNEY - generated rule marker isn't in the base release, so it can't make a tourney code. Use a Genji Ball release with tourneys (ask an admin which; Advanced → Ranked code release)"
        }
        RuleError::Twice => {
            "The base release has the TOURNEY - generated rule twice, so the host tool can't tell which to fill"
        }
        RuleError::Unended => "The TOURNEY - generated rule in the base release never ends",
    })?;
    Ok((filled, written))
}

/// An ISO 8601 time from the server.
pub fn parse_time(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Whether the lobby is over for its host, apart from its screenshot: its match is linked, or its
/// tourney is done or cancelled. Its code window is closed for good then.
pub fn is_done(lobby: &TourneyLobby) -> bool {
    lobby.match_id.is_some() || matches!(lobby.tourney.status.as_str(), "done" | "cancelled")
}

/// Whether the host still owes the lobby its verify screenshot: its match is uploaded (linked on
/// the server, or `uploaded`: sent by this tool with its `MATCH_END ROUNDS`), and there's no
/// screenshot (or it expired) and no admin verified one.
pub fn needs_screenshot(lobby: &TourneyLobby, uploaded: bool) -> bool {
    (lobby.match_id.is_some() || uploaded)
        && lobby.screenshot.is_none()
        && !lobby.verified
        && lobby.tourney.status != "cancelled"
}

/// What the host is told about, once per lobby.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Notice {
    /// The server gives the lobby's code values: "Copy tourney code" works.
    CodeOpen,
    /// The tourney starts within `config::TOURNEY_START_NOTICE_SECS`.
    StartsSoon,
    /// Its match is uploaded, and the screenshot isn't.
    Screenshot,
}

/// What's due about `lobby` at `now` (`uploaded` as for `needs_screenshot`). `start_notice`: how
/// long before the start `StartsSoon` is.
pub fn notices(
    lobby: &TourneyLobby,
    uploaded: bool,
    now: DateTime<Utc>,
    start_notice: Duration,
) -> Vec<Notice> {
    let mut due = Vec::new();
    if lobby.code.is_some() && !is_done(lobby) {
        due.push(Notice::CodeOpen);
    }
    if let Some(starts) = parse_time(&lobby.tourney.starts_at) {
        let soon = starts - chrono::Duration::from_std(start_notice).unwrap_or_default();
        if !is_done(lobby) && soon <= now && now < starts {
            due.push(Notice::StartsSoon);
        }
    }
    if needs_screenshot(lobby, uploaded) {
        due.push(Notice::Screenshot);
    }
    due
}

/// The notification's title and text.
pub fn notice_text(notice: Notice, lobby: &TourneyLobby) -> (String, String) {
    let lobby_name = format!("{}, {}", lobby.tourney.name, lobby.label);
    match notice {
        Notice::CodeOpen => (
            "Tourney code available".into(),
            format!("{lobby_name}: \"Copy tourney code\" in the host tool builds it now."),
        ),
        Notice::StartsSoon => (
            "Tourney starting soon".into(),
            format!("{lobby_name} starts soon. Import the tourney code and open the lobby."),
        ),
        Notice::Screenshot => (
            "Upload the verify screenshot".into(),
            format!(
                "{lobby_name}: the match is uploaded. Upload your screenshot of the final standings in the host tool."
            ),
        ),
    }
}

/// The next moment something about `lobbies` is due after `now`: a code window opening, or the
/// start notice. The tourney loop asks the server again then.
pub fn next_wake(
    lobbies: &[TourneyLobby],
    now: DateTime<Utc>,
    start_notice: Duration,
) -> Option<DateTime<Utc>> {
    let notice = chrono::Duration::from_std(start_notice).unwrap_or_default();
    lobbies
        .iter()
        .filter(|l| !is_done(l))
        .flat_map(|l| {
            let opens = l
                .code_from
                .as_deref()
                .and_then(parse_time)
                .filter(|_| l.code.is_none());
            let soon = parse_time(&l.tourney.starts_at).map(|s| s - notice);
            [opens, soon]
        })
        .flatten()
        .filter(|at| *at > now)
        .min()
}

/// The region a log is uploaded as. A file whose every match is a tourney match, all in lobbies
/// of one region the tool knows (`lobby_region`, by lobby key), goes as that region: the server
/// links a tourney match only when it's uploaded as its tourney's region. Anything else goes as
/// `tool_region`, the region the host picked (else their home region).
pub fn upload_region(
    scan: &Scan,
    lobby_region: impl Fn(&str) -> Option<String>,
    tool_region: Option<&str>,
) -> Option<String> {
    let keys = scan.lobby_keys();
    if keys.is_empty() || !scan.only_tourneys() {
        return tool_region.map(str::to_string);
    }
    let regions: Option<Vec<String>> = keys.iter().map(|k| lobby_region(k)).collect();
    match regions {
        Some(regions) if regions.windows(2).all(|w| w[0] == w[1]) => regions.into_iter().next(),
        _ => tool_region.map(str::to_string),
    }
}

/// The MIME type of a PNG, JPEG or WebP image, from its first bytes: what the server takes.
pub fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']) {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// The MIME type of a verify screenshot the server will take, or why it won't.
pub fn check_screenshot(bytes: &[u8]) -> Result<&'static str, String> {
    if bytes.is_empty() {
        return Err("The image is empty".into());
    }
    if bytes.len() as u64 > config::SCREENSHOT_MAX_BYTES {
        return Err(too_big());
    }
    image_type(bytes).ok_or_else(|| "That isn't a PNG, JPEG or WebP image".into())
}

fn too_big() -> String {
    format!(
        "The image is over the server's {} MB limit",
        config::SCREENSHOT_MAX_BYTES / (1024 * 1024)
    )
}

/// An image file in the screenshots folder.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotFile {
    pub path: PathBuf,
    pub name: String,
    /// When it was last written (taken), RFC 3339 in UTC.
    pub taken_at: String,
    pub bytes: u64,
}

/// Whether a file name is one of the image types offered (`config::SCREENSHOT_EXTENSIONS`).
pub fn is_image_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| {
            config::SCREENSHOT_EXTENSIONS
                .iter()
                .any(|x| e.eq_ignore_ascii_case(x))
        })
}

/// The newest image in `folder`, by when it was written. Only reads the folder's file list.
pub fn newest_screenshot(folder: &Path) -> io::Result<Option<ScreenshotFile>> {
    let mut newest: Option<(SystemTime, ScreenshotFile)> = None;
    for entry in fs::read_dir(folder)? {
        let Ok(entry) = entry else { continue };
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if !is_image_name(&name) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if newest.as_ref().is_none_or(|(at, _)| modified > *at) {
            let file = ScreenshotFile {
                path: entry.path(),
                name,
                taken_at: DateTime::<Utc>::from(modified)
                    .to_rfc3339_opts(SecondsFormat::Secs, true),
                bytes: meta.len(),
            };
            newest = Some((modified, file));
        }
    }
    Ok(newest.map(|(_, file)| file))
}

/// An image file the host picked, dropped or was offered, read for a preview and its upload:
/// its bytes and MIME type, or why the server wouldn't take it. Its size is checked before it's
/// read.
pub fn read_screenshot(path: &Path) -> Result<(Vec<u8>, &'static str), String> {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into(),
    );
    let meta = fs::metadata(path).map_err(|e| format!("Couldn't read {name}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{name} isn't a file"));
    }
    if meta.len() > config::SCREENSHOT_MAX_BYTES {
        return Err(too_big());
    }
    let bytes = fs::read(path).map_err(|e| format!("Couldn't read {name}: {e}"))?;
    let kind = check_screenshot(&bytes)?;
    Ok((bytes, kind))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_scan;
    use crate::ranked_code;
    use crate::server::LobbyTourney;

    /// A ranked code's two generated rules and a neighbour, from GenjiBall-CE
    /// `workshop/genjiball.txt` on `ranked/tourney`.
    const CODE: &str = include_str!("../tests/fixtures/tourney-rule-example.txt");

    fn values() -> TourneyCodeValues {
        TourneyCodeValues {
            lobby_key: "073518264903".into(),
            round_limit: 30,
            name: "October Cup".into(),
            label: "Lobby 1/2".into(),
        }
    }

    #[test]
    fn writes_the_rule_in_the_spec_shape() {
        let (action, written) = action(&values()).unwrap();
        // The example in `docs/tourney-rule.md`.
        assert_eq!(
            action,
            r#"Set Global Variable At Index(rankedState, 16, Array(True, Custom String("073518264903"), 30, Custom String("October Cup"), Custom String("Lobby 1/2")));"#
        );
        assert_eq!(written.lobby_key, "073518264903");
        assert_eq!(written.name, "October Cup");
    }

    #[test]
    fn replaces_only_the_tourney_rule() {
        let (filled, _) = fill(CODE, &values()).unwrap();
        let off = r#"Set Global Variable At Index(rankedState, 16, Array(False, Custom String(""), 30, Custom String(""), Custom String("")));"#;
        let (on, _) = action(&values()).unwrap();
        assert_eq!(filled, CODE.replace(off, &on));
        assert_eq!(filled.matches(RULE_START).count(), 1);
        // The rule after it, which reads the values, is untouched.
        assert!(filled.contains("rule (\"Tourney - settings\") {\n    event {"));
        // And CRLF stays CRLF.
        let crlf = CODE.replace('\n', "\r\n");
        let (filled, _) = fill(&crlf, &values()).unwrap();
        assert_eq!(filled, crlf.replace(off, &on));
    }

    #[test]
    fn a_tourney_code_has_both_rules_filled() {
        let tags = ranked_code::RankTags {
            header: "Top 0".into(),
            tiers: vec![],
        };
        let ranked = ranked_code::fill(CODE, &tags).unwrap().code;
        let (code, _) = fill(&ranked, &values()).unwrap();
        assert!(code.contains(r#"Set Global Variable(rankTags, Array(Custom String("Top 0")));"#));
        assert!(code.contains("Array(True, Custom String(\"073518264903\")"));
    }

    #[test]
    fn stops_without_the_rule_marker() {
        let old = CODE.replace(RULE_START, r#"rule ("Something else") {"#);
        let error = fill(&old, &values()).unwrap_err();
        assert!(
            error.contains("rule marker isn't in the base release"),
            "{error}"
        );
        // Only a whole line counts.
        let indented = CODE.replace(RULE_START, &format!("  {RULE_START}"));
        assert!(fill(&indented, &values()).is_err());
        let twice = format!("{CODE}\n{CODE}");
        assert!(fill(&twice, &values()).unwrap_err().contains("twice"));
        let start = CODE.find(RULE_START).unwrap();
        let cut = &CODE[..start + RULE_START.len() + 30];
        assert!(fill(cut, &values()).unwrap_err().contains("never ends"));
    }

    #[test]
    fn escapes_and_cleans_the_names() {
        let tricky = TourneyCodeValues {
            name: r#"The "Big" \ Cup {0}"#.into(),
            label: "Lobby\n1".into(),
            ..values()
        };
        let (action, written) = action(&tricky).unwrap();
        assert!(
            action.contains(r#"Custom String("The \"Big\" \\ Cup 0")"#),
            "{action}"
        );
        assert!(action.contains(r#"Custom String("Lobby 1")"#), "{action}");
        assert_eq!(written.name, r#"The "Big" \ Cup 0"#);
        // Cut to what the HUD line holds.
        let long = TourneyCodeValues {
            name: "x".repeat(60),
            ..values()
        };
        assert_eq!(
            action_written(&long).name.chars().count(),
            config::TOURNEY_TEXT_MAX_CHARS
        );
    }

    fn action_written(values: &TourneyCodeValues) -> Written {
        action(values).unwrap().1
    }

    #[test]
    fn refuses_a_lobby_key_that_isnt_digits() {
        for key in ["", "12a4", "1 2", "\"); Abort"] {
            let bad = TourneyCodeValues {
                lobby_key: key.into(),
                ..values()
            };
            assert!(action(&bad).is_err(), "{key}");
        }
    }

    fn at(text: &str) -> DateTime<Utc> {
        parse_time(text).unwrap()
    }

    fn lobby() -> TourneyLobby {
        TourneyLobby {
            id: 7,
            label: "Lobby 1/2".into(),
            region: "eu".into(),
            round_limit: 30,
            tourney: LobbyTourney {
                id: 3,
                name: "October Cup".into(),
                region: "eu".into(),
                starts_at: "2026-10-10T17:00:00Z".into(),
                status: "scheduled".into(),
            },
            match_id: None,
            screenshot: None,
            screenshot_expired: false,
            verified: false,
            code_from: Some("2026-10-10T16:00:00Z".into()),
            code: None,
        }
    }

    const NOTICE: Duration = Duration::from_secs(config::TOURNEY_START_NOTICE_SECS);

    #[test]
    fn tells_the_host_when_the_code_opens_and_before_the_start() {
        let early = lobby();
        assert!(notices(&early, false, at("2026-10-10T15:00:00Z"), NOTICE).is_empty());
        let open = TourneyLobby {
            code: Some(values()),
            ..lobby()
        };
        assert_eq!(
            notices(&open, false, at("2026-10-10T16:01:00Z"), NOTICE),
            [Notice::CodeOpen]
        );
        assert_eq!(
            notices(&open, false, at("2026-10-10T16:50:00Z"), NOTICE),
            [Notice::CodeOpen, Notice::StartsSoon]
        );
        // Started: no "starts soon" any more.
        assert_eq!(
            notices(&open, false, at("2026-10-10T17:05:00Z"), NOTICE),
            [Notice::CodeOpen]
        );
        // Cancelled: nothing.
        let mut cancelled = open.clone();
        cancelled.tourney.status = "cancelled".into();
        assert!(notices(&cancelled, true, at("2026-10-10T16:50:00Z"), NOTICE).is_empty());
    }

    #[test]
    fn asks_for_the_screenshot_once_the_match_is_uploaded() {
        let now = at("2026-10-10T18:00:00Z");
        let mut done = TourneyLobby {
            match_id: Some(812),
            ..lobby()
        };
        assert!(needs_screenshot(&done, false));
        assert_eq!(notices(&done, false, now, NOTICE), [Notice::Screenshot]);
        // Uploaded by this tool, before the server linked it (or while it's in review).
        assert!(needs_screenshot(&lobby(), true));
        assert!(!needs_screenshot(&lobby(), false));
        done.screenshot = Some("/api/screenshots/a".into());
        assert!(!needs_screenshot(&done, true));
        // Expired before an admin verified it: needed again.
        done.screenshot = None;
        done.screenshot_expired = true;
        assert!(needs_screenshot(&done, false));
        done.verified = true;
        assert!(!needs_screenshot(&done, true));
    }

    #[test]
    fn wakes_when_a_code_opens_or_a_start_nears() {
        let lobbies = [lobby()];
        assert_eq!(
            next_wake(&lobbies, at("2026-10-10T15:00:00Z"), NOTICE),
            Some(at("2026-10-10T16:00:00Z"))
        );
        assert_eq!(
            next_wake(&lobbies, at("2026-10-10T16:00:00Z"), NOTICE),
            Some(at("2026-10-10T16:45:00Z"))
        );
        assert_eq!(
            next_wake(&lobbies, at("2026-10-10T16:45:00Z"), NOTICE),
            None
        );
        // A code already given needs no wake for its window; a done lobby none at all.
        let open = TourneyLobby {
            code: Some(values()),
            ..lobby()
        };
        assert_eq!(
            next_wake(&[open], at("2026-10-10T15:00:00Z"), NOTICE),
            Some(at("2026-10-10T16:45:00Z"))
        );
        let done = TourneyLobby {
            match_id: Some(1),
            ..lobby()
        };
        assert_eq!(next_wake(&[done], at("2026-10-10T15:00:00Z"), NOTICE), None);
    }

    const TOURNEY_LOG: &str = include_str!("../tests/fixtures/ranked-log-tourney-example.txt");
    const RANKED_LOG: &str = include_str!("../tests/fixtures/ranked-log-example.txt");

    #[test]
    fn uploads_a_tourney_log_as_its_lobbys_region() {
        let tourney = log_scan::scan(TOURNEY_LOG);
        let known = |key: &str| (key == "073518264903").then(|| "na".to_string());
        assert_eq!(
            upload_region(&tourney, known, Some("eu")).as_deref(),
            Some("na")
        );
        // Even for a host without a region picked or known.
        assert_eq!(upload_region(&tourney, known, None).as_deref(), Some("na"));
        // A lobby the tool doesn't know: the tool's region (the server sends it to review).
        assert_eq!(
            upload_region(&tourney, |_| None, Some("eu")).as_deref(),
            Some("eu")
        );
        // A ranked match: the tool's region.
        let ranked = log_scan::scan(RANKED_LOG);
        assert_eq!(
            upload_region(&ranked, known, Some("eu")).as_deref(),
            Some("eu")
        );
        assert_eq!(upload_region(&ranked, known, None), None);
        // A ranked and a tourney match in one file: the ranked one must go as the tool's.
        let both = log_scan::scan(&format!("{RANKED_LOG}{TOURNEY_LOG}"));
        assert_eq!(
            upload_region(&both, known, Some("eu")).as_deref(),
            Some("eu")
        );
        // Tourney matches of two regions in one file: the tool's.
        let other = TOURNEY_LOG
            .replace("219604738815", "219604738816")
            .replace("073518264903", "111");
        let two = log_scan::scan(&format!("{TOURNEY_LOG}{other}"));
        let mixed = |key: &str| Some(if key == "111" { "eu" } else { "na" }.to_string());
        assert_eq!(
            upload_region(&two, mixed, Some("eu")).as_deref(),
            Some("eu")
        );
    }

    #[test]
    fn knows_the_images_the_server_takes() {
        let png = b"\x89PNG\r\n\x1a\n rest";
        assert_eq!(image_type(png), Some("image/png"));
        assert_eq!(image_type(b"\xff\xd8\xff\xe0 jfif"), Some("image/jpeg"));
        assert_eq!(image_type(b"RIFF\x10\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(image_type(b"GIF89a"), None);
        assert_eq!(image_type(b"RIFF\x10\0\0\0WAVE"), None);
        assert_eq!(check_screenshot(png), Ok("image/png"));
        assert!(check_screenshot(b"").is_err());
        assert!(check_screenshot(b"<html>").unwrap_err().contains("PNG"));
        let mut big = png.to_vec();
        big.resize(config::SCREENSHOT_MAX_BYTES as usize + 1, 0);
        assert!(check_screenshot(&big).unwrap_err().contains("8 MB"));
        big.truncate(config::SCREENSHOT_MAX_BYTES as usize);
        assert_eq!(check_screenshot(&big), Ok("image/png"));
    }

    #[test]
    fn offers_the_newest_image_in_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(newest_screenshot(dir.path()).unwrap(), None);
        let old = dir.path().join("Overwatch_old.jpg");
        fs::write(&old, b"\xff\xd8\xff old").unwrap();
        let older = SystemTime::now() - Duration::from_secs(3600);
        fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(older)
            .unwrap();
        fs::write(dir.path().join("Overwatch_new.PNG"), b"\x89PNG\r\n\x1a\n").unwrap();
        // Not an image, though newer.
        fs::write(dir.path().join("notes.txt"), "x").unwrap();
        let newest = newest_screenshot(dir.path()).unwrap().unwrap();
        assert_eq!(newest.name, "Overwatch_new.PNG");
        assert_eq!(newest.bytes, 8);
        assert!(parse_time(&newest.taken_at).is_some());
    }

    #[test]
    fn reads_only_an_image_the_server_takes() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("standings.png");
        fs::write(&png, b"\x89PNG\r\n\x1a\nxx").unwrap();
        assert_eq!(read_screenshot(&png).unwrap().1, "image/png");
        // The content counts, not the name.
        let renamed = dir.path().join("standings");
        fs::write(&renamed, b"\xff\xd8\xffxx").unwrap();
        assert_eq!(read_screenshot(&renamed).unwrap().1, "image/jpeg");
        let text = dir.path().join("fake.png");
        fs::write(&text, "not an image").unwrap();
        assert!(read_screenshot(&text).is_err());
        assert!(read_screenshot(dir.path()).is_err());
        assert!(read_screenshot(&dir.path().join("gone.png")).is_err());
    }
}
