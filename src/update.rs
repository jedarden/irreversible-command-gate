//! Self-updater command for icg
//!
//! User-triggered (not polling) update mechanism:
//! - Checks GitHub Releases API once per invocation
//! - Downloads the modular pack archive named by the trusted release
//! - Validates every manifest before atomically activating a complete pack tree
//! - No persistent process to restart (per-invocation architecture)

use crate::trust_pointer::*;
use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;
use tar::Archive;
use tokio::runtime::Runtime;
use uuid::Uuid;

const DEFAULT_PACK_DIRECTORY: &str = "/etc/icg/packs";
const DEFAULT_TRUST_POINTER_PATH: &str = "/etc/icg/trust-pointer.json";
const DEFAULT_STATE_PATH: &str = "/etc/icg/last-update-check.json";
const PACK_ARCHIVE_NAME: &str = "icg-packs.tar.gz";
const MAX_PACK_COUNT: usize = 256;
const MAX_PACK_BYTES: u64 = 4 * 1024 * 1024;
const MAX_TOTAL_PACK_BYTES: u64 = 32 * 1024 * 1024;

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
        let content =
            serde_json::to_string_pretty(self).context("Failed to serialize update check state")?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        // Write to temporary file first, then atomic rename
        let temp_path = path.with_extension("tmp");
        fs::write(&temp_path, content).with_context(|| {
            format!(
                "Failed to write update check state to {}",
                temp_path.display()
            )
        })?;

        fs::rename(&temp_path, path).with_context(|| {
            format!(
                "Failed to rename update check state from {} to {}",
                temp_path.display(),
                path.display()
            )
        })?;

        Ok(())
    }

    /// Load state from disk
    pub fn load(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(path).with_context(|| {
            format!("Failed to read update check state from {}", path.display())
        })?;

        let state: UpdateCheckState = serde_json::from_str(&content).with_context(|| {
            format!("Failed to parse update check state from {}", path.display())
        })?;

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
    /// Base URL for the GitHub Releases API.
    ///
    /// Production uses the public GitHub API.  Keeping the endpoint in the
    /// configuration also lets integration tests exercise the complete update
    /// path against a local release fixture without making network calls.
    pub release_api_base_url: String,
    /// Exact modular pack archive to download from the trusted release.
    pub pack_archive_name: String,
    /// Where to activate the complete modular pack directory.
    pub pack_directory: PathBuf,
    /// Trust pointer path
    pub trust_pointer_path: PathBuf,
    /// Path to the update check state file
    pub state_path: PathBuf,
    /// Optional channel identifier for canary rollout (e.g., "canary", "stable")
    ///
    /// When set, the updater uses a channel-specific trust pointer file
    /// (`trust-pointer-<channel>.json`) instead of the default.
    /// This supports staged rollout patterns where different fleet segments
    /// track different release channels.
    pub channel: Option<String>,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        // Use root-owned system location, not user-writable path
        // See docs/plan/plan.md Architecture 'Deploy location'
        Self {
            repository: "jedarden/irreversible-command-gate".to_string(),
            release_api_base_url: "https://api.github.com".to_string(),
            pack_archive_name: PACK_ARCHIVE_NAME.to_string(),
            pack_directory: PathBuf::from(DEFAULT_PACK_DIRECTORY),
            trust_pointer_path: PathBuf::from(DEFAULT_TRUST_POINTER_PATH),
            state_path: PathBuf::from(DEFAULT_STATE_PATH),
            channel: None,
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
    /// Path to the updated modular pack directory.
    pub pack_directory: PathBuf,
    /// The prior active directory, retained for an administrator rollback.
    pub rollback_directory: Option<PathBuf>,
    /// Current version (if any)
    pub previous_version: Option<String>,
}

impl UpdateResult {
    /// Create a new update result
    pub fn new(
        updated: bool,
        trusted_ref: String,
        release_tag: String,
        pack_directory: PathBuf,
        rollback_directory: Option<PathBuf>,
        previous_version: Option<String>,
    ) -> Self {
        Self {
            updated,
            trusted_ref,
            release_tag,
            pack_directory,
            rollback_directory,
            previous_version,
        }
    }
}

/// Run the self-updater
pub fn run_update(config: UpdateConfig) -> Result<UpdateResult> {
    let rt = Runtime::new().context("Failed to create async runtime for updater")?;

    rt.block_on(async { run_update_async(config).await })
}

/// Async implementation of the updater
async fn run_update_async(config: UpdateConfig) -> Result<UpdateResult> {
    let trust_pointer_path = resolve_trust_pointer_path(&config)?;
    let pack_directory = resolve_pack_directory(&config)?;
    let state_path = resolve_state_path(&config)?;

    // Load the trust pointer to get the trusted reference
    let trust_store = TrustPointerStore::new(&trust_pointer_path);
    let trusted_ref = trust_store
        .get_trusted_ref()?
        .context("No trust pointer exists. Set one with: icg trust set <reference>")?;

    if let Some(channel) = &config.channel {
        println!("📋 Channel: `{}`", channel);
    }
    println!("📋 Trusted reference: `{}`", trusted_ref);

    // Check GitHub Releases API for the release
    let release = fetch_github_release(
        &config.release_api_base_url,
        &config.repository,
        &trusted_ref,
    )
    .await?;

    println!("🔍 Found release: {} ({})", release.name, release.tag_name);

    // The production hook reads a directory because empty-keyword and
    // content-mode packs cannot be represented by the merged legacy asset.
    // Select the exact release asset rather than accepting a substring match.
    let matching_assets = release
        .assets
        .into_iter()
        .filter(|asset| asset.name == config.pack_archive_name)
        .collect::<Vec<_>>();
    let artifact = match matching_assets.as_slice() {
        [artifact] => artifact,
        [] => anyhow::bail!(
            "Release {} does not contain the required modular pack archive '{}'",
            release.tag_name,
            config.pack_archive_name
        ),
        _ => anyhow::bail!(
            "Release {} contains more than one asset named '{}'; refusing ambiguous update",
            release.tag_name,
            config.pack_archive_name
        ),
    };

    println!("📦 Artifact: {} ({} bytes)", artifact.name, artifact.size);

    // Check if we already have this version
    let previous_version = if pack_directory.exists() {
        Some("existing".to_string())
    } else {
        None
    };

    let staging_directory = create_staging_directory(&pack_directory)?;
    let archive_path = sibling_path(&pack_directory, "download", "tar.gz")?;
    let deployment = async {
        download_artifact(&artifact.browser_download_url, &archive_path, artifact.size).await?;
        let pack_count = extract_and_validate_pack_archive(&archive_path, &staging_directory)?;
        let rollback_directory = atomic_replace_directory(&staging_directory, &pack_directory)?;
        Ok::<_, anyhow::Error>((pack_count, rollback_directory))
    }
    .await;

    let _ = fs::remove_file(&archive_path);
    let (pack_count, rollback_directory) = match deployment {
        Ok(result) => result,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging_directory);
            return Err(error);
        }
    };

    println!(
        "✅ Updated {} rule pack(s) successfully: {}",
        pack_count,
        pack_directory.display()
    );
    if let Some(rollback_directory) = &rollback_directory {
        println!(
            "↩️  Previous pack directory retained at: {}",
            rollback_directory.display()
        );
    }

    // Save the update check state
    let state = UpdateCheckState::new(release.tag_name.clone(), trusted_ref.clone());
    if let Err(error) = state.save(&state_path) {
        // Activation is already complete and the prior directory is retained.
        // Bookkeeping must never turn a completed atomic deployment into a
        // reported failure that invites a second, unsafe attempt.
        eprintln!(
            "⚠️  Updated packs but could not record update state at {}: {error:#}",
            state_path.display()
        );
    }

    Ok(UpdateResult {
        updated: true,
        trusted_ref,
        release_tag: release.tag_name,
        pack_directory,
        rollback_directory,
        previous_version,
    })
}

/// Fetch a release from GitHub
async fn fetch_github_release(
    release_api_base_url: &str,
    repository: &str,
    reference: &str,
) -> Result<GitHubRelease> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("Failed to build HTTP client")?;

    // Try to fetch by tag first, then by commit SHA if that fails
    let url = format!(
        "{}/repos/{}/releases/tags/{}",
        release_api_base_url.trim_end_matches('/'),
        repository,
        reference
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
            anyhow::bail!("GitHub API returned error {}: {}", status, error_text);
        }
        Err(e) => {
            anyhow::bail!("Failed to fetch release from GitHub: {}", e);
        }
    }
}

/// Download an artifact to a file
async fn download_artifact(url: &str, dest_path: &Path, expected_size: u64) -> Result<()> {
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
            response
                .text()
                .await
                .unwrap_or_else(|_| "unknown".to_string())
        );
    }

    let bytes = response
        .bytes()
        .await
        .context("Failed to read response body")?;

    if bytes.len() as u64 != expected_size {
        anyhow::bail!(
            "Downloaded artifact size {} does not match trusted release metadata {}",
            bytes.len(),
            expected_size
        );
    }

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dest_path)
        .with_context(|| format!("Failed to create temporary file: {}", dest_path.display()))?;

    file.write_all(&bytes)
        .with_context(|| format!("Failed to write to temporary file: {}", dest_path.display()))?;
    file.sync_all()
        .with_context(|| format!("Failed to sync temporary file: {}", dest_path.display()))?;

    println!("✅ Download complete: {} bytes", bytes.len());

    Ok(())
}

fn resolve_trust_pointer_path(config: &UpdateConfig) -> Result<PathBuf> {
    let Some(channel) = &config.channel else {
        return Ok(config.trust_pointer_path.clone());
    };
    validate_channel(channel)?;
    if config.trust_pointer_path == Path::new(DEFAULT_TRUST_POINTER_PATH) {
        Ok(TrustPointerStore::for_channel(channel).path().to_path_buf())
    } else {
        Ok(config.trust_pointer_path.clone())
    }
}

fn resolve_pack_directory(config: &UpdateConfig) -> Result<PathBuf> {
    let Some(channel) = &config.channel else {
        return Ok(config.pack_directory.clone());
    };
    validate_channel(channel)?;
    if config.pack_directory == Path::new(DEFAULT_PACK_DIRECTORY) {
        Ok(PathBuf::from(format!("{DEFAULT_PACK_DIRECTORY}-{channel}")))
    } else {
        Ok(config.pack_directory.clone())
    }
}

fn resolve_state_path(config: &UpdateConfig) -> Result<PathBuf> {
    let Some(channel) = &config.channel else {
        return Ok(config.state_path.clone());
    };
    validate_channel(channel)?;
    if config.state_path == Path::new(DEFAULT_STATE_PATH) {
        Ok(PathBuf::from(format!(
            "/etc/icg/last-update-check-{channel}.json"
        )))
    } else {
        Ok(config.state_path.clone())
    }
}

fn validate_channel(channel: &str) -> Result<()> {
    if channel.is_empty()
        || !channel
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        anyhow::bail!(
            "Invalid update channel '{channel}'; use only ASCII letters, numbers, '-' and '_'"
        );
    }
    Ok(())
}

fn create_staging_directory(pack_directory: &Path) -> Result<PathBuf> {
    let parent = prepare_deployment_parent(pack_directory)?;
    for _ in 0..8 {
        let staging = parent.join(format!(".icg-packs-staging-{}", Uuid::new_v4()));
        match fs::create_dir(&staging) {
            Ok(()) => {
                set_directory_mode(&staging, 0o755)?;
                validate_production_directory_security(&staging)?;
                return Ok(staging);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("Failed to create staging directory: {}", staging.display())
                })
            }
        }
    }
    anyhow::bail!("Could not allocate a unique pack staging directory")
}

fn prepare_deployment_parent(pack_directory: &Path) -> Result<&Path> {
    let parent = pack_directory
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .context("Pack directory must have a parent directory")?;
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "Failed to create pack parent directory: {}",
            parent.display()
        )
    })?;
    ensure_real_directory(parent, "pack parent directory")?;
    validate_production_directory_security(parent)?;
    Ok(parent)
}

fn extract_and_validate_pack_archive(
    archive_path: &Path,
    staging_directory: &Path,
) -> Result<usize> {
    let archive_file = fs::File::open(archive_path).with_context(|| {
        format!(
            "Failed to open downloaded archive: {}",
            archive_path.display()
        )
    })?;
    let decoder = GzDecoder::new(archive_file);
    let mut archive = Archive::new(decoder);
    let mut engine = crate::engine::Engine::new();
    let mut names = HashSet::new();
    let mut pack_ids = HashSet::new();
    let mut total_bytes = 0_u64;

    for entry in archive
        .entries()
        .context("Downloaded pack artifact is not a readable gzip tar archive")?
    {
        let mut entry = entry.context("Failed to read entry from pack archive")?;
        let path = entry
            .path()
            .context("Pack archive entry has an invalid path")?
            .into_owned();
        let entry_type = entry.header().entry_type();

        if entry_type.is_dir() {
            validate_root_directory_entry(&path)?;
            continue;
        }
        if !entry_type.is_file() {
            anyhow::bail!(
                "Pack archive entry '{}' is not a regular file; links and special files are forbidden",
                path.display()
            );
        }

        let name = archive_pack_filename(&path)?;
        if !names.insert(name.to_string()) {
            anyhow::bail!("Pack archive contains duplicate file '{name}'");
        }
        if names.len() > MAX_PACK_COUNT {
            anyhow::bail!("Pack archive contains more than {MAX_PACK_COUNT} manifests");
        }
        let size = entry.size();
        if size > MAX_PACK_BYTES {
            anyhow::bail!("Pack archive entry '{name}' exceeds {MAX_PACK_BYTES} bytes");
        }
        total_bytes = total_bytes
            .checked_add(size)
            .context("Pack archive size overflow")?;
        if total_bytes > MAX_TOTAL_PACK_BYTES {
            anyhow::bail!("Pack archive exceeds {MAX_TOTAL_PACK_BYTES} bytes of manifest data");
        }

        let destination = staging_directory.join(name);
        let mut destination_file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .with_context(|| {
                format!(
                    "Failed to create staged manifest: {}",
                    destination.display()
                )
            })?;
        let extracted = std::io::copy(&mut entry, &mut destination_file).with_context(|| {
            format!(
                "Failed to extract staged manifest: {}",
                destination.display()
            )
        })?;
        if extracted != size {
            anyhow::bail!(
                "Pack archive entry '{name}' ended after {extracted} bytes; expected {size} bytes"
            );
        }
        destination_file.sync_all().with_context(|| {
            format!("Failed to sync staged manifest: {}", destination.display())
        })?;
        set_file_mode(&destination, 0o644)?;

        let pack = crate::rule_pack::load_pack(&destination)
            .with_context(|| format!("Pack archive manifest '{}' is invalid", name))?;
        if !pack_ids.insert(pack.id.clone()) {
            anyhow::bail!("Pack archive contains duplicate pack id '{}'", pack.id);
        }
        engine.load_pack(pack)?;
        if engine.has_guard_failure() {
            anyhow::bail!("Pack archive manifest '{}' failed engine validation", name);
        }
    }

    if names.is_empty() {
        anyhow::bail!("Pack archive contains no JSON manifests");
    }
    sync_directory(staging_directory)?;
    Ok(names.len())
}

fn validate_root_directory_entry(path: &Path) -> Result<()> {
    if path
        .components()
        .all(|component| component == Component::CurDir)
    {
        return Ok(());
    }
    anyhow::bail!(
        "Pack archive directory entry '{}' is nested; manifests must be at the archive root",
        path.display()
    )
}

fn archive_pack_filename(path: &Path) -> Result<&str> {
    let components = path.components().collect::<Vec<_>>();
    let name = match components.as_slice() {
        [Component::Normal(name)] | [Component::CurDir, Component::Normal(name)] => name,
        _ => anyhow::bail!(
            "Pack archive entry '{}' is not a root-level manifest (traversal and nested paths are forbidden)",
            path.display()
        ),
    };
    let name = name
        .to_str()
        .context("Pack archive entry name is not valid UTF-8")?;
    if !name.ends_with(".json") || name == ".json" {
        anyhow::bail!("Pack archive entry '{name}' is not a JSON manifest");
    }
    Ok(name)
}

/// Atomically activate a fully validated sibling directory. Existing packs not
/// present in the archive disappear because the directory itself is exchanged,
/// never updated in place.
fn atomic_replace_directory(
    staging_directory: &Path,
    pack_directory: &Path,
) -> Result<Option<PathBuf>> {
    ensure_real_directory(staging_directory, "staging directory")?;
    match fs::symlink_metadata(pack_directory) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                anyhow::bail!(
                    "Active pack path is a symlink: {}",
                    pack_directory.display()
                );
            }
            if !metadata.is_dir() {
                anyhow::bail!(
                    "Active pack path is not a directory: {}",
                    pack_directory.display()
                );
            }
            validate_production_directory_security(pack_directory)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::rename(staging_directory, pack_directory).with_context(|| {
                format!(
                    "Failed to activate initial pack directory {}",
                    pack_directory.display()
                )
            })?;
            if let Err(error) = sync_directory(
                pack_directory
                    .parent()
                    .context("Pack directory unexpectedly has no parent")?,
            ) {
                eprintln!(
                    "⚠️  Activated initial pack directory but could not sync parent: {error:#}"
                );
            }
            return Ok(None);
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "Failed to inspect active pack directory: {}",
                    pack_directory.display()
                )
            })
        }
    }

    let rollback_directory = rollback_directory(pack_directory)?;
    // Use symlink_metadata rather than exists(): a dangling symlink reports
    // false from exists(), but must still be rejected before it can be
    // replaced as the rollback destination.
    let retired_rollback = match fs::symlink_metadata(&rollback_directory) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                anyhow::bail!(
                    "Rollback pack path is a symlink: {}",
                    rollback_directory.display()
                );
            }
            if !metadata.is_dir() {
                anyhow::bail!(
                    "Rollback pack path is not a directory: {}",
                    rollback_directory.display()
                );
            }
            validate_production_directory_security(&rollback_directory)?;
            let retired = sibling_path(pack_directory, "retired-rollback", "dir")?;
            fs::rename(&rollback_directory, &retired).with_context(|| {
                format!(
                    "Failed to rotate existing rollback directory {}",
                    rollback_directory.display()
                )
            })?;
            Some(retired)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "Failed to inspect rollback pack directory: {}",
                    rollback_directory.display()
                )
            })
        }
    };

    if let Err(error) = atomic_exchange_directories(pack_directory, staging_directory) {
        restore_retired_rollback(retired_rollback.as_deref(), &rollback_directory);
        return Err(error).with_context(|| {
            format!(
                "Failed to atomically exchange {} and {}",
                pack_directory.display(),
                staging_directory.display()
            )
        });
    }

    if let Err(error) = fs::rename(staging_directory, &rollback_directory) {
        // The active directory has already been exchanged atomically. Never
        // report this as a failed update: that would violate the promise that
        // every returned failure leaves the active policy untouched. The old
        // tree still exists at the staging path, so make its recovery location
        // explicit and leave any older rollback tree intact as well.
        eprintln!(
            "⚠️  Activated new packs but could not rename the prior tree to {}: {error}. \
             The rollback directory remains at {}",
            rollback_directory.display(),
            staging_directory.display()
        );
        return Ok(Some(staging_directory.to_path_buf()));
    }

    if let Some(retired_rollback) = retired_rollback {
        if let Err(error) = fs::remove_dir_all(&retired_rollback) {
            eprintln!(
                "⚠️  Replaced rollback directory retained at {}: {error}",
                retired_rollback.display()
            );
        }
    }
    if let Err(error) = sync_directory(
        pack_directory
            .parent()
            .context("Pack directory unexpectedly has no parent")?,
    ) {
        eprintln!("⚠️  Activated pack directory but could not sync parent: {error:#}");
    }
    Ok(Some(rollback_directory))
}

fn restore_retired_rollback(retired_rollback: Option<&Path>, rollback_directory: &Path) {
    if let Some(retired_rollback) = retired_rollback {
        let _ = fs::rename(retired_rollback, rollback_directory);
    }
}

fn rollback_directory(pack_directory: &Path) -> Result<PathBuf> {
    let name = pack_directory
        .file_name()
        .and_then(|name| name.to_str())
        .context("Pack directory must have a UTF-8 final path component")?;
    Ok(pack_directory.with_file_name(format!("{name}.previous")))
}

fn sibling_path(pack_directory: &Path, purpose: &str, extension: &str) -> Result<PathBuf> {
    let parent = pack_directory
        .parent()
        .context("Pack directory must have a parent directory")?;
    let name = pack_directory
        .file_name()
        .and_then(|name| name.to_str())
        .context("Pack directory must have a UTF-8 final path component")?;
    Ok(parent.join(format!(
        ".{name}.{purpose}-{}.{}",
        Uuid::new_v4(),
        extension
    )))
}

fn ensure_real_directory(path: &Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("Failed to inspect {label}: {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        anyhow::bail!("{label} is a symlink: {}", path.display());
    }
    if !metadata.is_dir() {
        anyhow::bail!("{label} is not a directory: {}", path.display());
    }
    Ok(())
}

fn validate_production_directory_security(path: &Path) -> Result<()> {
    if !path.starts_with("/etc/icg") {
        return Ok(());
    }
    #[cfg(unix)]
    {
        let metadata = fs::metadata(path).with_context(|| {
            format!("Failed to inspect production directory: {}", path.display())
        })?;
        if metadata.uid() != 0 {
            anyhow::bail!("Production directory is not root-owned: {}", path.display());
        }
        if metadata.permissions().mode() & 0o022 != 0 {
            anyhow::bail!(
                "Production directory is group/world writable: {}",
                path.display()
            );
        }
    }
    Ok(())
}

#[cfg(unix)]
fn set_directory_mode(path: &Path, mode: u32) -> Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .with_context(|| format!("Failed to set directory mode on {}", path.display()))
}

#[cfg(not(unix))]
fn set_directory_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_file_mode(path: &Path, mode: u32) -> Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .with_context(|| format!("Failed to set file mode on {}", path.display()))
}

#[cfg(not(unix))]
fn set_file_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    fs::File::open(path)
        .with_context(|| format!("Failed to open directory for sync: {}", path.display()))?
        .sync_all()
        .with_context(|| format!("Failed to sync directory: {}", path.display()))
}

#[cfg(target_os = "linux")]
fn atomic_exchange_directories(left: &Path, right: &Path) -> std::io::Result<()> {
    let left = std::ffi::CString::new(left.as_os_str().as_bytes())
        .expect("Unix paths cannot contain NUL bytes");
    let right = std::ffi::CString::new(right.as_os_str().as_bytes())
        .expect("Unix paths cannot contain NUL bytes");
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            left.as_ptr(),
            libc::AT_FDCWD,
            right.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "linux"))]
fn atomic_exchange_directories(_left: &Path, _right: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "atomic pack directory exchange requires Linux renameat2",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    #[cfg(target_os = "linux")]
    fn test_atomic_replace_directory_preserves_prior_tree() {
        let dir = tempdir().unwrap();
        let active = dir.path().join("packs");
        let staging = dir.path().join("staging");
        std::fs::create_dir(&active).unwrap();
        std::fs::create_dir(&staging).unwrap();
        std::fs::write(active.join("old.json"), b"old").unwrap();
        std::fs::write(staging.join("new.json"), b"new").unwrap();

        let rollback = atomic_replace_directory(&staging, &active).unwrap();

        assert_eq!(std::fs::read(active.join("new.json")).unwrap(), b"new");
        assert_eq!(
            std::fs::read(rollback.unwrap().join("old.json")).unwrap(),
            b"old"
        );
    }

    #[test]
    #[cfg(all(unix, target_os = "linux"))]
    fn atomic_replace_directory_rejects_dangling_rollback_symlink() {
        let dir = tempdir().unwrap();
        let active = dir.path().join("packs");
        let staging = dir.path().join("staging");
        std::fs::create_dir(&active).unwrap();
        std::fs::create_dir(&staging).unwrap();
        std::fs::write(active.join("old.json"), b"old").unwrap();
        std::fs::write(staging.join("new.json"), b"new").unwrap();
        std::os::unix::fs::symlink(
            dir.path().join("missing-rollback"),
            active.with_file_name("packs.previous"),
        )
        .unwrap();

        let error = atomic_replace_directory(&staging, &active).unwrap_err();

        assert!(error
            .to_string()
            .contains("Rollback pack path is a symlink"));
        assert_eq!(std::fs::read(active.join("old.json")).unwrap(), b"old");
        assert_eq!(std::fs::read(staging.join("new.json")).unwrap(), b"new");
    }

    #[test]
    fn test_update_config_default() {
        let config = UpdateConfig::default();
        assert_eq!(config.repository, "jedarden/irreversible-command-gate");
        assert_eq!(config.pack_archive_name, "icg-packs.tar.gz");
        assert_eq!(config.pack_directory, PathBuf::from("/etc/icg/packs"));
        assert_eq!(
            config.state_path,
            PathBuf::from("/etc/icg/last-update-check.json")
        );
    }

    #[test]
    fn update_channels_get_isolated_default_paths() {
        let config = UpdateConfig {
            channel: Some("canary".to_string()),
            ..Default::default()
        };
        assert_eq!(
            resolve_trust_pointer_path(&config).unwrap(),
            PathBuf::from("/etc/icg/trust-pointer-canary.json")
        );
        assert_eq!(
            resolve_pack_directory(&config).unwrap(),
            PathBuf::from("/etc/icg/packs-canary")
        );
        assert_eq!(
            resolve_state_path(&config).unwrap(),
            PathBuf::from("/etc/icg/last-update-check-canary.json")
        );
        assert!(validate_channel("../escape").is_err());
    }

    #[test]
    fn archive_layout_rejects_traversal_and_nested_paths() {
        assert!(archive_pack_filename(Path::new("../escaped.json")).is_err());
        assert!(archive_pack_filename(Path::new("packs/secrets.json")).is_err());
        assert!(archive_pack_filename(Path::new("/escaped.json")).is_err());
        assert!(archive_pack_filename(Path::new("./secrets.json")).is_ok());
    }

    #[test]
    fn test_update_check_state_save_and_load() -> Result<()> {
        let dir = tempdir()?;
        let state_path = dir.path().join("update-check-state.json");

        // Create and save state
        let state = UpdateCheckState::new("icg-v0.1.0".to_string(), "v0.1.0".to_string());
        state.save(&state_path)?;

        // Load it back
        let loaded = UpdateCheckState::load(&state_path)?.unwrap();
        assert_eq!(loaded.release_tag, "icg-v0.1.0");
        assert_eq!(loaded.trusted_ref, "v0.1.0");
        assert!(!loaded.last_successful_check.is_empty()); // Should have a timestamp

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
        let state = UpdateCheckState::new("icg-v0.1.0".to_string(), "v0.1.0".to_string());
        state.save(&nested_path)?;

        // Verify it exists and can be loaded
        let loaded = UpdateCheckState::load(&nested_path)?.unwrap();
        assert_eq!(loaded.release_tag, "icg-v0.1.0");

        Ok(())
    }
}
