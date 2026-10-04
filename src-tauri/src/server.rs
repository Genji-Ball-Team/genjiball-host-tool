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
