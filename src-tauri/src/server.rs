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
    /// The match's id on the site (`/match?id=`), once the server gives it.
    #[serde(default)]
    pub match_id: Option<i64>,
    pub line_count: u32,
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
    #[serde(default)]
    pub matches: Vec<UploadedMatch>,
}

/// A match's status now, from `GET /api/host/matches`: changes when an admin accepts, rejects or
/// voids it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchState {
    pub match_key: String,
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
    /// The token doesn't work (`401`) or was revoked (`403`). Stop until the host changes it.
    TokenRejected {
        revoked: bool,
    },
    /// Try again later: offline, `409`, `429`, `5xx`. `after` is the server's `Retry-After`.
    Retry {
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
        413 | 422 => UploadOutcome::Refused {
            error: error
                .as_ref()
                .map_or_else(|| status.to_string(), |e| e.error.clone()),
            message: message(format!("The server refused the file ({status})")),
        },
        _ => UploadOutcome::Retry {
            message: message(format!("The server answered {status}")),
            after: retry_after
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map(|secs| Duration::from_secs(secs.min(config::RETRY_MAX_SECS))),
        },
    }
}

/// Sends a log file, unchanged, to `POST /api/upload`.
pub async fn upload(
    server_url: &str,
    token: &str,
    file_name: &str,
    started_at: Option<&str>,
    body: Vec<u8>,
) -> UploadOutcome {
    let retry = |e: reqwest::Error| UploadOutcome::Retry {
        message: format!("Couldn't reach the server: {}", e.without_url()),
        after: None,
    };
    let client = match client() {
        Ok(client) => client,
        Err(message) => {
            return UploadOutcome::Retry {
                message,
                after: None,
            }
        }
    };
    let mut request = client
        .post(format!("{server_url}/api/upload"))
        .bearer_auth(token)
        .header("Content-Type", "text/plain; charset=utf-8")
        .header("X-Log-File", file_name)
        .body(body);
    if let Some(started_at) = started_at {
        request = request.header("X-Log-Started-At", started_at);
    }
    let response = match request.send().await {
        Ok(response) => response,
        Err(e) => return retry(e),
    };
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("Retry-After")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    match response.text().await {
        Ok(body) => read_upload(status, retry_after.as_deref(), &body),
        Err(e) => retry(e),
    }
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(config::REQUEST_TIMEOUT_SECS))
        .user_agent(concat!("genjiball-host-tool/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())
}

/// Asks the server whose token this is.
pub async fn check_token(server_url: &str, token: &str) -> TokenCheck {
    let unreachable = |e: reqwest::Error| TokenCheck::Unreachable {
        // Without the URL: it can't hold the token, but keep messages short.
        message: format!("Couldn't reach the server: {}", e.without_url()),
    };
    let client = match client() {
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
    match response.text().await {
        Ok(body) => read_token_check(status, &body),
        Err(e) => unreachable(e),
    }
}

/// Asks the server for the status now of the host's matches with these keys.
pub async fn match_states(
    server_url: &str,
    token: &str,
    keys: &[String],
) -> Result<Vec<MatchState>, String> {
    let url = url::Url::parse_with_params(
        &format!("{server_url}/api/host/matches"),
        [("keys", keys.join(","))],
    )
    .map_err(|e| e.to_string())?;
    let response = client()?
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
        return Err("The server URL starts with https://".into());
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
                    trust: "trusted".into()
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
    fn reads_a_stored_upload() {
        let body = r#"{"result":"stored","uploadId":12,"matches":[{"matchKey":"482913507226","lineCount":57,"action":"insert","status":"review","rejection":null,"reviewReasons":["duplicate_name"]}]}"#;
        assert_eq!(
            read_upload(200, None, body),
            UploadOutcome::Stored(UploadAnswer {
                result: "stored".into(),
                matches: vec![UploadedMatch {
                    match_key: Some("482913507226".into()),
                    match_id: None,
                    line_count: 57,
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
    fn retries_everything_else() {
        assert_eq!(
            read_upload(
                429,
                Some("600"),
                r#"{"error":"rate_limited","message":"At most 60 uploads an hour"}"#
            ),
            UploadOutcome::Retry {
                message: "At most 60 uploads an hour".into(),
                after: Some(Duration::from_secs(600))
            }
        );
        // No longer than the longest backoff, however far off the server says.
        assert_eq!(
            read_upload(503, Some("31536000"), ""),
            UploadOutcome::Retry {
                message: "The server answered 503".into(),
                after: Some(Duration::from_secs(config::RETRY_MAX_SECS))
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
        // A server that gives the match's id on the site.
        let with_id = r#"{"matches":[{"matchKey":"1","matchId":12,"status":"accepted","rejection":null,"reviewReasons":[]}]}"#;
        assert_eq!(
            read_match_states(200, with_id).unwrap()[0].match_id,
            Some(12)
        );
        assert!(read_match_states(200, "<html>").is_err());
        // An older server without the route.
        assert!(read_match_states(404, r#"{"error":"not_found"}"#).is_err());
    }

    #[test]
    fn reads_the_match_id_when_the_server_gives_it() {
        let body = r#"{"result":"stored","uploadId":12,"matches":[{"matchKey":"1","matchId":40,"lineCount":9,"action":"insert","status":"accepted","rejection":null,"reviewReasons":[]}]}"#;
        let UploadOutcome::Stored(answer) = read_upload(200, None, body) else {
            panic!()
        };
        assert_eq!(answer.matches[0].match_id, Some(40));
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
