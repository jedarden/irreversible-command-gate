//! Self-updater command for icg
//!
//! User-triggered (not polling) update mechanism:
//! - Checks GitHub Releases API once per invocation
//! - Downloads the new rule-pack artifact per the trust pointer
//! - Atomically replaces the on-disk artifact (write-then-rename)
//! - No persistent process to restart (per-invocation architecture)

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::runtime::Runtime;
use crate::trust_pointer::*;

/// State file for tracking last successful update check
///
/// Persists the timestamp and release information from the last successful
/// update check so `icg status` can report it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCheckState {
    /// Timestamp of the last successful update check
    pub last_successful_check: String,
    /// The release tag that was checked
    pub release_tag: String,
    /// The trusted ref that was used
    pub trusted_ref: String,
}

impl UpdateCheckState {
    /// Create a new update check state record
    pub fn new(release_tag: String, trusted_ref: String) -> Self {
        Self {
            last_successful_check: chrono::Utc::now().to_rfc3339(),
            release_tag,
            trusted_ref,
        }
    }

    /// Save state to disk
    pub fn save(&self, path: &Path) -> Result<()> {
        let content = serde_json::to_string_pretty(self)
            .context("Failed to serialize update check state")?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        // Write to temporary file first, then atomic rename
        let temp_path = path.with_extension("tmp");
        fs::write(&temp_path, content)
            .with_context(|| format!("Failed to write update check state to {}", temp_path.display()))?;

        fs::rename(&temp_path, path)
            .with_context(|| format!("Failed to rename update check state from {} to {}", temp_path.display(), path.display()))?;

        Ok(())
    }

    /// Load state from disk
    pub fn load(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read update check state from {}", path.display()))?;

        let state: UpdateCheckState = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse update check state from {}", path.display()))?;

        Ok(Some(state))
    }
}

/// GitHub Release asset structure
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

/// GitHub Release structure
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    name: String,
    published_at: String,
    assets: Vec<GitHubAsset>,
}

/// Configuration for the updater
pub struct UpdateConfig {
    /// GitHub repository (owner/repo format)
    pub repository: String,
    /// Rule pack artifact name pattern to download
    pub artifact_pattern: String,
    /// Where to store the rule pack artifact
    pub artifact_path: PathBuf,
    /// Trust pointer path
    pub trust_pointer_path: PathBuf,
    /// Path to the update check state file
    pub state_path: PathBuf,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        // Use root-owned system location, not user-writable path
        // See docs/plan/plan.md Architecture 'Deploy location'
        let artifact_path = PathBuf::from("/etc/icg/rule-pack.json");

        Self {
            repository: "jedarden/irreversible-command-gate".to_string(),
            artifact_pattern: "rule-pack".to_string(),
            artifact_path,
            trust_pointer_path: PathBuf::from("/etc/icg/trust-pointer.json"),
            state_path: PathBuf::from("/etc/icg/last-update-check.json"),
        }
    }
}

/// Update result
#[derive(Debug)]
pub struct UpdateResult {
    /// Whether an update was performed
    pub updated: bool,
    /// The release reference that was used
    pub trusted_ref: String,
    /// The release version/tag
    pub release_tag: String,
    /// Path to the updated artifact
    pub artifact_path: PathBuf,
    /// Current version (if any)
    pub previous_version: Option<String>,
}

impl UpdateResult {
    /// Create a new update result
    pub fn new(
        updated: bool,
        trusted_ref: String,
        release_tag: String,
        artifact_path: PathBuf,
        previous_version: Option<String>,
    ) -> Self {
        Self {
            updated,
            trusted_ref,
            release_tag,
            artifact_path,
            previous_version,
        }
    }
}

/// Run the self-updater
pub fn run_update(config: UpdateConfig) -> Result<UpdateResult> {
    let rt = Runtime::new()
        .context("Failed to create async runtime for updater")?;

    rt.block_on(async {
        run_update_async(config).await
    })
}

/// Async implementation of the updater
async fn run_update_async(config: UpdateConfig) -> Result<UpdateResult> {
    // Load the trust pointer to get the trusted reference
    let trust_store = TrustPointerStore::new(&config.trust_pointer_path);
    let trusted_ref = trust_store.get_trusted_ref()?
        .context("No trust pointer exists. Set one with: icg trust set <reference>")?;

    println!("📋 Trusted reference: `{}`", trusted_ref);

    // Check GitHub Releases API for the release
    let release = fetch_github_release(&config.repository, &trusted_ref).await?;

    println!("🔍 Found release: {} ({})", release.name, release.tag_name);

    // Find the rule-pack artifact
    let artifact = release.assets
        .into_iter()
        .find(|a| a.name.contains(&config.artifact_pattern))
        .context(format!(
            "No artifact matching '{}' found in release {}",
            config.artifact_pattern, release.tag_name
        ))?;

    println!("📦 Artifact: {} ({} bytes)", artifact.name, artifact.size);

    // Check if we already have this version
    let previous_version = if config.artifact_path.exists() {
        // Try to read version from existing artifact
        // For now, we'll just note that it exists
        Some("existing".to_string())
    } else {
        None
    };

    // Download the artifact to a temporary file
    let temp_path = config.artifact_path.with_extension("tmp");
    download_artifact(&artifact.browser_download_url, &temp_path).await?;

    // Atomically replace the artifact
    atomic_replace(&temp_path, &config.artifact_path)?;

    println!("✅ Updated successfully: {}", config.artifact_path.display());

    // Save the update check state
    let state = UpdateCheckState::new(release.tag_name.clone(), trusted_ref.clone());
    state.save(&config.state_path)
        .context("Failed to save update check state")?;

    Ok(UpdateResult {
        updated: true,
        trusted_ref,
        release_tag: release.tag_name,
        artifact_path: config.artifact_path,
        previous_version,
    })
}

/// Fetch a release from GitHub
async fn fetch_github_release(repository: &str, reference: &str) -> Result<GitHubRelease> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("Failed to build HTTP client")?;

    // Try to fetch by tag first, then by commit SHA if that fails
    let url = format!(
        "https://api.github.com/repos/{}/releases/tags/{}",
        repository, reference
    );

    println!("🌐 Checking GitHub Releases API: {}", url);

    let response = client
        .get(&url)
        .header("User-Agent", "icg-updater/0.1.0")
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await;

    match response {
        Ok(resp) if resp.status().is_success() => {
            let release = resp
                .json::<GitHubRelease>()
                .await
                .context("Failed to parse GitHub release response")?;
            Ok(release)
        }
        Ok(resp) => {
            let status = resp.status();
            let error_text = resp.text().await.unwrap_or_else(|_| "unknown".to_string());
            anyhow::bail!(
                "GitHub API returned error {}: {}",
                status,
                error_text
            );
        }
        Err(e) => {
            anyhow::bail!("Failed to fetch release from GitHub: {}", e);
        }
    }
}

/// Download an artifact to a file
async fn download_artifact(url: &str, dest_path: &Path) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300)) // 5 minute timeout for download
        .build()
        .context("Failed to build HTTP client")?;

    println!("⬇️  Downloading: {}", url);

    let response = client
        .get(url)
        .header("User-Agent", "icg-updater/0.1.0")
        .send()
        .await
        .context("Failed to download artifact")?;

    if !response.status().is_success() {
        anyhow::bail!(
            "Download failed with status {}: {}",
            response.status(),
            response.text().await.unwrap_or_else(|_| "unknown".to_string())
        );
    }

    let bytes = response
        .bytes()
        .await
        .context("Failed to read response body")?;

    // Write to temporary file
    let mut file = std::fs::File::create(dest_path)
        .with_context(|| format!("Failed to create temporary file: {}", dest_path.display()))?;

    file.write_all(&bytes)
        .with_context(|| format!("Failed to write to temporary file: {}", dest_path.display()))?;

    println!("✅ Download complete: {} bytes", bytes.len());

    Ok(())
}

/// Atomically replace a file (write-then-rename pattern)
fn atomic_replace(temp_path: &Path, final_path: &Path) -> Result<()> {
    // Ensure the destination directory exists
    if let Some(parent) = final_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
    }

    // Atomic rename
    std::fs::rename(temp_path, final_path)
        .with_context(|| {
            format!(
                "Failed to rename {} to {}",
                temp_path.display(),
                final_path.display()
            )
        })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_atomic_replace() {
        let dir = tempdir().unwrap();
        let temp_path = dir.path().join("temp.txt");
        let final_path = dir.path().join("final.txt");

        // Write temporary file
        std::fs::write(&temp_path, b"test content").unwrap();

        // Atomic replace
        atomic_replace(&temp_path, &final_path).unwrap();

        // Verify final file exists and temp is gone
        assert!(final_path.exists());
        assert!(!temp_path.exists());
        assert_eq!(
            std::fs::read_to_string(&final_path).unwrap(),
            "test content"
        );
    }

    #[test]
    fn test_atomic_replace_creates_directory() {
        let dir = tempdir().unwrap();
        let nested_dir = dir.path().join("nested").join("dir");
        let temp_path = dir.path().join("temp.txt");
        let final_path = nested_dir.join("final.txt");

        // Write temporary file
        std::fs::write(&temp_path, b"test content").unwrap();

        // Atomic replace (should create nested directory)
        atomic_replace(&temp_path, &final_path).unwrap();

        // Verify final file exists and directory was created
        assert!(final_path.exists());
        assert_eq!(
            std::fs::read_to_string(&final_path).unwrap(),
            "test content"
        );
    }

    #[test]
    fn test_update_config_default() {
        let config = UpdateConfig::default();
        assert_eq!(config.repository, "jedarden/irreversible-command-gate");
        assert_eq!(config.artifact_pattern, "rule-pack");
        assert_eq!(config.state_path, PathBuf::from("/etc/icg/last-update-check.json"));
    }

    #[test]
    fn test_update_check_state_save_and_load() -> Result<()> {
        let dir = tempdir()?;
        let state_path = dir.path().join("update-check-state.json");

        // Create and save state
        let state = UpdateCheckState::new(
            "icg-v0.1.0".to_string(),
            "v0.1.0".to_string(),
        );
        state.save(&state_path)?;

        // Load it back
        let loaded = UpdateCheckState::load(&state_path)?.unwrap();
        assert_eq!(loaded.release_tag, "icg-v0.1.0");
        assert_eq!(loaded.trusted_ref, "v0.1.0");
        assert!(loaded.last_successful_check.len() > 0); // Should have a timestamp

        Ok(())
    }

    #[test]
    fn test_update_check_state_load_nonexistent() -> Result<()> {
        let dir = tempdir()?;
        let state_path = dir.path().join("nonexistent.json");

        // Loading nonexistent file should return Ok(None)
        let loaded = UpdateCheckState::load(&state_path)?;
        assert!(loaded.is_none());

        Ok(())
    }

    #[test]
    fn test_update_check_state_creates_parent_directory() -> Result<()> {
        let dir = tempdir()?;
        let nested_path = dir.path().join("nested").join("dir").join("state.json");

        // Create and save state (should create nested directories)
        let state = UpdateCheckState::new(
            "icg-v0.1.0".to_string(),
            "v0.1.0".to_string(),
        );
        state.save(&nested_path)?;

        // Verify it exists and can be loaded
        let loaded = UpdateCheckState::load(&nested_path)?.unwrap();
        assert_eq!(loaded.release_tag, "icg-v0.1.0");

        Ok(())
    }
}
