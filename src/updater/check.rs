use super::artifact::{Release, ReleaseAsset};

pub const OWNER: &str = "dyan80514-svg";
pub const REPOSITORY: &str = "MovieBox-Tui";

pub fn release_tag_url(tag: &str) -> String {
    let tag_clean = tag.trim_start_matches('v');
    format!("https://github.com/{OWNER}/{REPOSITORY}/releases/tag/v{tag_clean}")
}

pub async fn check_release(current: &str) -> Result<Option<Release>, String> {
    let release = match fetch_release().await {
        Ok(release) => release,
        Err(error) => {
            log::warn!("update check via API failed ({error}); falling back to release page");
            let tag = fetch_latest_tag().await?;
            log::info!("resolved latest release via redirect: {tag}");
            Release {
                version: tag.trim_start_matches('v').to_string(),
                tag_name: tag,
                notes: String::new(),
                assets: Vec::new(),
            }
        }
    };

    if !is_newer(current, &release.version) {
        return Ok(None);
    }
    Ok(Some(release))
}

pub async fn check(current: &str) -> Result<Option<(String, String)>, String> {
    let release = check_release(current).await?;
    Ok(release.map(|r| (r.version, r.notes)))
}

pub fn is_newer(current: &str, other: &str) -> bool {
    let parse = |v: &str| semver::Version::parse(v.strip_prefix('v').unwrap_or(v));
    match (parse(current), parse(other)) {
        (Ok(cur), Ok(o)) => o > cur,
        _ => false,
    }
}

async fn fetch_release() -> Result<Release, String> {
    let url = format!("https://api.github.com/repos/{OWNER}/{REPOSITORY}/releases/latest");
    // Share the bounded, redirect-validated downloader with checksum requests.
    let text = super::download::download_text(&url).await?;
    let item: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("bad release JSON: {e}"))?;
    let tag = item["tag_name"].as_str().ok_or("missing tag_name")?;
    semver::Version::parse(tag.strip_prefix('v').unwrap_or(tag))
        .map_err(|_| "invalid release version")?;
    let notes = item["body"].as_str().unwrap_or("").to_string();
    let assets = if let Some(arr) = item["assets"].as_array() {
        arr.iter()
            .filter_map(|a| {
                let name = a["name"].as_str()?.to_string();
                let download_url = a["browser_download_url"].as_str()?.to_string();
                let size = a["size"].as_u64();
                Some(ReleaseAsset {
                    name,
                    download_url,
                    size,
                })
            })
            .collect()
    } else {
        Vec::new()
    };

    Ok(Release {
        version: tag.trim_start_matches('v').to_string(),
        tag_name: tag.to_string(),
        notes,
        assets,
    })
}

async fn fetch_latest_tag() -> Result<String, String> {
    let url = format!("https://github.com/{OWNER}/{REPOSITORY}/releases/latest");
    let resp = http_client()?
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("GitHub release page failed: {}", e.without_url()))?;
    if !resp.status().is_success() {
        return Err(format!("GitHub release page {}", resp.status()));
    }

    let prefix = format!("/{OWNER}/{REPOSITORY}/releases/tag/");
    let tag = resp
        .url()
        .path()
        .strip_prefix(&prefix)
        .filter(|_| resp.url().host_str() == Some("github.com"))
        .ok_or("could not resolve the latest release tag")?;
    semver::Version::parse(tag.strip_prefix('v').unwrap_or(tag))
        .map_err(|_| "invalid release version")?;
    Ok(tag.to_string())
}

fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 10 {
            return attempt.error("too many update redirects");
        }
        if !super::download::is_allowed_download_url(attempt.url()) {
            return attempt.error("refusing unsafe update redirect");
        }
        // Even local HTTP is not an acceptable downgrade from an HTTPS origin.
        if attempt.url().scheme() != "https"
            && attempt.previous().iter().any(|url| url.scheme() == "https")
        {
            return attempt.error("refusing HTTPS downgrade");
        }
        attempt.follow()
    })
}

pub(crate) fn http_client() -> Result<reqwest::Client, String> {
    crate::net::http_client_builder()
        .user_agent(crate::net::APP_HTTP_USER_AGENT)
        .redirect(redirect_policy())
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client: {e}"))
}

pub(crate) fn download_client() -> Result<reqwest::Client, String> {
    crate::net::http_client_builder()
        .user_agent(crate::net::APP_HTTP_USER_AGENT)
        .redirect(redirect_policy())
        .timeout(std::time::Duration::from_secs(300))
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("Download client: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_fail_closed() {
        assert!(is_newer("v0.1.26", "0.1.27"));
        assert!(!is_newer("0.1.26", "0.1.26"));
        assert!(!is_newer("0.1.26", "0.1.25"));
        assert!(!is_newer("0.1.26", "latest"));
        assert!(!is_newer("broken", "0.1.27"));
        assert!(!is_newer("0.1.26", "vv0.1.27"));
        assert!(!is_newer("0.1.26", "0.1.26-rc.1"));
    }

    #[test]
    fn release_links_target_this_fork() {
        assert_eq!(
            release_tag_url("v0.1.27"),
            "https://github.com/dyan80514-svg/MovieBox-Tui/releases/tag/v0.1.27"
        );
    }
}
