//! The settings file: what the host chose, with `None` meaning "use the default".
//! The host token is never here: it lives in the OS credential store (`credentials.rs`).

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use url::{Host, Url};

use crate::config::{self, Tunable};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Overrides `config::DEFAULT_SERVER_URL`. Stored normalized (`normalize_server_url`).
    pub server_url: Option<String>,
    /// Overrides the detected Workshop log folder.
    pub log_folder: Option<PathBuf>,
    /// Overrides the detected Overwatch screenshots folder, where the verify screenshot of a
    /// tourney lobby is offered from (#10).
    pub screenshot_folder: Option<PathBuf>,
    /// The region the host hosts in now, an id from `config::REGIONS`. `None`: the host's home
    /// region, which an admin sets on the server.
    pub region: Option<String>,
    /// Whether the host's lobby is listed on the site while a ranked match is played (#6).
    /// `None`: `config::LIVE_LOBBY_ON_BY_DEFAULT`.
    pub live_lobby: Option<bool>,
    /// The name the site lists the lobby under, as `normalize_lobby_name` keeps it. `None`: no name.
    pub lobby_name: Option<String>,
    /// How much the tool logs, one of `config::LOG_LEVELS`. `None`: `config::DEFAULT_LOG_LEVEL`.
    pub log_level: Option<String>,
    /// The GenjiBall-CE release the ranked code is built from (`1.3.3R`). `None`: the latest ranked
    /// release. Ends in `config::RELEASE_TAG_SUFFIX` (`normalize_release_tag`).
    pub release_tag: Option<String>,
    /// Whether uploads are a dry run: picked as usual, but not sent. `None`:
    /// `config::DRY_RUN_BY_DEFAULT`.
    pub dry_run: Option<bool>,
    /// Whether the tool looks for updates by itself. `None`: `config::AUTO_UPDATE_CHECK_BY_DEFAULT`.
    pub auto_update_check: Option<bool>,
    /// Where updates come from, one of `config::UPDATE_CHANNELS`. `None`:
    /// `config::DEFAULT_UPDATE_CHANNEL`.
    pub update_channel: Option<String>,
    /// The data center each region's codes put the lobby on, by region id: one of its
    /// `config::DATA_CENTERS` or `config::BEST_AVAILABLE`. A region at its default isn't here
    /// (`normalize_data_center`).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub data_centers: BTreeMap<String, String>,
    /// The `config::TUNABLES` the host changed under Advanced, by key. One left at its default
    /// isn't here (`normalize_advanced`). Keys this version doesn't know are kept, for a newer one.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub advanced: BTreeMap<String, u64>,
    /// The overlay and the stream page (#49, #55), as `normalize_overlay` keeps them. Left out
    /// while it's all at its defaults.
    #[serde(skip_serializing_if = "OverlaySettings::is_default")]
    pub overlay: OverlaySettings,
}

/// The overlay's settings. Each one at its default is `None`, or left out of its map, so a new
/// default reaches every host.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OverlaySettings {
    /// The overlay window. `None`: `config::OVERLAY_ON_BY_DEFAULT`.
    pub on: Option<bool>,
    /// The widgets the host switched away from their default in the overlay, by key
    /// (`config::OVERLAY_WIDGETS`).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub widgets: BTreeMap<String, bool>,
    /// In percent. `None`: `config::OVERLAY_OPACITY`'s default.
    pub opacity: Option<u16>,
    /// In percent. `None`: `config::OVERLAY_SCALE`'s default.
    pub scale: Option<u16>,
    /// `None`: `config::OVERLAY_ONLY_WITH_GAME_BY_DEFAULT`.
    pub only_with_game: Option<bool>,
    /// Where the host dragged each widget, by key: its top left corner as a share of the screen's
    /// width and height, 0 to 1. A widget that's not here sits in its default place.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub layout: BTreeMap<String, [f64; 2]>,
    /// How big the host made each widget, by key, as a share of its normal size
    /// (`config::OVERLAY_WIDGET_SIZE_MIN` to `_MAX`). A widget that's not here is its normal size.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub sizes: BTreeMap<String, f64>,
    /// The hotkeys the host changed, by action (`config::OVERLAY_HOTKEYS`): the keys as typed,
    /// empty for none.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub hotkeys: BTreeMap<String, String>,
    /// The stream page. `None`: `config::STREAM_ON_BY_DEFAULT`.
    pub stream: Option<bool>,
    /// `None`: `config::STREAM_PORT`'s default.
    pub stream_port: Option<u16>,
    /// The widgets switched away from their default on the stream page, by key.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub stream_widgets: BTreeMap<String, bool>,
}

impl OverlaySettings {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn on(&self) -> bool {
        self.on.unwrap_or(config::OVERLAY_ON_BY_DEFAULT)
    }

    pub fn stream_on(&self) -> bool {
        self.stream.unwrap_or(config::STREAM_ON_BY_DEFAULT)
    }

    pub fn opacity(&self) -> u16 {
        self.opacity.unwrap_or(config::OVERLAY_OPACITY.default)
    }

    pub fn scale(&self) -> u16 {
        self.scale.unwrap_or(config::OVERLAY_SCALE.default)
    }

    pub fn only_with_game(&self) -> bool {
        self.only_with_game
            .unwrap_or(config::OVERLAY_ONLY_WITH_GAME_BY_DEFAULT)
    }

    pub fn stream_port(&self) -> u16 {
        self.stream_port.unwrap_or(config::STREAM_PORT.default)
    }

    /// The keys of the widgets on in the overlay, or on the stream page, in `OVERLAY_WIDGETS`
    /// order.
    pub fn widgets_on(&self, stream: bool) -> Vec<&'static str> {
        let chosen = if stream {
            &self.stream_widgets
        } else {
            &self.widgets
        };
        config::OVERLAY_WIDGETS
            .iter()
            .filter(|w| {
                chosen
                    .get(w.key)
                    .copied()
                    .unwrap_or(if stream { w.stream } else { w.overlay })
            })
            .map(|w| w.key)
            .collect()
    }

    /// The keys of a hotkey, `None` when the host cleared it.
    pub fn hotkey(&self, action: &str) -> Option<String> {
        let keys = match self.hotkeys.get(action) {
            Some(keys) => keys.clone(),
            None => config::OVERLAY_HOTKEYS
                .iter()
                .find(|h| h.action == action)?
                .default
                .to_string(),
        };
        (!keys.is_empty()).then_some(keys)
    }
}

/// The overlay settings as they're stored: values at their default left out, numbers checked
/// against their range, layout places kept on the screen, and hotkeys checked
/// (`parse_hotkey`), none twice. Widgets, places and hotkeys this version doesn't know are
/// dropped.
pub fn normalize_overlay(input: &OverlaySettings) -> Result<OverlaySettings, String> {
    let range = |value: Option<u16>, r: &config::OverlayRange, what: &str| match value {
        Some(v) if !(r.min..=r.max).contains(&v) => {
            Err(format!("The {what} must be {} to {}", r.min, r.max))
        }
        Some(v) if v == r.default => Ok(None),
        v => Ok(v),
    };
    let widgets = |chosen: &BTreeMap<String, bool>, stream: bool| {
        config::OVERLAY_WIDGETS
            .iter()
            .filter_map(|w| {
                let on = *chosen.get(w.key)?;
                let default = if stream { w.stream } else { w.overlay };
                (on != default).then(|| (w.key.to_string(), on))
            })
            .collect::<BTreeMap<_, _>>()
    };
    let layout = input
        .layout
        .iter()
        .filter(|(key, _)| config::OVERLAY_WIDGETS.iter().any(|w| w.key == *key))
        .filter(|(_, place)| place.iter().all(|v| v.is_finite()))
        .map(|(key, [x, y])| (key.clone(), [x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)]))
        .collect();
    let sizes = input
        .sizes
        .iter()
        .filter(|(key, _)| config::OVERLAY_WIDGETS.iter().any(|w| w.key == *key))
        .filter(|(_, size)| size.is_finite())
        .map(|(key, size)| {
            let size = size.clamp(
                config::OVERLAY_WIDGET_SIZE_MIN,
                config::OVERLAY_WIDGET_SIZE_MAX,
            );
            (key.clone(), (size * 100.0).round() / 100.0)
        })
        .filter(|(_, size)| *size != 1.0)
        .collect();
    let mut hotkeys = BTreeMap::new();
    let mut taken: Vec<(u32, &str)> = Vec::new();
    for hotkey in &config::OVERLAY_HOTKEYS {
        let typed = input.hotkeys.get(hotkey.action).map(|k| k.trim());
        let keys = typed.unwrap_or(hotkey.default);
        if !keys.is_empty() {
            let id = parse_hotkey(keys).map_err(|e| format!("{}: {e}", hotkey.label))?;
            if let Some((_, other)) = taken.iter().find(|(other, _)| *other == id) {
                return Err(format!("{} has the same keys as {other}", hotkey.label));
            }
            taken.push((id, hotkey.label));
        }
        if let Some(typed) = typed.filter(|&t| t != hotkey.default) {
            hotkeys.insert(hotkey.action.to_string(), typed.to_string());
        }
    }
    Ok(OverlaySettings {
        on: input.on.filter(|&on| on != config::OVERLAY_ON_BY_DEFAULT),
        widgets: widgets(&input.widgets, false),
        opacity: range(input.opacity, &config::OVERLAY_OPACITY, "opacity")?,
        scale: range(input.scale, &config::OVERLAY_SCALE, "size")?,
        only_with_game: input
            .only_with_game
            .filter(|&on| on != config::OVERLAY_ONLY_WITH_GAME_BY_DEFAULT),
        layout,
        sizes,
        hotkeys,
        stream: input
            .stream
            .filter(|&on| on != config::STREAM_ON_BY_DEFAULT),
        stream_port: range(
            input.stream_port,
            &config::STREAM_PORT,
            "stream page's port",
        )?,
        stream_widgets: widgets(&input.stream_widgets, true),
    })
}

/// A hotkey's keys (`Ctrl+Alt+O`), as its id. It needs Ctrl, Alt or the Windows key: Windows
/// takes a hotkey before the game sees it, so a plain key, or Shift and a key, would be lost to the
/// game.
pub fn parse_hotkey(keys: &str) -> Result<u32, String> {
    use tauri_plugin_global_shortcut::{Modifiers, Shortcut};
    let shortcut: Shortcut = keys
        .parse()
        .map_err(|_| format!("\"{keys}\" isn't a key combination, like Ctrl+Alt+O"))?;
    if !shortcut
        .mods
        .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER)
    {
        return Err(format!(
            "\"{keys}\" needs Ctrl, Alt or the Windows key, or the game would lose that key"
        ));
    }
    Ok(shortcut.id())
}

impl Settings {
    pub fn server_url(&self) -> &str {
        self.server_url
            .as_deref()
            .unwrap_or(config::DEFAULT_SERVER_URL)
    }

    /// Whether the live lobby is on.
    pub fn live_lobby_on(&self) -> bool {
        self.live_lobby.unwrap_or(config::LIVE_LOBBY_ON_BY_DEFAULT)
    }

    /// Switches the live lobby on or off. At the default it's stored as `None`.
    pub fn set_live_lobby(&mut self, on: bool) {
        self.live_lobby = (on != config::LIVE_LOBBY_ON_BY_DEFAULT).then_some(on);
    }

    /// Whether uploads are a dry run.
    pub fn dry_run_on(&self) -> bool {
        self.dry_run.unwrap_or(config::DRY_RUN_BY_DEFAULT)
    }

    /// Switches the dry run on or off. At the default it's stored as `None`.
    pub fn set_dry_run(&mut self, on: bool) {
        self.dry_run = (on != config::DRY_RUN_BY_DEFAULT).then_some(on);
    }

    /// Whether the tool looks for updates by itself.
    pub fn auto_update_check_on(&self) -> bool {
        self.auto_update_check
            .unwrap_or(config::AUTO_UPDATE_CHECK_BY_DEFAULT)
    }

    /// Switches the automatic update checks on or off. At the default it's stored as `None`.
    pub fn set_auto_update_check(&mut self, on: bool) {
        self.auto_update_check = (on != config::AUTO_UPDATE_CHECK_BY_DEFAULT).then_some(on);
    }

    /// The update channel the host picked, or the default.
    pub fn update_channel(&self) -> &str {
        self.update_channel
            .as_deref()
            .unwrap_or(config::DEFAULT_UPDATE_CHANNEL)
    }

    /// The data center `region`'s codes put the lobby on: the host's pick, else the region's
    /// default. `None` for `config::BEST_AVAILABLE` (the game picks), or a region without any.
    pub fn data_center(&self, region: &str) -> Option<&str> {
        let name = match self.data_centers.get(region) {
            Some(name) => name.as_str(),
            None => config::DATA_CENTERS
                .iter()
                .find(|d| d.region == region)?
                .names
                .first()?,
        };
        (name != config::BEST_AVAILABLE).then_some(name)
    }

    /// The host's value for `tunable`, or its default.
    pub fn get(&self, tunable: &Tunable) -> u64 {
        self.advanced
            .get(tunable.key)
            .copied()
            .unwrap_or(tunable.default)
    }

    /// `get`, as a duration: every tunable is in seconds.
    pub fn secs(&self, tunable: &Tunable) -> Duration {
        Duration::from_secs(self.get(tunable))
    }

    /// The log level the host picked, or the default.
    pub fn log_level(&self) -> &str {
        self.log_level
            .as_deref()
            .unwrap_or(config::DEFAULT_LOG_LEVEL)
    }

    /// Every tunable's value, from `values` (`normalize_advanced`): one left out goes back to its
    /// default. Keys a newer version wrote stay.
    pub fn replace_advanced(&mut self, values: BTreeMap<String, u64>) {
        self.advanced
            .retain(|key, _| !config::TUNABLES.iter().any(|t| t.key == key));
        self.advanced.extend(values);
    }
}

/// The Advanced values the host entered, by key, as they're stored: each checked against its
/// range, and one at its default left out. A key that isn't a tunable is an error.
pub fn normalize_advanced(values: &BTreeMap<String, u64>) -> Result<BTreeMap<String, u64>, String> {
    let mut normalized = BTreeMap::new();
    for (key, &value) in values {
        let tunable = config::TUNABLES
            .iter()
            .find(|t| t.key == key)
            .ok_or_else(|| format!("There's no setting {key}"))?;
        check(tunable, value)?;
        if value != tunable.default {
            normalized.insert(key.clone(), value);
        }
    }
    Ok(normalized)
}

fn check(tunable: &Tunable, value: u64) -> Result<(), String> {
    if (tunable.min..=tunable.max).contains(&value) {
        Ok(())
    } else {
        Err(format!(
            "{} must be {} to {} seconds",
            tunable.label, tunable.min, tunable.max
        ))
    }
}

/// `Ok` for an id in `config::REGIONS`.
pub fn check_region(region: &str) -> Result<(), String> {
    if config::REGIONS.iter().any(|r| r.id == region) {
        Ok(())
    } else {
        Err(format!("There's no region {region}"))
    }
}

/// A lobby name as the host typed it, as it's stored and sent: spaces around it dropped, `None`
/// when empty. One the server would refuse (over `config::LOBBY_NAME_MAX_CHARS` characters) or
/// with a line break is an error.
pub fn normalize_lobby_name(input: &str) -> Result<Option<String>, String> {
    let name = input.trim();
    if name.chars().count() > config::LOBBY_NAME_MAX_CHARS {
        return Err(format!(
            "The lobby name can be at most {} characters",
            config::LOBBY_NAME_MAX_CHARS
        ));
    }
    if name.chars().any(char::is_control) {
        return Err("The lobby name can't have line breaks or tabs".into());
    }
    Ok((!name.is_empty()).then(|| name.to_string()))
}

/// A log level the host picked, as it's stored: `None` for the default (or nothing picked).
pub fn normalize_log_level(level: Option<&str>) -> Result<Option<String>, String> {
    match level {
        None => Ok(None),
        Some(level) if level == config::DEFAULT_LOG_LEVEL => Ok(None),
        Some(level) if config::LOG_LEVELS.contains(&level) => Ok(Some(level.to_string())),
        Some(level) => Err(format!("There's no log level {level}")),
    }
}

/// An update channel the host picked, as it's stored: `None` for the default (or nothing picked).
pub fn normalize_update_channel(channel: Option<&str>) -> Result<Option<String>, String> {
    match channel {
        None => Ok(None),
        Some(channel) if channel == config::DEFAULT_UPDATE_CHANNEL => Ok(None),
        Some(channel) if config::UPDATE_CHANNELS.contains(&channel) => {
            Ok(Some(channel.to_string()))
        }
        Some(channel) => Err(format!("There's no update channel {channel}")),
    }
}

/// A data center the host picked for `region`, as it's stored: `None` for the region's default.
pub fn normalize_data_center(region: &str, name: &str) -> Result<Option<String>, String> {
    let centers = config::DATA_CENTERS
        .iter()
        .find(|d| d.region == region)
        .ok_or_else(|| format!("There's no region {region}"))?;
    if centers.names.first() == Some(&name) {
        Ok(None)
    } else if name == config::BEST_AVAILABLE || centers.names.contains(&name) {
        Ok(Some(name.to_string()))
    } else {
        Err(format!("There's no data center {name} for {region}"))
    }
}

/// The GenjiBall-CE release tag the host typed, as it's stored: `None` (empty) for the latest
/// ranked release. A ranked tag ends in `config::RELEASE_TAG_SUFFIX`; only letters, digits, `.`,
/// `-` and `_`, so it goes in a GitHub URL as it is.
pub fn normalize_release_tag(input: &str) -> Result<Option<String>, String> {
    let tag = input.trim();
    if tag.is_empty() {
        return Ok(None);
    }
    let suffix = config::RELEASE_TAG_SUFFIX;
    let ranked = tag.len() > suffix.len() && tag.ends_with(suffix);
    let plain = tag
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    if ranked && plain {
        Ok(Some(tag.to_string()))
    } else {
        Err(format!(
            "A ranked release's tag ends in {suffix}, like 1.3.3{suffix}. Leave it empty for the latest"
        ))
    }
}

/// The settings in `path`, or the defaults when the file doesn't exist yet. A file that can't be
/// read as settings is an error rather than silently reset, so a typo doesn't lose the others.
/// So is a server URL the window wouldn't take (`normalize_server_url`): the token is sent there,
/// and an Advanced value out of its range.
pub fn load(path: &Path) -> Result<Settings, String> {
    let mut settings: Settings = match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| format!("{} isn't valid settings: {e}", path.display()))?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Settings::default()),
        Err(e) => return Err(format!("Couldn't read {}: {e}", path.display())),
    };
    if let Some(url) = &settings.server_url {
        settings.server_url = normalize_server_url(url)
            .map_err(|e| format!("The server URL in {} won't do ({e})", path.display()))?;
    }
    // Edited by hand: the server would refuse every upload with a region it doesn't know.
    if let Some(region) = &settings.region {
        check_region(region).map_err(|e| format!("{e} in {}", path.display()))?;
    }
    // Edited by hand: the server would refuse every heartbeat with it.
    if let Some(name) = &settings.lobby_name {
        settings.lobby_name =
            normalize_lobby_name(name).map_err(|e| format!("{e} (in {})", path.display()))?;
    }
    // Edited by hand, like the region: stored as the window would have.
    settings.log_level = normalize_log_level(settings.log_level.as_deref())
        .map_err(|e| format!("{e} in {}", path.display()))?;
    settings.update_channel = normalize_update_channel(settings.update_channel.as_deref())
        .map_err(|e| format!("{e} in {}", path.display()))?;
    if let Some(tag) = &settings.release_tag {
        settings.release_tag =
            normalize_release_tag(tag).map_err(|e| format!("{e} (in {})", path.display()))?;
    }
    // Edited by hand: the Workshop wouldn't take the code with a data center it doesn't know.
    let mut data_centers = BTreeMap::new();
    for (region, name) in &settings.data_centers {
        if let Some(name) =
            normalize_data_center(region, name).map_err(|e| format!("{e} in {}", path.display()))?
        {
            data_centers.insert(region.clone(), name);
        }
    }
    settings.data_centers = data_centers;
    // Edited by hand. Unlike the rest, an overlay setting the window wouldn't take never holds
    // uploads up: the overlay goes back to its defaults (off) instead.
    settings.overlay = normalize_overlay(&settings.overlay).unwrap_or_else(|e| {
        log::warn!("Overlay settings in {} reset: {e}", path.display());
        OverlaySettings::default()
    });
    // Edited by hand: a value out of range (a quiet time of 0, say) would upload every match
    // half-played.
    for tunable in config::TUNABLES {
        if let Some(&value) = settings.advanced.get(tunable.key) {
            check(tunable, value).map_err(|e| format!("{e} in {}", path.display()))?;
        }
    }
    Ok(settings)
}

pub fn save(path: &Path, settings: &Settings) -> Result<(), String> {
    save_json(path, settings)
}

/// Writes `value` to a temporary file next to `path`, then moves it over, so a crash mid-write
/// never leaves half a file. Every JSON file the tool keeps is written through here. The
/// temporary file's name is this write's own, so two writes at once never share one.
pub fn save_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    static WRITES: AtomicU64 = AtomicU64::new(0);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let write = WRITES.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("json.{}-{write}.tmp", std::process::id()));
    fs::write(&tmp, text).map_err(|e| format!("Couldn't write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("Couldn't write {}: {e}", path.display())
    })
}

/// A server URL as the host typed it, as `scheme://host[:port][/path]` without a trailing slash,
/// or why it's not one. Empty means "use the default" (`None`).
///
/// The token is sent to this URL, so it must be `https://`, except `http://` to this PC
/// (`localhost` or a loopback address) for a local `wrangler dev`. The host is checked on the
/// parsed URL, so `http://localhost@example.com` (host `example.com`) is refused.
pub fn normalize_server_url(input: &str) -> Result<Option<String>, String> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(None);
    }
    let not_a_url = || "That isn't a server URL".to_string();
    let url = Url::parse(input).map_err(|_| {
        if input.contains("://") {
            not_a_url()
        } else {
            "The server URL starts with https://".to_string()
        }
    })?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(not_a_url());
    }
    let host = url.host().ok_or_else(not_a_url)?;
    match url.scheme() {
        "https" => {}
        "http" if is_this_pc(&host) => {}
        "http" => {
            return Err(
                "Use https://. Plain http:// is only for a server on this PC (localhost)".into(),
            )
        }
        _ => return Err("The server URL starts with https://".into()),
    }
    let normalized = url.as_str().trim_end_matches('/').to_string();
    Ok(if normalized == config::DEFAULT_SERVER_URL {
        None
    } else {
        Some(normalized)
    })
}

fn is_this_pc(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(name) => name.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(ip) => ip.is_loopback(),
        Host::Ipv6(ip) => ip.is_loopback(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let settings = load(&dir.path().join("settings.json")).unwrap();
        assert_eq!(settings, Settings::default());
        assert_eq!(settings.server_url(), config::DEFAULT_SERVER_URL);
    }

    #[test]
    fn saves_and_loads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("settings.json");
        let settings = Settings {
            server_url: Some("http://localhost:8787".into()),
            log_folder: Some(PathBuf::from(r"D:\Logs")),
            screenshot_folder: Some(PathBuf::from(r"D:\Shots")),
            region: Some("na".into()),
            live_lobby: Some(false),
            lobby_name: Some("Kenzo's ranked".into()),
            log_level: Some("debug".into()),
            release_tag: Some("1.3.3R".into()),
            dry_run: Some(true),
            auto_update_check: Some(false),
            update_channel: Some("prerelease".into()),
            data_centers: BTreeMap::from([("eu".into(), "Germany".into())]),
            advanced: BTreeMap::from([("quietSecs".into(), 90)]),
            overlay: OverlaySettings {
                on: Some(true),
                widgets: BTreeMap::from([("killFeed".into(), true)]),
                opacity: Some(80),
                scale: Some(120),
                only_with_game: Some(false),
                layout: BTreeMap::from([("roster".into(), [0.25, 0.5])]),
                sizes: BTreeMap::from([("roster".into(), 1.5)]),
                hotkeys: BTreeMap::from([("afk".into(), "Ctrl+Shift+F9".into())]),
                stream: Some(true),
                stream_port: Some(8000),
                stream_widgets: BTreeMap::from([("roster".into(), false)]),
            },
        };
        save(&path, &settings).unwrap();
        assert_eq!(load(&path).unwrap(), settings);
        // No temporary file left behind.
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn writes_at_the_same_time_dont_share_a_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("uploads.json");
        let writers: Vec<_> = (0..8)
            .map(|n| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for _ in 0..20 {
                        // Renaming over a file another write is renaming over may be refused on
                        // Windows; what mustn't happen is half a file or another write's text.
                        let _ = save_json(&path, &vec![n; 1000]);
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        let text = fs::read_to_string(&path).unwrap();
        let written: Vec<u32> = serde_json::from_str(&text).unwrap();
        assert!(written.iter().all(|&n| n == written[0]) && written.len() == 1000);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn ignores_unknown_and_missing_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            r#"{ "serverUrl": "http://localhost:8787", "later": 1 }"#,
        )
        .unwrap();
        assert_eq!(load(&path).unwrap().server_url(), "http://localhost:8787");
    }

    #[test]
    fn checks_the_server_url_in_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        // Edited by hand: plain http to another PC would send it the token unencrypted.
        for bad in [
            "http://192.168.1.20:8787",
            "https://user:pass@genjiball.us",
            "nope",
        ] {
            fs::write(&path, format!(r#"{{ "serverUrl": "{bad}" }}"#)).unwrap();
            assert!(load(&path).is_err(), "{bad}");
        }
        // Stored as the window would have: normalized, and the default as `None`.
        fs::write(&path, r#"{ "serverUrl": "https://test.genjiball.us/" }"#).unwrap();
        assert_eq!(
            load(&path).unwrap().server_url.as_deref(),
            Some("https://test.genjiball.us")
        );
        fs::write(&path, r#"{ "serverUrl": "https://genjiball.us" }"#).unwrap();
        assert_eq!(load(&path).unwrap().server_url, None);
    }

    #[test]
    fn advanced_values_default_until_set() {
        let mut settings = Settings::default();
        assert_eq!(
            settings.get(&config::QUIET_SECS),
            config::QUIET_SECS.default
        );
        settings.advanced.insert("quietSecs".into(), 90);
        assert_eq!(settings.secs(&config::QUIET_SECS), Duration::from_secs(90));
        assert_eq!(
            settings.get(&config::POLL_INTERVAL_SECS),
            config::POLL_INTERVAL_SECS.default
        );
    }

    #[test]
    fn normalizes_advanced_values() {
        let entered = BTreeMap::from([
            ("quietSecs".to_string(), 90),
            // At its default: not stored, so a new default reaches this host.
            (
                "pollIntervalSecs".to_string(),
                config::POLL_INTERVAL_SECS.default,
            ),
        ]);
        assert_eq!(
            normalize_advanced(&entered),
            Ok(BTreeMap::from([("quietSecs".to_string(), 90)]))
        );
        let too_short = BTreeMap::from([("quietSecs".to_string(), 0)]);
        assert!(normalize_advanced(&too_short)
            .unwrap_err()
            .contains("Quiet time"));
        let unknown = BTreeMap::from([("nope".to_string(), 1)]);
        assert!(normalize_advanced(&unknown).is_err());
    }

    #[test]
    fn replacing_advanced_values_keeps_unknown_keys() {
        let mut settings = Settings {
            advanced: BTreeMap::from([("quietSecs".into(), 90), ("later".into(), 1)]),
            ..Settings::default()
        };
        settings.replace_advanced(BTreeMap::from([("pollIntervalSecs".into(), 2)]));
        assert_eq!(
            settings.advanced,
            BTreeMap::from([("later".into(), 1), ("pollIntervalSecs".into(), 2)])
        );
        settings.replace_advanced(BTreeMap::new());
        assert_eq!(settings.advanced, BTreeMap::from([("later".into(), 1)]));
    }

    #[test]
    fn checks_the_advanced_values_in_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, r#"{ "advanced": { "quietSecs": 0 } }"#).unwrap();
        assert!(load(&path).unwrap_err().contains("Quiet time"));
        fs::write(&path, r#"{ "advanced": { "quietSecs": -1 } }"#).unwrap();
        assert!(load(&path).is_err());
        // One a newer version added is kept, not refused.
        fs::write(&path, r#"{ "advanced": { "quietSecs": 90, "later": 0 } }"#).unwrap();
        let settings = load(&path).unwrap();
        assert_eq!(settings.get(&config::QUIET_SECS), 90);
        assert_eq!(settings.advanced.get("later"), Some(&0));
    }

    #[test]
    fn checks_the_region_in_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, r#"{ "region": "na" }"#).unwrap();
        assert_eq!(load(&path).unwrap().region.as_deref(), Some("na"));
        // Left at the home region.
        fs::write(&path, r#"{ "region": null }"#).unwrap();
        assert_eq!(load(&path).unwrap().region, None);
        for bad in ["NA", "us", ""] {
            fs::write(&path, format!(r#"{{ "region": "{bad}" }}"#)).unwrap();
            assert!(load(&path).unwrap_err().contains("region"), "{bad}");
        }
    }

    #[test]
    fn the_live_lobby_is_on_until_switched_off() {
        let mut settings = Settings::default();
        assert!(settings.live_lobby_on());
        settings.set_live_lobby(false);
        assert_eq!(settings.live_lobby, Some(false));
        assert!(!settings.live_lobby_on());
        // Back at the default: stored as `null`, so a new default reaches this host.
        settings.set_live_lobby(true);
        assert_eq!(settings.live_lobby, None);
    }

    #[test]
    fn the_dry_run_is_off_until_switched_on() {
        let mut settings = Settings::default();
        assert!(!settings.dry_run_on());
        settings.set_dry_run(true);
        assert_eq!(settings.dry_run, Some(true));
        assert!(settings.dry_run_on());
        settings.set_dry_run(false);
        assert_eq!(settings.dry_run, None);
    }

    #[test]
    fn normalizes_lobby_names() {
        assert_eq!(
            normalize_lobby_name("  Kenzo's ranked "),
            Ok(Some("Kenzo's ranked".into()))
        );
        assert_eq!(normalize_lobby_name("   "), Ok(None));
        // Characters, not bytes: the server counts them the same way.
        let longest = "é".repeat(config::LOBBY_NAME_MAX_CHARS);
        assert_eq!(normalize_lobby_name(&longest), Ok(Some(longest.clone())));
        assert!(normalize_lobby_name(&format!("{longest}e")).is_err());
        assert!(normalize_lobby_name("two\nlines").is_err());
    }

    #[test]
    fn checks_the_lobby_name_in_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            r#"{ "lobbyName": " EU night ", "liveLobby": false }"#,
        )
        .unwrap();
        let settings = load(&path).unwrap();
        assert_eq!(settings.lobby_name.as_deref(), Some("EU night"));
        assert!(!settings.live_lobby_on());
        let long = "x".repeat(config::LOBBY_NAME_MAX_CHARS + 1);
        fs::write(&path, format!(r#"{{ "lobbyName": "{long}" }}"#)).unwrap();
        assert!(load(&path).unwrap_err().contains("lobby name"));
    }

    #[test]
    fn log_levels_are_stored_as_null_at_the_default() {
        assert_eq!(normalize_log_level(None), Ok(None));
        assert_eq!(
            normalize_log_level(Some(config::DEFAULT_LOG_LEVEL)),
            Ok(None)
        );
        assert_eq!(normalize_log_level(Some("debug")), Ok(Some("debug".into())));
        assert!(normalize_log_level(Some("trace")).is_err());
        assert!(normalize_log_level(Some("DEBUG")).is_err());
        assert_eq!(Settings::default().log_level(), config::DEFAULT_LOG_LEVEL);
    }

    #[test]
    fn checks_the_log_level_in_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, r#"{ "logLevel": "warn" }"#).unwrap();
        assert_eq!(load(&path).unwrap().log_level(), "warn");
        fs::write(&path, r#"{ "logLevel": "info" }"#).unwrap();
        assert_eq!(load(&path).unwrap().log_level, None);
        fs::write(&path, r#"{ "logLevel": "loud" }"#).unwrap();
        assert!(load(&path).unwrap_err().contains("log level"));
    }

    #[test]
    fn data_centers_default_per_region_and_best_available_sets_none() {
        let mut settings = Settings::default();
        assert_eq!(settings.data_center("eu"), Some("Netherlands"));
        assert_eq!(settings.data_center("na"), Some("USA - Central"));
        assert_eq!(normalize_data_center("eu", "Netherlands"), Ok(None));
        assert_eq!(
            normalize_data_center("eu", "Germany"),
            Ok(Some("Germany".into()))
        );
        assert!(normalize_data_center("eu", "USA - East").is_err());
        assert!(normalize_data_center("asia", "Japan").is_err());
        settings
            .data_centers
            .insert("na".into(), config::BEST_AVAILABLE.into());
        assert_eq!(settings.data_center("na"), None);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            r#"{ "dataCenters": { "eu": "Netherlands", "na": "USA - West" } }"#,
        )
        .unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(
            loaded.data_centers,
            BTreeMap::from([("na".into(), "USA - West".into())])
        );
        fs::write(&path, r#"{ "dataCenters": { "eu": "Atlantis" } }"#).unwrap();
        assert!(load(&path).is_err());
    }

    #[test]
    fn update_channels_are_stored_as_null_at_the_default() {
        assert_eq!(normalize_update_channel(Some("stable")), Ok(None));
        assert_eq!(
            normalize_update_channel(Some("prerelease")),
            Ok(Some("prerelease".into()))
        );
        assert!(normalize_update_channel(Some("nightly")).is_err());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, r#"{ "updateChannel": "nightly" }"#).unwrap();
        assert!(load(&path).unwrap_err().contains("update channel"));
    }

    #[test]
    fn update_checks_are_on_until_switched_off() {
        let mut settings = Settings::default();
        assert!(settings.auto_update_check_on());
        settings.set_auto_update_check(false);
        assert_eq!(settings.auto_update_check, Some(false));
        settings.set_auto_update_check(true);
        assert_eq!(settings.auto_update_check, None);
    }

    #[test]
    fn a_pinned_release_is_a_ranked_tag() {
        assert_eq!(normalize_release_tag(" 1.3.3R "), Ok(Some("1.3.3R".into())));
        assert_eq!(normalize_release_tag(""), Ok(None));
        assert_eq!(normalize_release_tag("  "), Ok(None));
        for bad in ["1.3.3", "1.3.3T", "R", "1.3.3 R", "../1.3.3R", "1.3.3R?x=R"] {
            assert!(
                normalize_release_tag(bad)
                    .unwrap_err()
                    .contains("ends in R"),
                "{bad}"
            );
        }
    }

    #[test]
    fn checks_the_release_tag_in_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, r#"{ "releaseTag": "1.3.3R" }"#).unwrap();
        assert_eq!(load(&path).unwrap().release_tag.as_deref(), Some("1.3.3R"));
        fs::write(&path, r#"{ "releaseTag": "" }"#).unwrap();
        assert_eq!(load(&path).unwrap().release_tag, None);
        fs::write(&path, r#"{ "releaseTag": "1.3.3" }"#).unwrap();
        assert!(load(&path).is_err());
    }

    #[test]
    fn the_overlay_is_off_with_its_default_widgets_until_changed() {
        let overlay = OverlaySettings::default();
        assert!(!overlay.on());
        assert!(!overlay.stream_on());
        assert_eq!(overlay.opacity(), config::OVERLAY_OPACITY.default);
        assert_eq!(overlay.hotkey("toggle").as_deref(), Some("Ctrl+Alt+O"));
        let on = overlay.widgets_on(false);
        assert!(on.contains(&"afk") && !on.contains(&"killFeed"));
        let stream = overlay.widgets_on(true);
        assert!(stream.contains(&"killFeed") && !stream.contains(&"afk"));
    }

    #[test]
    fn normalizes_overlay_settings() {
        let input = OverlaySettings {
            on: Some(false),
            widgets: BTreeMap::from([
                ("afk".into(), true),
                ("killFeed".into(), true),
                ("fromANewerVersion".into(), true),
            ]),
            opacity: Some(config::OVERLAY_OPACITY.default),
            layout: BTreeMap::from([
                ("roster".into(), [1.5, -0.2]),
                ("standings".into(), [f64::NAN, 0.5]),
                ("nope".into(), [0.5, 0.5]),
            ]),
            sizes: BTreeMap::from([
                ("roster".into(), 9.0),
                ("standings".into(), 1.0),
                ("killFeed".into(), 1.234),
                ("nope".into(), 2.0),
            ]),
            hotkeys: BTreeMap::from([
                ("toggle".into(), " Ctrl+Alt+O ".into()),
                ("edit".into(), "".into()),
            ]),
            stream_widgets: BTreeMap::from([("killFeed".into(), false)]),
            ..OverlaySettings::default()
        };
        let stored = normalize_overlay(&input).unwrap();
        assert_eq!(
            stored,
            OverlaySettings {
                widgets: BTreeMap::from([("killFeed".into(), true)]),
                layout: BTreeMap::from([("roster".into(), [1.0, 0.0])]),
                // Kept in range, rounded, and left out at the normal size.
                sizes: BTreeMap::from([
                    ("killFeed".into(), 1.23),
                    ("roster".into(), config::OVERLAY_WIDGET_SIZE_MAX),
                ]),
                hotkeys: BTreeMap::from([("edit".into(), "".into())]),
                stream_widgets: BTreeMap::from([("killFeed".into(), false)]),
                ..OverlaySettings::default()
            }
        );
        assert_eq!(stored.hotkey("edit"), None);
        assert!(normalize_overlay(&OverlaySettings::default())
            .unwrap()
            .is_default());
    }

    #[test]
    fn overlay_numbers_and_hotkeys_are_checked() {
        let with = |change: fn(&mut OverlaySettings)| {
            let mut overlay = OverlaySettings::default();
            change(&mut overlay);
            normalize_overlay(&overlay)
        };
        assert!(with(|o| o.opacity = Some(5)).is_err());
        assert!(with(|o| o.stream_port = Some(80)).is_err());
        assert!(with(|o| o.hotkeys = BTreeMap::from([("afk".into(), "Banana".into())])).is_err());
        // A key the game would lose.
        assert!(with(|o| o.hotkeys = BTreeMap::from([("afk".into(), "F".into())])).is_err());
        assert!(with(|o| o.hotkeys = BTreeMap::from([("afk".into(), "Shift+F".into())])).is_err());
        // The same keys as another action, typed another way.
        assert!(
            with(|o| o.hotkeys = BTreeMap::from([("afk".into(), "alt+control+o".into())])).is_err()
        );
        assert!(with(|o| o.hotkeys = BTreeMap::from([("afk".into(), "Ctrl+F9".into())])).is_ok());
    }

    #[test]
    fn bad_overlay_settings_in_the_file_reset_the_overlay_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            r#"{ "region": "na", "overlay": { "on": true, "hotkeys": { "afk": "Q" } } }"#,
        )
        .unwrap();
        let settings = load(&path).unwrap();
        assert_eq!(settings.region.as_deref(), Some("na"));
        assert_eq!(settings.overlay, OverlaySettings::default());
    }

    #[test]
    fn a_broken_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, "{ not json").unwrap();
        assert!(load(&path).is_err());
    }

    #[test]
    fn normalizes_server_urls() {
        assert_eq!(
            normalize_server_url(" https://test.genjiball.us/ "),
            Ok(Some("https://test.genjiball.us".into()))
        );
        assert_eq!(
            normalize_server_url("http://localhost:8787"),
            Ok(Some("http://localhost:8787".into()))
        );
        assert_eq!(normalize_server_url(""), Ok(None));
        assert_eq!(normalize_server_url("https://genjiball.us/"), Ok(None));
        assert!(normalize_server_url("genjiball.us").is_err());
        assert!(normalize_server_url("https://").is_err());
        assert!(normalize_server_url("https://genji ball.us").is_err());
    }

    #[test]
    fn allows_plain_http_only_to_this_pc() {
        for local in [
            "http://localhost:8787",
            "http://LOCALHOST:8787",
            "http://127.0.0.1:8787",
            "http://[::1]:8787",
        ] {
            assert!(normalize_server_url(local).is_ok(), "{local}");
        }
        for remote in [
            "http://genjiball.us",
            "http://192.168.1.20:8787",
            "http://localhost.example.com",
            // The host here is example.com: "localhost" is a user name.
            "http://localhost@example.com",
            "http://localhost:secret@example.com",
        ] {
            assert!(normalize_server_url(remote).is_err(), "{remote}");
        }
    }

    #[test]
    fn refuses_credentials_queries_and_other_schemes() {
        for bad in [
            "https://user:pass@genjiball.us",
            "https://genjiball.us/?x=1",
            "https://genjiball.us/#x",
            "ftp://genjiball.us",
            "file:///C:/x",
        ] {
            assert!(normalize_server_url(bad).is_err(), "{bad}");
        }
    }
}
