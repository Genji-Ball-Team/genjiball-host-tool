//! Talking to the ranked server (genjiball-ranked `docs/api.md`).

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config;

/// The host a token belongs to, from `GET /api/host/me`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Host {
    pub id: i64,
    pub name: String,
    /// `trusted` or `untrusted`: an untrusted host's matches wait for an admin.
    pub trust: String,
    /// The home region, which an admin sets: what an upload without `X-Region` is stored as.
    /// `None`: every upload must say its region.
    #[serde(default)]
    pub region: Option<String>,
}

/// What checking a token found.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "camelCase")]
pub enum TokenCheck {
    Ok {
        host: Host,
    },
    /// The server doesn't know the token (401).
    Unknown,
    /// An admin revoked it (403). Stop using it.
    Revoked,
    /// No answer, or one we can't read. The token may be fine.
    Unreachable {
        message: String,
    },
}

impl TokenCheck {
    /// The server answered that the token won't work.
    pub fn is_rejected(&self) -> bool {
        matches!(self, TokenCheck::Unknown | TokenCheck::Revoked)
    }

    /// What the check found, for the log: whose token it is, never the token.
    pub fn summary(&self) -> String {
        match self {
            TokenCheck::Ok { host } => format!(
                "works (host {}, {}, home region {})",
                host.name,
                host.trust,
                host.region.as_deref().unwrap_or("none")
            ),
            TokenCheck::Unknown => "unknown to the server (401)".into(),
            TokenCheck::Revoked => "revoked (403)".into(),
            TokenCheck::Unreachable { message } => format!("not checked: {message}"),
        }
    }
}

#[derive(Deserialize)]
struct HostMe {
    host: Host,
}

/// What a `GET /api/host/me` answer means.
pub fn read_token_check(status: u16, body: &str) -> TokenCheck {
    match status {
        200 => match serde_json::from_str::<HostMe>(body) {
            Ok(me) => TokenCheck::Ok { host: me.host },
            Err(_) => TokenCheck::Unreachable {
                message: "The server's answer wasn't what the host tool expected".into(),
            },
        },
        401 => TokenCheck::Unknown,
        403 => TokenCheck::Revoked,
        // An older server without the route, or the wrong URL.
        404 => TokenCheck::Unreachable {
            message: "This server can't check host tokens yet (it may need updating), or the server URL is wrong".into(),
        },
        _ => TokenCheck::Unreachable {
            message: format!("The server answered {status}"),
        },
    }
}

/// One match in an upload's answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadedMatch {
    pub match_key: Option<String>,
    /// The match's id on the site (`/match?id=`). Not in the upload's answer: the status refresh
    /// (`MatchState`) fills it in.
    #[serde(default)]
    pub match_id: Option<i64>,
    pub line_count: u32,
    /// The region the match is stored in: the upload's for a new match, the stored one's for
    /// another copy. `None` in records from before regions.
    #[serde(default)]
    pub region: Option<String>,
    /// `insert`, `replace`, `repoint` or `skip`.
    pub action: String,
    /// `accepted`, `review`, `rejected` or `void`.
    pub status: String,
    pub rejection: Option<Rejection>,
    #[serde(default)]
    pub review_reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rejection {
    pub code: String,
    #[serde(default)]
    pub message: String,
}

/// What the server answered a stored upload (`200`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadAnswer {
    /// `stored`, `unchanged` or `duplicate`.
    pub result: String,
    /// The region the upload was sent as (its `X-Region`, or the host's home region). `None` in
    /// records from before regions.
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub matches: Vec<UploadedMatch>,
}

/// A match's status now, from `GET /api/host/matches`: changes when an admin accepts, rejects or
/// voids it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchState {
    pub match_key: String,
    /// The match's id on the site (`/match?id=`), which a longer copy keeps. `None` from a server
    /// that doesn't give it yet.
    #[serde(default)]
    pub match_id: Option<i64>,
    pub status: String,
    pub rejection: Option<Rejection>,
    #[serde(default)]
    pub review_reasons: Vec<String>,
}

#[derive(Deserialize)]
struct MatchStates {
    matches: Vec<MatchState>,
}

/// What a `GET /api/host/matches` answer means: the states, or why there are none. A bad token
/// shows up on the next upload, so it's just an error here.
pub fn read_match_states(status: u16, body: &str) -> Result<Vec<MatchState>, String> {
    match status {
        200 => serde_json::from_str::<MatchStates>(body)
            .map(|s| s.matches)
            .map_err(|_| "The server's answer wasn't what the host tool expected".into()),
        _ => Err(format!("The server answered {status}")),
    }
}

/// One tier of the rank tags, as `GET /api/rank-tags` gives it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RankTier {
    /// The tag over the player (`Grandmaster`).
    pub label: String,
    /// RGBA, 0–255.
    pub color: [u8; 4],
    /// The tier's line in the game's guide (`Grandmaster - 1900`).
    pub guide: String,
    /// Display names, raw: not escaped for the Workshop yet.
    pub names: Vec<String>,
}

/// What `GET /api/rank-tags` answers: the rank tiers and who's in each.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RankTiers {
    /// The line under the game's guide header (`Ranks updated 2026-10-03`).
    pub header: String,
    /// When the server worked the tiers out (ISO 8601).
    pub updated_at: String,
    /// The region whose ratings they're from. `None` from a server without regions.
    #[serde(default)]
    pub region: Option<String>,
    /// Lowest tier first.
    pub tiers: Vec<RankTier>,
}

/// What a `GET /api/rank-tags` answer means.
pub fn read_rank_tags(status: u16, body: &str) -> Result<RankTiers, String> {
    match status {
        200 => serde_json::from_str(body)
            .map_err(|_| "The server's rank tags weren't what the host tool expected".into()),
        // An older server without the route, or the wrong URL.
        404 => Err(
            "This server has no rank tags (it may need updating), or the server URL is wrong"
                .into(),
        ),
        _ => Err(format!(
            "The server answered {status} when asked for the rank tags"
        )),
    }
}

/// Asks the server for a region's rank tags (its first region's for `None`). Public: no token.
pub async fn rank_tags(
    server_url: &str,
    region: Option<&str>,
    timeout: Duration,
) -> Result<RankTiers, String> {
    let mut url =
        url::Url::parse(&format!("{server_url}/api/rank-tags")).map_err(|e| e.to_string())?;
    if let Some(region) = region {
        url.query_pairs_mut().append_pair("region", region);
    }
    let response = client(timeout)?
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Couldn't reach the server: {}", e.without_url()))?;
    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .map_err(|e| format!("Couldn't reach the server: {}", e.without_url()))?;
    read_rank_tags(status, &body)
}

/// A player's tier on the leaderboard, as `GET /api/leaderboard` gives it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LeaderboardTier {
    /// RGB, 0–255.
    pub color: [u8; 3],
}

/// One player on the leaderboard.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Ranked {
    /// Their place on the region's leaderboard (`1` is the best).
    pub rank: u32,
    /// Their display name, raw: not escaped for the Workshop yet.
    pub name: String,
    /// Their display rating (the Elo-like number the site shows).
    pub rating: f64,
    /// `None` below the lowest tier.
    pub tier: Option<LeaderboardTier>,
}

/// What `GET /api/leaderboard` answers: the first page, best first.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Leaderboard {
    /// The region whose ratings they are. `None` from a server without regions.
    #[serde(default)]
    pub region: Option<String>,
    pub players: Vec<Ranked>,
}

/// What a `GET /api/leaderboard` answer means.
pub fn read_leaderboard(status: u16, body: &str) -> Result<Leaderboard, String> {
    match status {
        200 => serde_json::from_str(body)
            .map_err(|_| "The server's leaderboard wasn't what the host tool expected".into()),
        404 => Err("This server has no leaderboard, or the server URL is wrong".into()),
        _ => Err(format!(
            "The server answered {status} when asked for the leaderboard"
        )),
    }
}

/// Asks the server for the first page of a region's leaderboard (its first region's for `None`).
/// Public: no token.
pub async fn leaderboard(
    server_url: &str,
    region: Option<&str>,
    timeout: Duration,
) -> Result<Leaderboard, String> {
    let mut url =
        url::Url::parse(&format!("{server_url}/api/leaderboard")).map_err(|e| e.to_string())?;
    if let Some(region) = region {
        url.query_pairs_mut().append_pair("region", region);
    }
    let response = client(timeout)?
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Couldn't reach the server: {}", e.without_url()))?;
    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .map_err(|e| format!("Couldn't reach the server: {}", e.without_url()))?;
    read_leaderboard(status, &body)
}

/// The tourney a lobby is in, as `GET /api/host/tourneys` gives it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LobbyTourney {
    pub id: i64,
    pub name: String,
    pub region: String,
    /// When it starts, ISO 8601 in UTC.
    pub starts_at: String,
    /// `scheduled`, `live`, `done` or `cancelled`.
    pub status: String,
}

/// The values of the game's `TOURNEY - generated` rule for a lobby (GenjiBall-CE
/// `docs/tourney-rule.md`), while its code window is open.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TourneyCodeValues {
    /// The server's id for the lobby: digits, text.
    pub lobby_key: String,
    pub round_limit: u32,
    pub name: String,
    pub label: String,
}

/// A tourney lobby the host is assigned to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TourneyLobby {
    pub id: i64,
    /// `Lobby 1/2`.
    pub label: String,
    /// Its tourney's region: the region its match must be uploaded as.
    pub region: String,
    pub round_limit: u32,
    pub tourney: LobbyTourney,
    /// The match linked to it, whatever its status.
    #[serde(default)]
    pub match_id: Option<i64>,
    /// The verify screenshot's URL on the server, `None` while there's none.
    #[serde(default)]
    pub screenshot: Option<String>,
    #[serde(default)]
    pub screenshot_expired: bool,
    /// An admin checked the screenshot: only an admin can change it now.
    #[serde(default)]
    pub verified: bool,
    /// When the code window opens (ISO 8601), `None` from a server that doesn't say.
    #[serde(default)]
    pub code_from: Option<String>,
    /// The rule's values, only while the code window is open.
    #[serde(default)]
    pub code: Option<TourneyCodeValues>,
}

/// What `GET /api/host/tourneys` answers.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostTourneys {
    /// How long before a start the code window opens, in minutes.
    #[serde(default)]
    pub code_lead_minutes: Option<u64>,
    /// Soonest start first.
    pub lobbies: Vec<TourneyLobby>,
}

/// Why the tourneys couldn't be read.
#[derive(Debug, Clone, PartialEq)]
pub enum TourneysError {
    /// The token doesn't work (`401`) or was revoked (`403`).
    TokenRejected { revoked: bool },
    /// Offline, the server down, an older server, or an answer that isn't one.
    Failed { message: String },
}

/// What a `GET /api/host/tourneys` answer means.
pub fn read_host_tourneys(status: u16, body: &str) -> Result<HostTourneys, TourneysError> {
    let failed = |message: String| TourneysError::Failed { message };
    match status {
        200 => serde_json::from_str(body).map_err(|_| {
            failed("The server's tourney list wasn't what the host tool expected".into())
        }),
        401 => Err(TourneysError::TokenRejected { revoked: false }),
        403 => Err(TourneysError::TokenRejected { revoked: true }),
        // An older server without the route, or the wrong URL.
        404 => Err(failed(
            "This server has no tourneys yet (it may need updating), or the server URL is wrong"
                .into(),
        )),
        _ => Err(failed(api_message(body).unwrap_or_else(|| {
            format!("The server answered {status} when asked for your tourneys")
        }))),
    }
}

/// The tourney lobbies the host with `token` is assigned to, in every region.
pub async fn host_tourneys(
    server_url: &str,
    token: &str,
    timeout: Duration,
) -> Result<HostTourneys, TourneysError> {
    let failed = |e: reqwest::Error| TourneysError::Failed {
        message: format!("Couldn't reach the server: {}", e.without_url()),
    };
    let client = client(timeout).map_err(|message| TourneysError::Failed { message })?;
    let response = client
        .get(format!("{server_url}/api/host/tourneys"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(failed)?;
    let status = response.status().as_u16();
    log::debug!("{server_url} answered {status} to the tourney list");
    match response.text().await {
        Ok(body) => read_host_tourneys(status, &body),
        Err(_) if matches!(status, 401 | 403) => read_host_tourneys(status, ""),
        Err(e) => Err(failed(e)),
    }
}

#[derive(Deserialize)]
struct ScreenshotAnswer {
    lobby: Option<TourneyLobby>,
}

/// What a `PUT` or `DELETE` of `/api/host/lobbies/:id/screenshot` answered: the lobby as it is now
/// (`None` if it stopped being the host's right after), or why it didn't work, in words.
pub fn read_screenshot(status: u16, body: &str) -> Result<Option<TourneyLobby>, String> {
    let error = serde_json::from_str::<ApiError>(body).ok();
    let code = error.as_ref().map(|e| e.error.as_str()).unwrap_or_default();
    let ours = match (status, code) {
        (200, _) => {
            return serde_json::from_str::<ScreenshotAnswer>(body)
                .map(|a| a.lobby)
                .map_err(|_| "The server's answer wasn't what the host tool expected".into())
        }
        (401, _) => "The server doesn't know your host token",
        (403, "not_assigned") => "This lobby isn't assigned to you any more",
        (403, _) => "Your host token was revoked",
        (404, "not_found") => "The server has no such lobby any more",
        (404, _) => "This server can't take verify screenshots yet (it may need updating)",
        (409, "verified") => {
            "An admin already verified this lobby's screenshot: only an admin can change it now"
        }
        (409, "wrong_region") => "The lobby is in another region than the one sent",
        (409, _) => "Something changed on the server meanwhile (the tourney was cancelled, say). Check again and retry",
        (413, _) => "The image is too big for the server",
        (415, _) => "The server only takes PNG, JPEG or WebP images",
        _ => "",
    };
    let message = error
        .map(|e| e.message)
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| format!("The server answered {status}"));
    Err(if ours.is_empty() {
        message
    } else {
        ours.to_string()
    })
}

/// Uploads `image` (of type `content_type`) as lobby `lobby_id`'s verify screenshot, replacing
/// the one there; `None` deletes it.
pub async fn put_screenshot(
    server_url: &str,
    token: &str,
    lobby_id: i64,
    image: Option<(Vec<u8>, &str)>,
    timeout: Duration,
) -> Result<Option<TourneyLobby>, String> {
    let client = client(timeout)?;
    let url = format!("{server_url}/api/host/lobbies/{lobby_id}/screenshot");
    let request = match image {
        Some((bytes, content_type)) => client
            .put(url)
            .header("Content-Type", content_type)
            .body(bytes),
        None => client.delete(url),
    };
    let response = request
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("Couldn't reach the server: {}", e.without_url()))?;
    let status = response.status().as_u16();
    log::debug!("{server_url} answered {status} to the screenshot of lobby {lobby_id}");
    let body = response.text().await.unwrap_or_default();
    read_screenshot(status, &body)
}

/// The `message` of an error answer, if it has one.
fn api_message(body: &str) -> Option<String> {
    serde_json::from_str::<ApiError>(body)
        .ok()
        .map(|e| e.message)
        .filter(|m| !m.is_empty())
}

#[derive(Deserialize)]
struct ApiError {
    error: String,
    #[serde(default)]
    message: String,
}

/// What came of an upload.
#[derive(Debug, Clone, PartialEq)]
pub enum UploadOutcome {
    Stored(UploadAnswer),
    /// The server won't take this file (`413`, `422`): sending it again changes nothing.
    Refused {
        error: String,
        message: String,
    },
    /// The host has no home region and the upload didn't say one (`422 no_region`). Nothing was
    /// stored: send the file again once the host picked a region.
    NoRegion,
    /// The token doesn't work (`401`) or was revoked (`403`). Stop until the host changes it.
    TokenRejected {
        revoked: bool,
    },
    /// Try again later: offline, `409`, `5xx`. `after` is the server's `Retry-After`.
    Retry {
        message: String,
        after: Option<Duration>,
    },
    /// Too many uploads (`429`): no upload to this server until `after` (its `Retry-After`).
    RateLimited {
        message: String,
        after: Option<Duration>,
    },
}

/// What a `POST /api/upload` answer means.
pub fn read_upload(status: u16, retry_after: Option<&str>, body: &str) -> UploadOutcome {
    let error = serde_json::from_str::<ApiError>(body).ok();
    let message = |fallback: String| match &error {
        Some(e) if !e.message.is_empty() => e.message.clone(),
        _ => fallback,
    };
    match status {
        200 => match serde_json::from_str(body) {
            Ok(answer) => UploadOutcome::Stored(answer),
            Err(_) => UploadOutcome::Retry {
                message: "The server's answer wasn't what the host tool expected".into(),
                after: None,
            },
        },
        401 => UploadOutcome::TokenRejected { revoked: false },
        403 => UploadOutcome::TokenRejected { revoked: true },
        422 if error.as_ref().is_some_and(|e| e.error == "no_region") => UploadOutcome::NoRegion,
        413 | 422 => UploadOutcome::Refused {
            error: error
                .as_ref()
                .map_or_else(|| status.to_string(), |e| e.error.clone()),
            message: message(format!("The server refused the file ({status})")),
        },
        _ => {
            let message = message(format!("The server answered {status}"));
            let after = retry_after
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map(Duration::from_secs);
            if status == 429 {
                UploadOutcome::RateLimited { message, after }
            } else {
                UploadOutcome::Retry { message, after }
            }
        }
    }
}

/// What an upload says about its file, besides the file itself: its headers.
#[derive(Debug, Clone, Copy, Default)]
pub struct UploadInfo<'a> {
    /// `X-Log-File`.
    pub file_name: &'a str,
    /// `X-Log-Started-At`: when the file was started.
    pub started_at: Option<&'a str>,
    /// `X-Region`: the region it was hosted in, `None` for the host's home region.
    pub region: Option<&'a str>,
    /// `X-Host-Afk`: the rounds of its matches that started while the host was AFK
    /// (`afk::Afk::header`), `None` for none.
    pub host_afk: Option<&'a str>,
}

impl UploadInfo<'_> {
    /// The headers an upload sends besides the token, in order. The debug panel shows the same.
    pub fn headers(&self) -> Vec<(&'static str, String)> {
        let mut headers = vec![
            ("Content-Type", "text/plain; charset=utf-8".to_string()),
            ("X-Log-File", self.file_name.to_string()),
        ];
        if let Some(started_at) = self.started_at {
            headers.push(("X-Log-Started-At", started_at.to_string()));
        }
        if let Some(region) = self.region {
            headers.push(("X-Region", region.to_string()));
        }
        if let Some(host_afk) = self.host_afk {
            headers.push(("X-Host-Afk", host_afk.to_string()));
        }
        headers
    }
}

/// An answer as the server sent it, for the debug panel: its HTTP status and body, cut to
/// `config::DEBUG_ANSWER_BYTES`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RawAnswer {
    pub status: u16,
    pub body: String,
}

impl RawAnswer {
    fn new(status: u16, body: &str) -> Self {
        let mut end = body.len().min(config::DEBUG_ANSWER_BYTES);
        while !body.is_char_boundary(end) {
            end -= 1;
        }
        Self {
            status,
            body: body[..end].to_string(),
        }
    }
}

/// What came of an upload, and the answer as it came (`None` when there was none).
#[derive(Debug, Clone, PartialEq)]
pub struct Uploaded {
    pub outcome: UploadOutcome,
    pub answer: Option<RawAnswer>,
}

/// Sends a log file, unchanged, to `POST /api/upload`, with the headers in `info`.
pub async fn upload(
    server_url: &str,
    token: &str,
    info: UploadInfo<'_>,
    body: Vec<u8>,
    timeout: Duration,
) -> Uploaded {
    let unanswered = |outcome| Uploaded {
        outcome,
        answer: None,
    };
    let retry = |e: reqwest::Error| {
        unanswered(UploadOutcome::Retry {
            message: format!("Couldn't reach the server: {}", e.without_url()),
            after: None,
        })
    };
    let client = match client(timeout) {
        Ok(client) => client,
        Err(message) => {
            return unanswered(UploadOutcome::Retry {
                message,
                after: None,
            })
        }
    };
    let mut request = client
        .post(format!("{server_url}/api/upload"))
        .bearer_auth(token)
        .body(body);
    for (name, value) in info.headers() {
        request = request.header(name, value);
    }
    let response = match request.send().await {
        Ok(response) => response,
        Err(e) => return retry(e),
    };
    let status = response.status().as_u16();
    log::debug!(
        "{server_url} answered {status} to the upload of {}",
        info.file_name
    );
    let retry_after = response
        .headers()
        .get("Retry-After")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let (outcome, body) = match response.text().await {
        Ok(body) => (read_upload(status, retry_after.as_deref(), &body), body),
        // The status alone says the token or the file won't do: the body only explains it.
        Err(_) if matches!(status, 401 | 403 | 413 | 422) => {
            (read_upload(status, None, ""), String::new())
        }
        Err(e) => return retry(e),
    };
    Uploaded {
        outcome,
        answer: Some(RawAnswer::new(status, &body)),
    }
}

/// The host's lobby as the site lists it, from a heartbeat's answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListedLobby {
    /// The region it's listed in: the heartbeat's `X-Region`, else the host's home region.
    pub region: String,
    pub name: Option<String>,
    pub players: u32,
}

/// What a heartbeat (`PUT /api/host/lobby`) answered when the server took it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LobbyAnswer {
    pub lobby: ListedLobby,
    /// When to send the next heartbeat. `None`: `config::LOBBY_HEARTBEAT_SECS`.
    #[serde(default)]
    pub heartbeat_seconds: Option<u64>,
    /// How long the lobby is listed without one. `None`: `config::LOBBY_TTL_SECS`.
    #[serde(default)]
    pub ttl_seconds: Option<u64>,
}

/// What came of a heartbeat or a close.
#[derive(Debug, Clone, PartialEq)]
pub enum LobbyOutcome {
    /// The heartbeat was taken: the lobby is listed.
    Listed(LobbyAnswer),
    /// The close was taken: the lobby is off the list (or wasn't on it).
    Closed,
    /// The host has no home region and the heartbeat didn't say one (`422 no_region`).
    NoRegion,
    /// The token doesn't work (`401`) or was revoked (`403`): a revoked host's lobby isn't listed.
    TokenRejected { revoked: bool },
    /// Too soon after the last heartbeat or close (`429`): nothing was written. Not an error: send
    /// it after `after` (its `Retry-After`).
    RateLimited { after: Option<Duration> },
    /// Offline, the server down, or an answer that isn't one of the above.
    Failed { message: String },
}

/// What a `PUT` (`closing` false) or `DELETE` (`closing` true) of `/api/host/lobby` answered.
pub fn read_lobby(
    status: u16,
    retry_after: Option<&str>,
    body: &str,
    closing: bool,
) -> LobbyOutcome {
    let unexpected = || LobbyOutcome::Failed {
        message: "The server's answer wasn't what the host tool expected".into(),
    };
    let error = serde_json::from_str::<ApiError>(body).ok();
    match status {
        // `{ "closed": false }` when no lobby was open: off the list all the same.
        200 if closing => match serde_json::from_str::<serde_json::Value>(body) {
            Ok(answer) if answer.get("closed").is_some_and(|c| c.is_boolean()) => {
                LobbyOutcome::Closed
            }
            _ => unexpected(),
        },
        200 => serde_json::from_str(body).map_or_else(|_| unexpected(), LobbyOutcome::Listed),
        401 => LobbyOutcome::TokenRejected { revoked: false },
        403 => LobbyOutcome::TokenRejected { revoked: true },
        422 if error.as_ref().is_some_and(|e| e.error == "no_region") => LobbyOutcome::NoRegion,
        429 => LobbyOutcome::RateLimited {
            after: retry_after
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map(Duration::from_secs),
        },
        // An older server without the route, or the wrong URL.
        404 => LobbyOutcome::Failed {
            message: "This server can't list live lobbies yet (it may need updating), or the server URL is wrong".into(),
        },
        _ => LobbyOutcome::Failed {
            message: match error {
                Some(e) if !e.message.is_empty() => e.message,
                _ => format!("The server answered {status}"),
            },
        },
    }
}

/// Says the host's lobby is open: `PUT /api/host/lobby` with its players and name (`None`: no
/// name, which clears one sent before), as `region` (`None`: the host's home region).
pub async fn lobby_heartbeat(
    server_url: &str,
    token: &str,
    region: Option<&str>,
    players: u32,
    name: Option<&str>,
    timeout: Duration,
) -> LobbyOutcome {
    let client = match client(timeout) {
        Ok(client) => client,
        Err(message) => return LobbyOutcome::Failed { message },
    };
    let mut request = client
        .put(format!("{server_url}/api/host/lobby"))
        .bearer_auth(token)
        .json(&serde_json::json!({ "name": name, "players": players }));
    if let Some(region) = region {
        request = request.header("X-Region", region);
    }
    lobby_request(request, false).await
}

/// Takes the host's lobby off the list: `DELETE /api/host/lobby`.
pub async fn close_lobby(server_url: &str, token: &str, timeout: Duration) -> LobbyOutcome {
    let client = match client(timeout) {
        Ok(client) => client,
        Err(message) => return LobbyOutcome::Failed { message },
    };
    let request = client
        .delete(format!("{server_url}/api/host/lobby"))
        .bearer_auth(token);
    lobby_request(request, true).await
}

async fn lobby_request(request: reqwest::RequestBuilder, closing: bool) -> LobbyOutcome {
    let failed = |e: reqwest::Error| LobbyOutcome::Failed {
        message: format!("Couldn't reach the server: {}", e.without_url()),
    };
    let response = match request.send().await {
        Ok(response) => response,
        Err(e) => return failed(e),
    };
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("Retry-After")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    match response.text().await {
        Ok(body) => read_lobby(status, retry_after.as_deref(), &body, closing),
        // The status alone says the token is turned down, or to wait.
        Err(_) if matches!(status, 401 | 403 | 429) => {
            read_lobby(status, retry_after.as_deref(), "", closing)
        }
        Err(e) => failed(e),
    }
}

/// The HTTP client every request goes through (the ranked server and GitHub). A request taking
/// longer than `timeout` (`config::REQUEST_TIMEOUT_SECS`, as the host set it) fails.
pub fn client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(timeout)
        .user_agent(concat!("genjiball-host-tool/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())
}

/// Asks the server whose token this is.
pub async fn check_token(server_url: &str, token: &str, timeout: Duration) -> TokenCheck {
    let unreachable = |e: reqwest::Error| TokenCheck::Unreachable {
        // Without the URL: it can't hold the token, but keep messages short.
        message: format!("Couldn't reach the server: {}", e.without_url()),
    };
    let client = match client(timeout) {
        Ok(client) => client,
        Err(message) => return TokenCheck::Unreachable { message },
    };
    let response = match client
        .get(format!("{server_url}/api/host/me"))
        .bearer_auth(token)
        .send()
        .await
    {
        Ok(response) => response,
        Err(e) => return unreachable(e),
    };
    let status = response.status().as_u16();
    log::debug!("{server_url} answered {status} to a token check");
    match response.text().await {
        Ok(body) => read_token_check(status, &body),
        // The status alone says the token is turned down.
        Err(_) if matches!(status, 401 | 403) => read_token_check(status, ""),
        Err(e) => unreachable(e),
    }
}

/// Asks the server for the status now of the host's matches with these keys.
pub async fn match_states(
    server_url: &str,
    token: &str,
    keys: &[String],
    timeout: Duration,
) -> Result<Vec<MatchState>, String> {
    let url = url::Url::parse_with_params(
        &format!("{server_url}/api/host/matches"),
        [("keys", keys.join(","))],
    )
    .map_err(|e| e.to_string())?;
    let response = client(timeout)?
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("Couldn't reach the server: {}", e.without_url()))?;
    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .map_err(|e| format!("Couldn't reach the server: {}", e.without_url()))?;
    read_match_states(status, &body)
}

/// Whether the site shows a match with this status: only `accepted` and `void` ones are public.
pub fn is_public(status: &str) -> bool {
    matches!(status, "accepted" | "void")
}

/// The match's page on the site of `server_url`. Only an `http(s)` page under the server: the
/// host's browser opens it.
pub fn match_page_url(server_url: &str, match_id: i64) -> Result<String, String> {
    let mut url = url::Url::parse(server_url).map_err(|_| "That isn't a server URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("The server URL must start with http:// or https://".into());
    }
    url.set_path(&format!(
        "{}{}",
        url.path().trim_end_matches('/'),
        config::MATCH_PAGE_PATH
    ));
    url.query_pairs_mut()
        .clear()
        .append_pair("id", &match_id.to_string());
    url.set_fragment(None);
    Ok(url.into())
}

/// A server on this PC for the tests, at the URL returned: reads one request ending in `body`
/// (the request body), runs `then`, answers with `response` as it is, and hangs up.
#[cfg(test)]
pub fn test_server(response: String, body: &str, then: impl FnOnce() + Send + 'static) -> String {
    test_server_with(response, body, |_| then())
}

/// `test_server`, sending the request it read (head and body) to `seen`.
#[cfg(test)]
pub fn test_server_seeing(
    response: String,
    body: &str,
    seen: std::sync::mpsc::Sender<String>,
) -> String {
    test_server_with(response, body, move |request| {
        let _ = seen.send(request);
    })
}

#[cfg(test)]
fn test_server_with(
    response: String,
    body: &str,
    then: impl FnOnce(String) + Send + 'static,
) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let end = format!("\r\n\r\n{body}");
    std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buf = [0; 4096];
        while !request.ends_with(end.as_bytes()) {
            let n = socket.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            request.extend_from_slice(&buf[..n]);
        }
        then(String::from_utf8_lossy(&request).into_owned());
        socket.write_all(response.as_bytes()).unwrap();
    });
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_host() {
        assert_eq!(
            read_token_check(200, r#"{"host":{"id":3,"name":"Kenzo","trust":"trusted"}}"#),
            TokenCheck::Ok {
                host: Host {
                    id: 3,
                    name: "Kenzo".into(),
                    trust: "trusted".into(),
                    region: None,
                }
            }
        );
    }

    #[test]
    fn reads_unknown_and_revoked_tokens() {
        let unknown = read_token_check(
            401,
            r#"{"error":"unauthorized","message":"Unknown host token"}"#,
        );
        assert_eq!(unknown, TokenCheck::Unknown);
        assert!(unknown.is_rejected());
        assert_eq!(
            read_token_check(403, r#"{"error":"revoked"}"#),
            TokenCheck::Revoked
        );
    }

    #[test]
    fn anything_else_is_unreachable_not_rejected() {
        for (status, body) in [
            (200, "<html>"),
            (404, r#"{"error":"not_found"}"#),
            (500, ""),
            (502, "Bad gateway"),
        ] {
            let check = read_token_check(status, body);
            assert!(
                matches!(check, TokenCheck::Unreachable { .. }),
                "{status}: {check:?}"
            );
            assert!(!check.is_rejected());
        }
    }

    #[test]
    fn an_upload_sends_only_the_headers_it_has() {
        let names = |info: UploadInfo| -> Vec<&str> {
            info.headers().into_iter().map(|(name, _)| name).collect()
        };
        let plain = UploadInfo {
            file_name: "Log-a.txt",
            ..UploadInfo::default()
        };
        assert_eq!(names(plain), ["Content-Type", "X-Log-File"]);
        let full = UploadInfo {
            started_at: Some("2026-10-02T20:15:33+02:00"),
            region: Some("eu"),
            host_afk: Some("482913507226:3"),
            ..plain
        };
        assert_eq!(
            names(full),
            [
                "Content-Type",
                "X-Log-File",
                "X-Log-Started-At",
                "X-Region",
                "X-Host-Afk"
            ]
        );
    }

    #[test]
    fn a_long_answer_is_cut_for_the_debug_panel() {
        let long = "é".repeat(config::DEBUG_ANSWER_BYTES);
        let answer = RawAnswer::new(500, &long);
        assert!(answer.body.len() <= config::DEBUG_ANSWER_BYTES);
        assert!(long.starts_with(&answer.body));
        assert_eq!(RawAnswer::new(200, "{}").body, "{}");
    }

    #[test]
    fn reads_a_stored_upload() {
        let body = r#"{"result":"stored","uploadId":12,"region":"eu","matches":[{"matchKey":"482913507226","lineCount":57,"region":"eu","action":"insert","status":"review","rejection":null,"reviewReasons":["duplicate_name"]}]}"#;
        assert_eq!(
            read_upload(200, None, body),
            UploadOutcome::Stored(UploadAnswer {
                result: "stored".into(),
                region: Some("eu".into()),
                matches: vec![UploadedMatch {
                    match_key: Some("482913507226".into()),
                    match_id: None,
                    line_count: 57,
                    region: Some("eu".into()),
                    action: "insert".into(),
                    status: "review".into(),
                    rejection: None,
                    review_reasons: vec!["duplicate_name".into()],
                }],
            })
        );
        let rejected = r#"{"result":"stored","uploadId":13,"matches":[{"matchKey":"1","lineCount":9,"action":"insert","status":"rejected","rejection":{"code":"unranked","message":"MAP"},"reviewReasons":[]}]}"#;
        let UploadOutcome::Stored(answer) = read_upload(200, None, rejected) else {
            panic!()
        };
        assert_eq!(
            answer.matches[0].rejection.as_ref().unwrap().code,
            "unranked"
        );
        assert_eq!(
            read_upload(
                200,
                None,
                r#"{"result":"duplicate","uploadId":null,"matches":[]}"#
            ),
            UploadOutcome::Stored(UploadAnswer {
                result: "duplicate".into(),
                region: None,
                matches: vec![]
            })
        );
    }

    #[test]
    fn stops_on_a_bad_token() {
        assert_eq!(
            read_upload(401, None, r#"{"error":"unauthorized","message":"x"}"#),
            UploadOutcome::TokenRejected { revoked: false }
        );
        assert_eq!(
            read_upload(403, None, r#"{"error":"revoked","message":"x"}"#),
            UploadOutcome::TokenRejected { revoked: true }
        );
    }

    #[test]
    fn doesnt_retry_a_refused_file() {
        assert_eq!(
            read_upload(
                422,
                None,
                r#"{"error":"not_ranked","message":"No GBR line"}"#
            ),
            UploadOutcome::Refused {
                error: "not_ranked".into(),
                message: "No GBR line".into()
            }
        );
        assert!(matches!(
            read_upload(413, None, "too big"),
            UploadOutcome::Refused { error, .. } if error == "413"
        ));
    }

    #[test]
    fn asks_for_a_region_rather_than_refusing_the_file() {
        // Nothing was stored, and the file is fine: it goes once the host picked a region.
        assert_eq!(
            read_upload(
                422,
                None,
                r#"{"error":"no_region","message":"This host has no home region"}"#
            ),
            UploadOutcome::NoRegion
        );
    }

    #[test]
    fn reads_the_home_region() {
        let check = read_token_check(
            200,
            r#"{"host":{"id":3,"name":"Kenzo","trust":"trusted","region":"na"}}"#,
        );
        let TokenCheck::Ok { host } = check else {
            panic!("{check:?}")
        };
        assert_eq!(host.region.as_deref(), Some("na"));
        let none = read_token_check(
            200,
            r#"{"host":{"id":3,"name":"Kenzo","trust":"trusted","region":null}}"#,
        );
        assert!(matches!(none, TokenCheck::Ok { host } if host.region.is_none()));
    }

    #[test]
    fn sends_the_region() {
        use tauri::async_runtime::block_on as run;
        let timeout = Duration::from_secs(config::REQUEST_TIMEOUT_SECS.default);
        let (seen, request) = std::sync::mpsc::channel();
        let body = r#"{"result":"duplicate","uploadId":1,"region":"na","matches":[]}"#;
        let url = test_server_seeing(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ),
            "x",
            seen,
        );
        let outcome = run(upload(
            &url,
            "t",
            UploadInfo {
                file_name: "Log-a.txt",
                region: Some("na"),
                ..UploadInfo::default()
            },
            b"x".to_vec(),
            timeout,
        ))
        .outcome;
        assert!(matches!(outcome, UploadOutcome::Stored(a) if a.region.as_deref() == Some("na")));
        let request = request.recv().unwrap().to_ascii_lowercase();
        assert!(request.contains("\r\nx-region: na\r\n"), "{request}");
        assert!(!request.contains("x-host-afk"), "{request}");
    }

    #[test]
    fn sends_the_hosts_afk_rounds() {
        use tauri::async_runtime::block_on as run;
        let timeout = Duration::from_secs(config::REQUEST_TIMEOUT_SECS.default);
        let (seen, request) = std::sync::mpsc::channel();
        let body = r#"{"result":"stored","matches":[]}"#;
        let url = test_server_seeing(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ),
            "x",
            seen,
        );
        run(upload(
            &url,
            "t",
            UploadInfo {
                file_name: "Log-a.txt",
                host_afk: Some("482913507226:3,4,5"),
                ..UploadInfo::default()
            },
            b"x".to_vec(),
            timeout,
        ));
        let request = request.recv().unwrap().to_ascii_lowercase();
        assert!(
            request.contains("\r\nx-host-afk: 482913507226:3,4,5\r\n"),
            "{request}"
        );
    }

    #[test]
    fn retries_everything_else() {
        assert_eq!(
            read_upload(
                429,
                Some("600"),
                r#"{"error":"rate_limited","message":"At most 60 uploads an hour"}"#
            ),
            UploadOutcome::RateLimited {
                message: "At most 60 uploads an hour".into(),
                after: Some(Duration::from_secs(600))
            }
        );
        // As long as the server says, even past the longest backoff.
        assert_eq!(
            read_upload(503, Some("31536000"), ""),
            UploadOutcome::Retry {
                message: "The server answered 503".into(),
                after: Some(Duration::from_secs(31_536_000))
            }
        );
        assert_eq!(
            read_upload(429, None, ""),
            UploadOutcome::RateLimited {
                message: "The server answered 429".into(),
                after: None
            }
        );
        for (status, body) in [
            (409, r#"{"error":"conflict"}"#),
            (500, ""),
            (502, "<html>"),
            (200, "<html>"),
        ] {
            assert!(
                matches!(
                    read_upload(status, None, body),
                    UploadOutcome::Retry { after: None, .. }
                ),
                "{status}"
            );
        }
    }

    #[test]
    fn reads_match_states() {
        let body = r#"{"matches":[{"matchKey":"482913507226","status":"accepted","rejection":null,"reviewReasons":["untrusted_host"]}]}"#;
        assert_eq!(
            read_match_states(200, body).unwrap(),
            [MatchState {
                match_key: "482913507226".into(),
                match_id: None,
                status: "accepted".into(),
                rejection: None,
                review_reasons: vec!["untrusted_host".into()],
            }]
        );
        // The match's id on the site, from a server that gives it (genjiball-ranked `host-match-id`).
        let with_id = r#"{"matches":[{"matchKey":"1","matchId":812,"status":"accepted","rejection":null,"reviewReasons":[]}]}"#;
        assert_eq!(
            read_match_states(200, with_id).unwrap()[0].match_id,
            Some(812)
        );
        assert!(read_match_states(200, "<html>").is_err());
        // An older server without the route.
        assert!(read_match_states(404, r#"{"error":"not_found"}"#).is_err());
    }

    #[test]
    fn reads_rank_tags() {
        let body = r#"{"header":"Ranks updated 2026-10-03","updatedAt":"2026-10-03T12:00:00Z","region":"na","tiers":[{"label":"Apprentice","color":[205,127,50,255],"guide":"Apprentice - 1300","names":["Kenzo"]},{"label":"Master","color":[255,215,0,255],"guide":"Master - 1600","names":[]}]}"#;
        assert_eq!(
            read_rank_tags(200, body).unwrap(),
            RankTiers {
                header: "Ranks updated 2026-10-03".into(),
                updated_at: "2026-10-03T12:00:00Z".into(),
                region: Some("na".into()),
                tiers: vec![
                    RankTier {
                        label: "Apprentice".into(),
                        color: [205, 127, 50, 255],
                        guide: "Apprentice - 1300".into(),
                        names: vec!["Kenzo".into()],
                    },
                    RankTier {
                        label: "Master".into(),
                        color: [255, 215, 0, 255],
                        guide: "Master - 1600".into(),
                        names: vec![],
                    },
                ],
            }
        );
    }

    #[test]
    fn rank_tags_errors_say_what_went_wrong() {
        assert!(read_rank_tags(200, "<html>").is_err());
        // A colour out of range isn't a colour.
        assert!(read_rank_tags(
            200,
            r#"{"header":"","updatedAt":"","tiers":[{"label":"A","color":[256,0,0,255],"guide":"A","names":[]}]}"#
        )
        .is_err());
        assert!(read_rank_tags(404, r#"{"error":"not_found"}"#)
            .unwrap_err()
            .contains("no rank tags"));
        assert!(read_rank_tags(503, "").unwrap_err().contains("503"));
    }

    #[test]
    fn reads_the_leaderboard() {
        let body = r#"{"region":"na","page":1,"pageSize":50,"hasMore":false,"players":[{"rank":1,"id":7,"name":"Kenzo","rating":2143,"rounds":90,"wins":30,"lastPlayedAt":"2026-10-03T12:00:00Z","tier":{"label":"Champion","color":[150,0,0],"threshold":2100},"inactiveSince":null},{"rank":2,"id":8,"name":"Hana","rating":1010,"rounds":3,"wins":0,"lastPlayedAt":"2026-10-03T12:00:00Z","tier":null,"inactiveSince":null}]}"#;
        assert_eq!(
            read_leaderboard(200, body).unwrap(),
            Leaderboard {
                region: Some("na".into()),
                players: vec![
                    Ranked {
                        rank: 1,
                        name: "Kenzo".into(),
                        rating: 2143.0,
                        tier: Some(LeaderboardTier { color: [150, 0, 0] }),
                    },
                    Ranked {
                        rank: 2,
                        name: "Hana".into(),
                        rating: 1010.0,
                        tier: None,
                    },
                ],
            }
        );
    }

    #[test]
    fn leaderboard_errors_say_what_went_wrong() {
        assert!(read_leaderboard(200, "<html>").is_err());
        assert!(read_leaderboard(404, r#"{"error":"not_found"}"#)
            .unwrap_err()
            .contains("no leaderboard"));
        assert!(read_leaderboard(503, "").unwrap_err().contains("503"));
    }

    #[test]
    fn an_upload_answer_has_no_match_id() {
        // The status refresh brings it later.
        let body = r#"{"result":"stored","uploadId":12,"matches":[{"matchKey":"1","lineCount":9,"action":"insert","status":"accepted","rejection":null,"reviewReasons":[]}]}"#;
        let UploadOutcome::Stored(answer) = read_upload(200, None, body) else {
            panic!()
        };
        assert_eq!(answer.matches[0].match_id, None);
        // Records written before the id was known still load.
        let old: UploadedMatch = serde_json::from_str(
            r#"{"matchKey":"1","lineCount":9,"action":"insert","status":"review","rejection":null}"#,
        )
        .unwrap();
        assert_eq!(old.match_id, None);
    }

    #[test]
    fn only_accepted_and_void_matches_are_public() {
        assert!(is_public("accepted"));
        assert!(is_public("void"));
        assert!(!is_public("review"));
        assert!(!is_public("rejected"));
    }

    #[test]
    fn builds_the_match_page_url() {
        assert_eq!(
            match_page_url("https://genjiball.us", 12).unwrap(),
            "https://genjiball.us/match?id=12"
        );
        assert_eq!(
            match_page_url("http://127.0.0.1:8787", 3).unwrap(),
            "http://127.0.0.1:8787/match?id=3"
        );
        // A server under a path keeps it.
        assert_eq!(
            match_page_url("https://example.com/ranked/", 3).unwrap(),
            "https://example.com/ranked/match?id=3"
        );
        assert!(match_page_url("file:///C:/Windows", 3).is_err());
        assert!(match_page_url("not a url", 3).is_err());
    }

    /// A server that answers with `head` (status line and headers), then hangs up before the body
    /// it announced.
    fn cut_off_server(head: &str, body: &str) -> String {
        test_server(
            format!("{head}Content-Length: 100\r\n\r\n{{\"error\":"),
            body,
            || {},
        )
    }

    #[test]
    fn a_cut_off_answer_still_turns_a_token_down() {
        use tauri::async_runtime::block_on as run;
        let timeout = Duration::from_secs(config::REQUEST_TIMEOUT_SECS.default);
        let revoked = cut_off_server("HTTP/1.1 403 Forbidden\r\n", "");
        assert_eq!(
            run(check_token(&revoked, "t", timeout)),
            TokenCheck::Revoked
        );
        let unknown = cut_off_server("HTTP/1.1 401 Unauthorized\r\n", "x");
        assert_eq!(
            run(upload(
                &unknown,
                "t",
                UploadInfo {
                    file_name: "Log-a.txt",
                    ..UploadInfo::default()
                },
                b"x".to_vec(),
                timeout
            ))
            .outcome,
            UploadOutcome::TokenRejected { revoked: false }
        );
        let too_big = cut_off_server("HTTP/1.1 413 Payload Too Large\r\n", "x");
        assert!(matches!(
            run(upload(
                &too_big,
                "t",
                UploadInfo {
                    file_name: "Log-a.txt",
                    ..UploadInfo::default()
                },
                b"x".to_vec(),
                timeout
            ))
            .outcome,
            UploadOutcome::Refused { .. }
        ));
        // Any other answer cut off is worth another try.
        let failing = cut_off_server("HTTP/1.1 500 Internal Server Error\r\n", "x");
        assert!(matches!(
            run(upload(
                &failing,
                "t",
                UploadInfo {
                    file_name: "Log-a.txt",
                    ..UploadInfo::default()
                },
                b"x".to_vec(),
                timeout
            ))
            .outcome,
            UploadOutcome::Retry { .. }
        ));
    }

    #[test]
    fn reads_a_listed_lobby() {
        let body = r#"{"lobby":{"region":"eu","name":"Kenzo's ranked","players":6,"openedAt":"2026-10-05T20:00:00Z","seenAt":"2026-10-05T20:14:00Z"},"heartbeatSeconds":60,"ttlSeconds":180}"#;
        assert_eq!(
            read_lobby(200, None, body, false),
            LobbyOutcome::Listed(LobbyAnswer {
                lobby: ListedLobby {
                    region: "eu".into(),
                    name: Some("Kenzo's ranked".into()),
                    players: 6,
                },
                heartbeat_seconds: Some(60),
                ttl_seconds: Some(180),
            })
        );
        let unnamed = r#"{"lobby":{"region":"na","name":null,"players":0}}"#;
        assert!(matches!(
            read_lobby(200, None, unnamed, false),
            LobbyOutcome::Listed(LobbyAnswer { lobby, heartbeat_seconds: None, ttl_seconds: None })
                if lobby.name.is_none() && lobby.region == "na"
        ));
        assert!(matches!(
            read_lobby(200, None, "<html>", false),
            LobbyOutcome::Failed { .. }
        ));
    }

    #[test]
    fn reads_a_close() {
        for body in [r#"{"closed":true}"#, r#"{"closed":false}"#] {
            assert_eq!(read_lobby(200, None, body, true), LobbyOutcome::Closed);
        }
        assert!(matches!(
            read_lobby(200, None, "{}", true),
            LobbyOutcome::Failed { .. }
        ));
    }

    #[test]
    fn a_lobby_too_soon_waits_and_a_bad_token_stops() {
        let limited =
            r#"{"error":"rate_limited","message":"At most one heartbeat every 30 seconds"}"#;
        assert_eq!(
            read_lobby(429, Some("30"), limited, false),
            LobbyOutcome::RateLimited {
                after: Some(Duration::from_secs(30))
            }
        );
        assert_eq!(
            read_lobby(429, None, "", false),
            LobbyOutcome::RateLimited { after: None }
        );
        assert_eq!(
            read_lobby(401, None, r#"{"error":"unauthorized"}"#, true),
            LobbyOutcome::TokenRejected { revoked: false }
        );
        assert_eq!(
            read_lobby(403, None, r#"{"error":"revoked"}"#, false),
            LobbyOutcome::TokenRejected { revoked: true }
        );
        assert_eq!(
            read_lobby(422, None, r#"{"error":"no_region","message":"x"}"#, false),
            LobbyOutcome::NoRegion
        );
        assert_eq!(
            read_lobby(
                400,
                None,
                r#"{"error":"bad_request","message":"name is longer than 64 characters"}"#,
                false
            ),
            LobbyOutcome::Failed {
                message: "name is longer than 64 characters".into()
            }
        );
        for status in [404, 500, 503] {
            assert!(
                matches!(
                    read_lobby(status, None, "", false),
                    LobbyOutcome::Failed { .. }
                ),
                "{status}"
            );
        }
    }

    #[test]
    fn sends_a_heartbeat_and_a_close() {
        use tauri::async_runtime::block_on as run;
        let timeout = Duration::from_secs(config::REQUEST_TIMEOUT_SECS.default);
        let (seen, request) = std::sync::mpsc::channel();
        let answer = r#"{"lobby":{"region":"na","name":"Late night","players":7},"heartbeatSeconds":60,"ttlSeconds":180}"#;
        let sent = r#"{"name":"Late night","players":7}"#;
        let url = test_server_seeing(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{answer}",
                answer.len()
            ),
            sent,
            seen.clone(),
        );
        let outcome = run(lobby_heartbeat(
            &url,
            "t",
            Some("na"),
            7,
            Some("Late night"),
            timeout,
        ));
        assert!(matches!(outcome, LobbyOutcome::Listed(a) if a.lobby.players == 7));
        let head = request.recv().unwrap();
        assert!(head.starts_with("PUT /api/host/lobby "), "{head}");
        let lower = head.to_ascii_lowercase();
        assert!(lower.contains("\r\nx-region: na\r\n"), "{head}");
        assert!(lower.contains("\r\nauthorization: bearer t\r\n"), "{head}");
        assert!(head.ends_with(sent), "{head}");

        // The home region: no `X-Region`; no name: `null`, which clears one sent before.
        let unnamed = r#"{"name":null,"players":0}"#;
        let url = test_server_seeing(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{answer}",
                answer.len()
            ),
            unnamed,
            seen.clone(),
        );
        run(lobby_heartbeat(&url, "t", None, 0, None, timeout));
        let head = request.recv().unwrap();
        assert!(!head.to_ascii_lowercase().contains("x-region"), "{head}");
        assert!(head.ends_with(unnamed), "{head}");

        let closed = r#"{"closed":true}"#;
        let url = test_server_seeing(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{closed}",
                closed.len()
            ),
            "",
            seen,
        );
        assert_eq!(run(close_lobby(&url, "t", timeout)), LobbyOutcome::Closed);
        assert!(request
            .recv()
            .unwrap()
            .starts_with("DELETE /api/host/lobby "));
    }

    /// `GET /api/host/tourneys`'s example answer (genjiball-ranked `docs/api.md`).
    const TOURNEYS: &str = r#"{"region":null,"codeLeadMinutes":60,"lobbies":[{"id":7,"label":"Lobby 1/2","region":"eu","roundLimit":30,"tourney":{"id":3,"name":"October Cup","region":"eu","startsAt":"2026-10-10T17:00:00Z","status":"scheduled"},"matchId":null,"screenshot":null,"screenshotExpired":false,"verified":false,"codeFrom":"2026-10-10T16:00:00Z","code":{"lobbyKey":"482913507226","roundLimit":30,"name":"October Cup","label":"Lobby 1/2"}}]}"#;

    #[test]
    fn reads_the_hosts_tourneys() {
        let found = read_host_tourneys(200, TOURNEYS).unwrap();
        assert_eq!(found.code_lead_minutes, Some(60));
        let lobby = &found.lobbies[0];
        assert_eq!((lobby.id, lobby.label.as_str()), (7, "Lobby 1/2"));
        assert_eq!(lobby.tourney.starts_at, "2026-10-10T17:00:00Z");
        assert_eq!(
            lobby.code,
            Some(TourneyCodeValues {
                lobby_key: "482913507226".into(),
                round_limit: 30,
                name: "October Cup".into(),
                label: "Lobby 1/2".into(),
            })
        );
        // Outside the window: no code.
        let closed = TOURNEYS.replace(
            r#""code":{"lobbyKey":"482913507226","roundLimit":30,"name":"October Cup","label":"Lobby 1/2"}"#,
            r#""code":null"#,
        );
        assert_eq!(
            read_host_tourneys(200, &closed).unwrap().lobbies[0].code,
            None
        );
        assert!(read_host_tourneys(200, r#"{"lobbies":[]}"#)
            .unwrap()
            .lobbies
            .is_empty());
    }

    #[test]
    fn tourney_list_errors_say_what_went_wrong() {
        assert_eq!(
            read_host_tourneys(401, ""),
            Err(TourneysError::TokenRejected { revoked: false })
        );
        assert_eq!(
            read_host_tourneys(403, r#"{"error":"revoked"}"#),
            Err(TourneysError::TokenRejected { revoked: true })
        );
        for (status, body) in [(200, "<html>"), (404, ""), (500, "")] {
            assert!(
                matches!(
                    read_host_tourneys(status, body),
                    Err(TourneysError::Failed { .. })
                ),
                "{status}"
            );
        }
    }

    #[test]
    fn reads_a_screenshot_answer() {
        let lobby = TOURNEYS
            .split_once(r#""lobbies":["#)
            .unwrap()
            .1
            .trim_end_matches("]}");
        let answer = format!(r#"{{"lobby":{lobby}}}"#);
        assert_eq!(read_screenshot(200, &answer).unwrap().unwrap().id, 7);
        assert_eq!(read_screenshot(200, r#"{"lobby":null}"#), Ok(None));
        let verified = read_screenshot(409, r#"{"error":"verified","message":"x"}"#);
        assert!(verified.unwrap_err().contains("verified"));
        assert!(read_screenshot(403, r#"{"error":"not_assigned"}"#)
            .unwrap_err()
            .contains("isn't assigned"));
        assert!(read_screenshot(415, "").unwrap_err().contains("PNG"));
        // Anything else: the server's own words.
        assert_eq!(
            read_screenshot(400, r#"{"error":"empty","message":"No image"}"#),
            Err("No image".into())
        );
    }

    #[test]
    fn sends_a_screenshot() {
        use tauri::async_runtime::block_on as run;
        let timeout = Duration::from_secs(config::REQUEST_TIMEOUT_SECS.default);
        let (seen, request) = std::sync::mpsc::channel();
        let answer = r#"{"lobby":null}"#;
        let url = test_server_seeing(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{answer}",
                answer.len()
            ),
            "PNGDATA",
            seen,
        );
        let image = Some((b"PNGDATA".to_vec(), "image/png"));
        assert_eq!(run(put_screenshot(&url, "t", 7, image, timeout)), Ok(None));
        let head = request.recv().unwrap();
        assert!(
            head.starts_with("PUT /api/host/lobbies/7/screenshot "),
            "{head}"
        );
        let lower = head.to_ascii_lowercase();
        assert!(lower.contains("\r\ncontent-type: image/png\r\n"), "{head}");
        assert!(lower.contains("\r\nauthorization: bearer t\r\n"), "{head}");
    }

    #[test]
    fn serializes_for_the_frontend() {
        let json = serde_json::to_value(TokenCheck::Unreachable {
            message: "x".into(),
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "result": "unreachable", "message": "x" })
        );
        assert_eq!(
            serde_json::to_value(TokenCheck::Revoked).unwrap(),
            serde_json::json!({ "result": "revoked" })
        );
    }
}
