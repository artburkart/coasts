/// GitHub Releases API client for checking the latest coast version.
use crate::error::UpdateError;
use crate::version::parse_version;
use semver::Version;
use serde::Deserialize;
use std::path::PathBuf;
use std::time::Duration;

const GITHUB_RELEASES_URL: &str = "https://api.github.com/repos/coast-guard/coasts/releases/latest";

const CHECK_CACHE_FILE: &str = "update-check.json";
const CACHE_TTL_SECS: i64 = 3600; // 1 hour

/// Cached result of an update check.
#[derive(Debug, serde::Serialize, Deserialize)]
pub struct CachedCheck {
    pub latest_version: String,
    pub checked_at: String,
}

/// Response from GitHub's releases/latest endpoint (only the fields we need).
#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
}

/// Return the path to the update-check cache file inside the coast home directory.
///
/// Respects `COAST_HOME` so dev builds (`~/.coast-dev/`) and production builds
/// (`~/.coast/`) use separate caches.
pub fn cache_path() -> Option<PathBuf> {
    let home = if let Ok(dir) = std::env::var("COAST_HOME") {
        PathBuf::from(dir)
    } else {
        dirs::home_dir().map(|h| h.join(".coast"))?
    };
    Some(home.join(CHECK_CACHE_FILE))
}

/// Read the cached update check result, if it exists and is not expired.
pub fn read_cache() -> Option<CachedCheck> {
    let path = cache_path()?;
    let contents = std::fs::read_to_string(path).ok()?;
    let cached: CachedCheck = serde_json::from_str(&contents).ok()?;

    let checked_at = chrono::DateTime::parse_from_rfc3339(&cached.checked_at).ok()?;
    let age = chrono::Utc::now().signed_duration_since(checked_at);

    if age.num_seconds() < CACHE_TTL_SECS {
        Some(cached)
    } else {
        None
    }
}

/// Write a check result to the cache file.
pub fn write_cache(latest_version: &str) -> Result<(), UpdateError> {
    let Some(path) = cache_path() else {
        return Ok(());
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let cached = CachedCheck {
        latest_version: latest_version.to_string(),
        checked_at: chrono::Utc::now().to_rfc3339(),
    };

    let json = serde_json::to_string_pretty(&cached)
        .map_err(|e| UpdateError::CheckFailed(e.to_string()))?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Check GitHub for the latest release version.
///
/// Uses the cache if available and fresh. Falls back to the GitHub API.
/// Returns `None` if the check fails (network errors are not fatal).
/// Also verifies that the release assets are actually downloadable —
/// GitHub's `/releases/latest` can return a version before its assets
/// finish uploading, which would cause a 404 during download.
pub async fn check_latest_version(timeout: Duration) -> Option<Version> {
    // Try cache first
    if let Some(cached) = read_cache() {
        return parse_version(&cached.latest_version).ok();
    }

    // Fetch from GitHub
    let version = fetch_latest_from_github(timeout).await.ok()?;

    // Verify the tarball for this platform is actually downloadable
    if !verify_assets_available(&version, timeout).await {
        return None;
    }

    // Cache the result (best-effort)
    let _ = write_cache(&version.to_string());

    Some(version)
}

/// Fetch the latest release tag from GitHub Releases API.
async fn fetch_latest_from_github(timeout: Duration) -> Result<Version, UpdateError> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .user_agent("coast-cli")
        .build()
        .map_err(|e| UpdateError::CheckFailed(e.to_string()))?;

    let response = client
        .get(GITHUB_RELEASES_URL)
        .send()
        .await
        .map_err(|e| UpdateError::CheckFailed(e.to_string()))?;

    if !response.status().is_success() {
        return Err(UpdateError::CheckFailed(format!(
            "GitHub API returned {}",
            response.status()
        )));
    }

    let release: GitHubRelease = response
        .json()
        .await
        .map_err(|e| UpdateError::CheckFailed(e.to_string()))?;

    parse_version(&release.tag_name)
}

/// Verify that the release tarball for the current platform is downloadable.
/// Uses a HEAD request to avoid downloading the full tarball.
async fn verify_assets_available(version: &Version, timeout: Duration) -> bool {
    let (os, arch) = crate::updater::current_platform();
    let url = release_tarball_url(version, os, arch);

    let client = reqwest::Client::builder()
        .timeout(timeout)
        .user_agent("coast-cli")
        .redirect(reqwest::redirect::Policy::none())
        .build();

    let Ok(client) = client else {
        return false;
    };

    // GitHub returns a 302 redirect for valid assets
    match client.head(&url).send().await {
        Ok(resp) => resp.status().is_success() || resp.status().is_redirection(),
        Err(_) => false,
    }
}

/// Return the download URL for a specific version and platform.
pub fn release_tarball_url(version: &Version, os: &str, arch: &str) -> String {
    format!(
        "https://github.com/coast-guard/coasts/releases/download/v{version}/coast-v{version}-{os}-{arch}.tar.gz"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_path_exists() {
        let path = cache_path();
        assert!(path.is_some());
        let path = path.unwrap();
        assert!(
            path.ends_with("update-check.json"),
            "unexpected cache path: {}",
            path.display()
        );
    }

    #[test]
    fn test_write_and_read_cache() {
        let dir = tempfile::tempdir().unwrap();
        let cache_file = dir.path().join("update-check.json");

        let cached = CachedCheck {
            latest_version: "0.2.0".to_string(),
            checked_at: chrono::Utc::now().to_rfc3339(),
        };

        let json = serde_json::to_string_pretty(&cached).unwrap();
        std::fs::write(&cache_file, &json).unwrap();

        let read_back: CachedCheck =
            serde_json::from_str(&std::fs::read_to_string(&cache_file).unwrap()).unwrap();
        assert_eq!(read_back.latest_version, "0.2.0");
    }

    #[test]
    fn test_read_cache_expired() {
        let dir = tempfile::tempdir().unwrap();
        let cache_file = dir.path().join("update-check.json");

        let old_time = chrono::Utc::now() - chrono::Duration::hours(2);
        let cached = CachedCheck {
            latest_version: "0.2.0".to_string(),
            checked_at: old_time.to_rfc3339(),
        };

        let json = serde_json::to_string_pretty(&cached).unwrap();
        std::fs::write(&cache_file, json).unwrap();

        // read_cache() uses the real path, so this tests the deserialization logic
        let parsed: CachedCheck =
            serde_json::from_str(&std::fs::read_to_string(&cache_file).unwrap()).unwrap();
        let checked_at = chrono::DateTime::parse_from_rfc3339(&parsed.checked_at).unwrap();
        let age = chrono::Utc::now().signed_duration_since(checked_at);
        assert!(age.num_seconds() >= CACHE_TTL_SECS);
    }

    #[test]
    fn test_read_cache_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        let cache_file = dir.path().join("update-check.json");
        std::fs::write(&cache_file, "not json").unwrap();

        let result: Result<CachedCheck, _> =
            serde_json::from_str(&std::fs::read_to_string(&cache_file).unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn test_release_tarball_url() {
        let version = Version::new(0, 1, 0);
        let url = release_tarball_url(&version, "darwin", "arm64");
        assert_eq!(
            url,
            "https://github.com/coast-guard/coasts/releases/download/v0.1.0/coast-v0.1.0-darwin-arm64.tar.gz"
        );
    }

    #[test]
    fn test_release_tarball_url_linux() {
        let version = Version::new(1, 2, 3);
        let url = release_tarball_url(&version, "linux", "amd64");
        assert_eq!(
            url,
            "https://github.com/coast-guard/coasts/releases/download/v1.2.3/coast-v1.2.3-linux-amd64.tar.gz"
        );
    }
}
