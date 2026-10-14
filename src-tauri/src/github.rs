//! GitHub's releases API (`config::GITHUB_API_URL`): the ranked code's base release
//! (`release.rs`) and the pre-release update channel (`updates.rs`). Public, so no token.

use std::time::Duration;

use serde::Deserialize;

use crate::{config, server};

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Asset {
    #[serde(default)]
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub updated_at: String,
    pub browser_download_url: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Release {
    #[serde(default)]
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

/// Without a token GitHub allows 60 requests an hour, then answers `403` or `429`.
pub fn rate_limited(status: u16) -> bool {
    matches!(status, 403 | 429)
}

pub const RATE_LIMITED: &str = "GitHub is limiting requests from this PC. Try again in a while";

/// What a `GET /repos/{repo}/releases` answer means: `repo`'s releases, newest first.
pub fn read_releases(status: u16, body: &str, repo: &str) -> Result<Vec<Release>, String> {
    match status {
        200 => serde_json::from_str(body)
            .map_err(|_| "GitHub's list of releases wasn't what the host tool expected".into()),
        _ if rate_limited(status) => Err(RATE_LIMITED.into()),
        404 => Err(format!("GitHub has no repo {repo}")),
        _ => Err(format!("GitHub answered {status} when asked for releases")),
    }
}

/// `GET /repos/{repo}/releases`, the newest `per_page` first.
pub fn releases_url(repo: &str, per_page: u32) -> Result<url::Url, String> {
    url::Url::parse_with_params(
        &format!("{}/repos/{repo}/releases", config::GITHUB_API_URL),
        [("per_page", per_page.to_string())],
    )
    .map_err(|e| e.to_string())
}

/// `GET /repos/{repo}/releases/tags/{tag}`.
pub fn tagged_url(repo: &str, tag: &str) -> Result<url::Url, String> {
    let mut url = url::Url::parse(&format!("{}/repos/{repo}/releases", config::GITHUB_API_URL))
        .map_err(|e| e.to_string())?;
    url.path_segments_mut()
        .map_err(|()| "GitHub's API URL can't take a path".to_string())?
        .extend(["tags", tag]);
    Ok(url)
}

pub fn unreachable(e: reqwest::Error) -> String {
    format!("Couldn't reach GitHub: {}", e.without_url())
}

/// Asks GitHub's API: the answer's status and body.
pub async fn get(url: url::Url, timeout: Duration) -> Result<(u16, String), String> {
    let response = server::client(timeout)?
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(unreachable)?;
    let status = response.status().as_u16();
    log::debug!("GitHub answered {status}");
    let body = response.text().await.map_err(unreachable)?;
    Ok((status, body))
}
