//! The base of the ranked code: the Workshop code in the latest ranked (`R`) GenjiBall-CE
//! release on GitHub. Kept in memory for `RELEASE_CACHE_SECS`, and the code per tag, so a click
//! on "Copy ranked code" rarely asks GitHub.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::{config, server};

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

/// The release the ranked code is built from.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub tag: String,
    pub asset_url: String,
}

/// The newest published ranked release in a `GET /repos/{repo}/releases` answer (newest first),
/// and its Workshop code asset.
pub fn pick(status: u16, body: &str) -> Result<Release, String> {
    match status {
        200 => {}
        // Without a token GitHub allows 60 requests an hour, then answers 403 or 429.
        403 | 429 => {
            return Err("GitHub is limiting requests from this PC. Try again in a while".into())
        }
        404 => return Err(format!("GitHub has no repo {}", config::RELEASE_REPO)),
        _ => return Err(format!("GitHub answered {status} when asked for releases")),
    }
    let releases: Vec<GithubRelease> = serde_json::from_str(body)
        .map_err(|_| "GitHub's list of releases wasn't what the host tool expected")?;
    let release = releases
        .into_iter()
        .find(|r| !r.draft && !r.prerelease && r.tag_name.ends_with(config::RELEASE_TAG_SUFFIX))
        .ok_or(format!(
            "No ranked ({}) release of {} on GitHub yet",
            config::RELEASE_TAG_SUFFIX,
            config::RELEASE_REPO
        ))?;
    let asset = release
        .assets
        .into_iter()
        .find(|a| {
            a.name.starts_with(config::RELEASE_ASSET_PREFIX)
                && a.name.ends_with(config::RELEASE_ASSET_SUFFIX)
        })
        .ok_or(format!(
            "Release {} has no Workshop code file ({}*{})",
            release.tag_name,
            config::RELEASE_ASSET_PREFIX,
            config::RELEASE_ASSET_SUFFIX
        ))?;
    Ok(Release {
        tag: release.tag_name,
        asset_url: asset.browser_download_url,
    })
}

struct Cached {
    checked: Instant,
    tag: String,
    code: String,
}

/// The last release found and its code.
#[derive(Default)]
pub struct ReleaseCache(Mutex<Option<Cached>>);

impl ReleaseCache {
    /// The latest ranked release's tag and Workshop code: from memory while it's fresh, else from
    /// GitHub (the code only when the tag changed).
    pub async fn latest(&self) -> Result<(String, String), String> {
        let max_age = Duration::from_secs(config::RELEASE_CACHE_SECS);
        if let Some(c) = self.0.lock().unwrap().as_ref() {
            if c.checked.elapsed() < max_age {
                return Ok((c.tag.clone(), c.code.clone()));
            }
        }
        let release = find().await?;
        let known = self
            .0
            .lock()
            .unwrap()
            .as_ref()
            .filter(|c| c.tag == release.tag)
            .map(|c| c.code.clone());
        let code = match known {
            Some(code) => code,
            None => download(&release).await?,
        };
        *self.0.lock().unwrap() = Some(Cached {
            checked: Instant::now(),
            tag: release.tag.clone(),
            code: code.clone(),
        });
        Ok((release.tag, code))
    }
}

fn unreachable(e: reqwest::Error) -> String {
    format!("Couldn't reach GitHub: {}", e.without_url())
}

async fn find() -> Result<Release, String> {
    let url = url::Url::parse_with_params(
        &format!(
            "{}/repos/{}/releases",
            config::GITHUB_API_URL,
            config::RELEASE_REPO
        ),
        [("per_page", config::RELEASES_SEARCHED.to_string())],
    )
    .map_err(|e| e.to_string())?;
    let response = server::client()?
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(unreachable)?;
    let status = response.status().as_u16();
    let body = response.text().await.map_err(unreachable)?;
    pick(status, &body)
}

async fn download(release: &Release) -> Result<String, String> {
    let response = server::client()?
        .get(&release.asset_url)
        .send()
        .await
        .map_err(unreachable)?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "GitHub answered {} when asked for release {}'s code",
            status.as_u16(),
            release.tag
        ));
    }
    response.text().await.map_err(unreachable)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, draft: bool, prerelease: bool, assets: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "tag_name": tag,
            "name": format!("Genji Ball CE v{tag}"),
            "draft": draft,
            "prerelease": prerelease,
            "assets": assets.iter().map(|name| serde_json::json!({
                "name": name,
                "size": 186627,
                "browser_download_url": format!("https://github.com/Genji-Ball-Team/GenjiBall-CE/releases/download/{tag}/{name}"),
            })).collect::<Vec<_>>(),
        })
    }

    fn list(releases: &[serde_json::Value]) -> String {
        serde_json::to_string(releases).unwrap()
    }

    #[test]
    fn picks_the_newest_ranked_release() {
        let body = list(&[
            release("1.3.4T", false, false, &["genjiball-v1.3.4T.txt"]),
            release("1.3.5R", true, false, &["genjiball-v1.3.5R.txt"]),
            release("1.3.4R", false, true, &["genjiball-v1.3.4R.txt"]),
            release(
                "1.3.3R",
                false,
                false,
                &["notes.md", "genjiball-v1.3.3R.txt"],
            ),
            release("1.3.2R", false, false, &["genjiball-v1.3.2R.txt"]),
        ]);
        assert_eq!(
            pick(200, &body).unwrap(),
            Release {
                tag: "1.3.3R".into(),
                asset_url: "https://github.com/Genji-Ball-Team/GenjiBall-CE/releases/download/1.3.3R/genjiball-v1.3.3R.txt".into(),
            }
        );
    }

    #[test]
    fn says_when_there_is_no_ranked_release() {
        // GitHub as it is now: no R release yet.
        let body = list(&[
            release("1.3.3T", false, false, &["genjiball-v1.3.3T.txt"]),
            release("1.3.3", false, false, &["genjiball-v1.3.3.txt"]),
        ]);
        assert_eq!(
            pick(200, &body).unwrap_err(),
            "No ranked (R) release of Genji-Ball-Team/GenjiBall-CE on GitHub yet"
        );
        assert!(pick(200, "[]").unwrap_err().contains("No ranked"));
    }

    #[test]
    fn says_when_the_release_has_no_code() {
        let body = list(&[release("1.3.3R", false, false, &["notes.md", "code.json"])]);
        assert!(pick(200, &body)
            .unwrap_err()
            .contains("1.3.3R has no Workshop code"));
    }

    #[test]
    fn github_errors() {
        assert!(pick(403, "{}").unwrap_err().contains("limiting"));
        assert!(pick(429, "{}").unwrap_err().contains("limiting"));
        assert!(pick(404, "{}").unwrap_err().contains("no repo"));
        assert!(pick(502, "").unwrap_err().contains("502"));
        assert!(pick(200, "<html>").is_err());
    }
}
