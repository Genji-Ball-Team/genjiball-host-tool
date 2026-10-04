//! The base of the ranked code: the Workshop code in the latest ranked (`R`) GenjiBall-CE
//! release on GitHub. Kept in memory for `RELEASE_CACHE_SECS`, and the code per asset, so a click
//! on "Copy ranked code" rarely asks GitHub.

use std::future::Future;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::sync::Mutex;

use crate::{config, server};

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct GithubAsset {
    id: u64,
    name: String,
    updated_at: String,
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
    /// The asset's id and when it was last uploaded: a re-pushed tag replaces the asset
    /// (`gh release upload --clobber` in GenjiBall-CE's release workflow) under the same tag.
    pub asset_id: u64,
    pub asset_updated_at: String,
    pub asset_url: String,
}

impl Release {
    /// Whether `other` is the same file as this, so its code needn't be downloaded again.
    fn same_asset(&self, other: &Release) -> bool {
        self.asset_id == other.asset_id && self.asset_updated_at == other.asset_updated_at
    }
}

/// The Workshop code asset's name for a tag, as GenjiBall-CE's release workflow names it:
/// `genjiball-v1.3.3R.txt` for `1.3.3R`.
pub fn asset_name(tag: &str) -> String {
    format!(
        "{}{tag}{}",
        config::RELEASE_ASSET_PREFIX,
        config::RELEASE_ASSET_SUFFIX
    )
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
    let name = asset_name(&release.tag_name);
    let mut assets = release.assets.into_iter().filter(|a| a.name == name);
    let asset = assets.next().ok_or(format!(
        "Release {} has no Workshop code file ({name})",
        release.tag_name
    ))?;
    if assets.next().is_some() {
        return Err(format!(
            "Release {} has {name} more than once, so the host tool can't tell which to use",
            release.tag_name
        ));
    }
    Ok(Release {
        tag: release.tag_name,
        asset_id: asset.id,
        asset_updated_at: asset.updated_at,
        asset_url: asset.browser_download_url,
    })
}

struct Cached {
    /// When GitHub was last asked. None: ask at the next call.
    checked: Option<Instant>,
    release: Release,
    code: String,
}

/// The last release found and its code. The lock is held while GitHub is asked, so two clicks
/// at once ask once, and an older answer never replaces a newer one.
#[derive(Default)]
pub struct ReleaseCache(Mutex<Option<Cached>>);

impl ReleaseCache {
    /// The latest ranked release's tag and Workshop code: from memory while it's fresh, else from
    /// GitHub (the code only when the asset changed).
    pub async fn latest(&self) -> Result<(String, String), String> {
        self.latest_with(find, download).await
    }

    async fn latest_with<F, FF, D, DF>(
        &self,
        find: F,
        download: D,
    ) -> Result<(String, String), String>
    where
        F: FnOnce() -> FF,
        FF: Future<Output = Result<Release, String>>,
        D: FnOnce(Release) -> DF,
        DF: Future<Output = Result<String, String>>,
    {
        let mut cached = self.0.lock().await;
        let max_age = Duration::from_secs(config::RELEASE_CACHE_SECS);
        if let Some(c) = cached
            .as_ref()
            .filter(|c| c.checked.is_some_and(|t| t.elapsed() < max_age))
        {
            return Ok((c.release.tag.clone(), c.code.clone()));
        }
        let release = find().await?;
        let code = match cached.as_ref().filter(|c| c.release.same_asset(&release)) {
            Some(c) => c.code.clone(),
            None => download(release.clone()).await?,
        };
        let tag = release.tag.clone();
        *cached = Some(Cached {
            checked: Some(Instant::now()),
            release,
            code: code.clone(),
        });
        Ok((tag, code))
    }

    /// Makes the next `latest` ask GitHub again, keeping the code. Tests only, for now.
    #[cfg(test)]
    async fn expire(&self) {
        if let Some(c) = self.0.lock().await.as_mut() {
            c.checked = None;
        }
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

async fn download(release: Release) -> Result<String, String> {
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;

    fn release(tag: &str, draft: bool, prerelease: bool, assets: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "tag_name": tag,
            "name": format!("Genji Ball CE v{tag}"),
            "draft": draft,
            "prerelease": prerelease,
            "assets": assets.iter().enumerate().map(|(i, name)| serde_json::json!({
                "id": 605952576 + i,
                "name": name,
                "size": 186627,
                "updated_at": "2026-10-03T12:00:00Z",
                "browser_download_url": format!("https://github.com/Genji-Ball-Team/GenjiBall-CE/releases/download/{tag}/{name}"),
            })).collect::<Vec<_>>(),
        })
    }

    fn list(releases: &[serde_json::Value]) -> String {
        serde_json::to_string(releases).unwrap()
    }

    #[test]
    fn names_the_asset_as_the_release_workflow_does() {
        assert_eq!(asset_name("1.3.3R"), "genjiball-v1.3.3R.txt");
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
                asset_id: 605952577,
                asset_updated_at: "2026-10-03T12:00:00Z".into(),
                asset_url: "https://github.com/Genji-Ball-Team/GenjiBall-CE/releases/download/1.3.3R/genjiball-v1.3.3R.txt".into(),
            }
        );
    }

    #[test]
    fn takes_only_the_exact_asset_name() {
        let body = list(&[release(
            "1.3.3R",
            false,
            false,
            &[
                "genjiball-v1.3.3R-debug.txt",
                "genjiball-v1.3.2R.txt",
                "genjiball-v1.3.3R.txt",
            ],
        )]);
        assert_eq!(pick(200, &body).unwrap().asset_id, 605952578);
        // Only lookalikes: not the code.
        let lookalikes = list(&[release(
            "1.3.3R",
            false,
            false,
            &["genjiball-v1.3.3R-debug.txt", "genjiball-1.3.3R.txt"],
        )]);
        assert!(pick(200, &lookalikes)
            .unwrap_err()
            .contains("1.3.3R has no Workshop code file (genjiball-v1.3.3R.txt)"));
        let twice = list(&[release(
            "1.3.3R",
            false,
            false,
            &["genjiball-v1.3.3R.txt", "genjiball-v1.3.3R.txt"],
        )]);
        assert!(pick(200, &twice).unwrap_err().contains("more than once"));
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

    fn found(tag: &str, asset_id: u64, updated_at: &str) -> Release {
        Release {
            tag: tag.into(),
            asset_id,
            asset_updated_at: updated_at.into(),
            asset_url: format!("https://example.com/{asset_id}"),
        }
    }

    /// `latest_with` with a canned release; the code downloaded is `code` and counted.
    async fn latest(
        cache: &ReleaseCache,
        release: Release,
        code: &str,
        downloads: &AtomicUsize,
    ) -> Result<(String, String), String> {
        cache
            .latest_with(
                || async move { Ok(release) },
                |_| async move {
                    downloads.fetch_add(1, Ordering::SeqCst);
                    Ok(code.to_string())
                },
            )
            .await
    }

    #[test]
    fn reuses_the_code_until_the_asset_changes() {
        tauri::async_runtime::block_on(async {
            let cache = ReleaseCache::default();
            let downloads = AtomicUsize::new(0);
            let first = found("1.3.3R", 1, "2026-10-03T12:00:00Z");
            assert_eq!(
                latest(&cache, first.clone(), "old", &downloads)
                    .await
                    .unwrap(),
                ("1.3.3R".into(), "old".into())
            );
            // Fresh: GitHub isn't asked at all.
            let never = found("9.9.9R", 9, "x");
            assert_eq!(
                latest(&cache, never, "new", &downloads).await.unwrap().1,
                "old"
            );
            // Stale but the same file: asked, not downloaded.
            cache.expire().await;
            assert_eq!(
                latest(&cache, first, "new", &downloads).await.unwrap().1,
                "old"
            );
            assert_eq!(downloads.load(Ordering::SeqCst), 1);
            // A re-pushed tag replaced the file under the same tag.
            cache.expire().await;
            let replaced = found("1.3.3R", 2, "2026-10-04T08:00:00Z");
            assert_eq!(
                latest(&cache, replaced, "new", &downloads).await.unwrap(),
                ("1.3.3R".into(), "new".into())
            );
            // Same id, uploaded again.
            cache.expire().await;
            let reuploaded = found("1.3.3R", 2, "2026-10-05T08:00:00Z");
            assert_eq!(
                latest(&cache, reuploaded, "newer", &downloads)
                    .await
                    .unwrap()
                    .1,
                "newer"
            );
            assert_eq!(downloads.load(Ordering::SeqCst), 3);
        });
    }

    #[test]
    fn a_failed_check_keeps_nothing() {
        tauri::async_runtime::block_on(async {
            let cache = ReleaseCache::default();
            let failed = cache
                .latest_with(
                    || async { Err::<Release, _>("offline".to_string()) },
                    |_| async { Ok(String::new()) },
                )
                .await;
            assert_eq!(failed.unwrap_err(), "offline");
            assert!(cache.0.lock().await.is_none());
        });
    }

    #[test]
    fn clicks_at_once_ask_github_once() {
        tauri::async_runtime::block_on(async {
            let cache = Arc::new(ReleaseCache::default());
            let finds = Arc::new(AtomicUsize::new(0));
            let tasks: Vec<_> = (0..2u64)
                .map(|i| {
                    let (cache, finds) = (cache.clone(), finds.clone());
                    tauri::async_runtime::spawn(async move {
                        cache
                            .latest_with(
                                || async move {
                                    finds.fetch_add(1, Ordering::SeqCst);
                                    tokio::time::sleep(Duration::from_millis(50)).await;
                                    Ok(found(&format!("1.3.{i}R"), i, "x"))
                                },
                                move |_| async move { Ok(format!("code {i}")) },
                            )
                            .await
                    })
                })
                .collect();
            let mut answers = Vec::new();
            for task in tasks {
                answers.push(task.await.unwrap().unwrap());
            }
            assert_eq!(finds.load(Ordering::SeqCst), 1);
            // Both got the one answer, and it's what's kept.
            assert_eq!(answers[0], answers[1]);
            let kept = cache.0.lock().await.as_ref().unwrap().code.clone();
            assert_eq!(kept, answers[0].1);
        });
    }
}
