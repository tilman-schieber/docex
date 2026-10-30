//! Is there a newer docex? One request to GitHub's "latest release"; the app
//! only says so, the person downloads it themselves.

use serde::{Deserialize, Serialize};
use std::time::Duration;

const REPO: &str = "tilman-schieber/docex";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Update {
    pub version: String,
    pub url: String,
}

/// The latest release, if it is newer than this build.
pub async fn check() -> Result<Option<Update>, String> {
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
        html_url: String,
    }
    let r = reqwest::Client::new()
        .get(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .header("User-Agent", concat!("docex/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if r.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let rel: Release = r.error_for_status().map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
    let version = rel.tag_name.trim_start_matches('v').to_string();
    Ok(newer(&version, env!("CARGO_PKG_VERSION")).then_some(Update { version, url: rel.html_url }))
}

/// "0.10.0" > "0.9.3"; a pre-release suffix ("1.0.0-rc1") counts as its base version.
pub fn newer(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> { v.split(['-', '+']).next().unwrap_or("").split('.').map(|n| n.parse().unwrap_or(0)).collect() };
    parse(a) > parse(b)
}

/// Open a release page of docex in the browser (nothing else).
pub fn open_release(url: &str) -> Result<(), String> {
    if !url.starts_with(&format!("https://github.com/{REPO}/releases/")) {
        return Err("not a docex release page".into());
    }
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(url).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::newer;

    #[test]
    fn versions_compare_by_number() {
        assert!(newer("0.10.0", "0.9.3"));
        assert!(newer("0.1.1", "0.1.0"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1.0-rc1", "0.1.0"));
        assert!(newer("1.0", "0.9.9"));
    }
}
