//! Trust pointer mechanism (Layer 4 minimal form)
//!
//! Tracks a separately-advancing release reference -- what the fleet currently
//! trusts, distinct from bare "latest". This is the minimal form of Layer 4,
//! separable from how a host actually adopts a release (self-updater).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Trust pointer data structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustPointer {
    /// The currently trusted release reference
    /// This can be:
    /// - A git tag (e.g., "v0.1.0", "icg-v0.2.3")
    /// - A commit SHA (full 40-character or abbreviated)
    /// - A version identifier
    /// - A channel name (e.g., "stable", "canary")
    pub trusted_ref: String,

    /// When this trust pointer was last updated
    /// (ISO 8601 timestamp)
    #[serde(default)]
    pub updated_at: String,

    /// Optional: metadata about why this ref is trusted
    /// (e.g., which gate/check validated it)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub justification: Option<String>,
}

impl TrustPointer {
    /// Create a new trust pointer
    pub fn new(trusted_ref: impl Into<String>) -> Self {
        let trusted_ref = trusted_ref.into();
        Self {
            trusted_ref,
            updated_at: chrono::Utc::now().to_rfc3339(),
            justification: None,
        }
    }

    /// Create a new trust pointer with justification
    pub fn with_justification(
        trusted_ref: impl Into<String>,
        justification: impl Into<String>,
    ) -> Self {
        let mut pointer = Self::new(trusted_ref);
        pointer.justification = Some(justification.into());
        pointer
    }
}

/// Trust pointer storage manager
pub struct TrustPointerStore {
    /// Path to the trust pointer file
    path: PathBuf,
}

impl TrustPointerStore {
    /// Create a new trust pointer store
    ///
    /// The trust pointer file is stored in a root-owned system location:
    /// - Default: `/etc/icg/trust-pointer.json`
    /// - With channel: `/etc/icg/trust-pointer-<channel>.json` (e.g., `trust-pointer-canary.json`)
    /// - Or a custom path for testing/CI contexts
    ///
    /// See docs/plan/plan.md Architecture 'Deploy location' for security rationale
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    /// Create a trust pointer store for a specific channel
    ///
    /// This supports canary rollout patterns where different fleet segments
    /// track different release channels (e.g., "stable" vs "canary").
    ///
    /// # Arguments
    /// * `channel` - The channel name (e.g., "canary", "stable")
    ///
    /// # Returns
    /// A TrustPointerStore that reads from `/etc/icg/trust-pointer-<channel>.json`
    ///
    /// # Example
    /// ```no_run
    /// # use icg::trust_pointer::TrustPointerStore;
    /// // Canary channel worker (launched via NEEDLE --identifier canary-icg)
    /// let canary_store = TrustPointerStore::for_channel("canary");
    /// // Stable channel (default fleet)
    /// let stable_store = TrustPointerStore::for_channel("stable");
    /// ```
    pub fn for_channel(channel: &str) -> Self {
        let filename = format!("trust-pointer-{}.json", channel);
        let path = PathBuf::from("/etc/icg").join(filename);
        Self::new(path)
    }

    /// Verify that the artifact directory is secure
    ///
    /// This check ensures that:
    /// - The directory is owned by root (uid 0)
    /// - The directory is not world-writable
    /// - If not running as root, the directory is not writable by the current user
    ///
    /// This prevents the guarded agent from being able to modify its own
    /// trust configuration, which would reproduce the security gap that
    /// org-rule-guard.py has.
    ///
    /// Returns Ok(()) if the directory is secure, Err otherwise.
    /// For testing/CI contexts using custom paths, this check only warns
    /// rather than failing.
    pub fn verify_artifact_directory_security(&self) -> Result<()> {
        let artifact_dir = self.path
            .parent()
            .context("Trust pointer path has no parent directory")?;

        // If the directory doesn't exist yet, we can't verify security yet
        // This is expected during initial setup with sudo
        if !artifact_dir.exists() {
            return Ok(());
        }

        // Check directory metadata
        let metadata = fs::metadata(artifact_dir)
            .with_context(|| format!("Failed to read metadata for directory: {}", artifact_dir.display()))?;

        // Get ownership information
        let owner = metadata.uid();
        let perms = metadata.permissions().mode();

        // Check if owned by root
        if owner != 0 {
            // If we're using the default /etc/icg path, this is a security issue
            if artifact_dir == PathBuf::from("/etc/icg") {
                anyhow::bail!(
                    "Security violation: Artifact directory {} is NOT owned by root (owned by uid {}). \
                    This reproduces the self-edit gap that org-rule-guard.py has. \
                    Run: sudo chown root:root {}",
                    artifact_dir.display(),
                    owner,
                    artifact_dir.display()
                );
            } else {
                // For custom paths (testing/CI), just warn
                eprintln!(
                    "⚠️  Warning: Custom artifact directory {} is owned by uid {}, not root. \
                    This is acceptable for testing but NOT for production.",
                    artifact_dir.display(),
                    owner
                );
            }
        }

        // Check if world-writable (should not be)
        if perms & 0o002 != 0 {
            anyhow::bail!(
                "Security violation: Artifact directory {} is world-writable (mode {:o}). \
                This allows any user to modify trust configuration. \
                Run: sudo chmod o-w {}",
                artifact_dir.display(),
                perms,
                artifact_dir.display()
            );
        }

        // If not running as root, verify we don't have write access
        if std::env::var("USER").as_deref() != Ok("root") {
            // Try to create a temporary file in the directory
            let test_file = artifact_dir.join(".icg-security-test");
            match fs::write(&test_file, b"test") {
                Ok(_) => {
                    // We successfully wrote - this is a security issue for the default path
                    let _ = fs::remove_file(&test_file); // Clean up
                    if artifact_dir == PathBuf::from("/etc/icg") {
                        anyhow::bail!(
                            "Security violation: Current user can WRITE to artifact directory {}. \
                            This reproduces the self-edit gap that org-rule-guard.py has. \
                            The guarded agent must NOT be able to modify its own trust configuration. \
                            Fix the permissions or run as root to update.",
                            artifact_dir.display()
                        );
                    } else {
                        eprintln!(
                            "⚠️  Warning: Current user can write to custom artifact directory {}. \
                            This is acceptable for testing but NOT for production.",
                            artifact_dir.display()
                        );
                    }
                }
                Err(_) => {
                    // Write failed as expected - directory is secure from this user
                }
            }
        }

        Ok(())
    }

    /// Get the default trust pointer file path
    pub fn default_path() -> Result<PathBuf> {
        // Use root-owned system location, not user-writable path
        // See docs/plan/plan.md Architecture 'Deploy location'
        Ok(PathBuf::from("/etc/icg/trust-pointer.json"))
    }

    /// Get the path to the trust pointer file
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load the current trust pointer
    ///
    /// Returns None if the file doesn't exist yet
    pub fn load(&self) -> Result<Option<TrustPointer>> {
        // Verify security before reading
        self.verify_artifact_directory_security()?;

        if !self.path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&self.path)
            .with_context(|| format!("Failed to read trust pointer from {}", self.path.display()))?;

        let pointer: TrustPointer = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse trust pointer from {}", self.path.display()))?;

        Ok(Some(pointer))
    }

    /// Save a trust pointer
    pub fn save(&self, pointer: &TrustPointer) -> Result<()> {
        // Verify security before writing
        self.verify_artifact_directory_security()?;

        // Write to a temporary file first, then atomic rename
        let temp_path = self.path.with_extension("tmp");

        let content = serde_json::to_string_pretty(pointer)
            .context("Failed to serialize trust pointer")?;

        std::fs::write(&temp_path, content)
            .with_context(|| format!("Failed to write trust pointer to {}", temp_path.display()))?;

        // Atomic rename
        std::fs::rename(&temp_path, &self.path)
            .with_context(|| format!("Failed to rename trust pointer from {} to {}", temp_path.display(), self.path.display()))?;

        Ok(())
    }

    /// Get the current trusted reference
    ///
    /// Returns None if no trust pointer exists
    pub fn get_trusted_ref(&self) -> Result<Option<String>> {
        match self.load()? {
            Some(pointer) => Ok(Some(pointer.trusted_ref)),
            None => Ok(None),
        }
    }

    /// Set a new trusted reference
    pub fn set_trusted_ref(&self, trusted_ref: impl Into<String>) -> Result<()> {
        let pointer = TrustPointer::new(trusted_ref);
        self.save(&pointer)
    }

    /// Set a new trusted reference with justification
    pub fn set_trusted_ref_with_justification(
        &self,
        trusted_ref: impl Into<String>,
        justification: impl Into<String>,
    ) -> Result<()> {
        let pointer = TrustPointer::with_justification(trusted_ref, justification);
        self.save(&pointer)
    }

    /// Check if a given reference matches the trusted reference
    pub fn is_trusted(&self, reference: &str) -> Result<bool> {
        match self.get_trusted_ref()? {
            Some(trusted) => Ok(trusted == reference),
            None => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_trust_pointer_create() {
        let pointer = TrustPointer::new("v0.1.0");
        assert_eq!(pointer.trusted_ref, "v0.1.0");
        assert!(pointer.justification.is_none());
    }

    #[test]
    fn test_trust_pointer_with_justification() {
        let pointer = TrustPointer::with_justification("v0.1.0", "Passed Layer 1/2 gates");
        assert_eq!(pointer.trusted_ref, "v0.1.0");
        assert_eq!(pointer.justification, Some("Passed Layer 1/2 gates".to_string()));
    }

    #[test]
    fn test_store_save_and_load() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("trust-pointer.json");
        let store = TrustPointerStore::new(&path);

        // Initially no pointer
        assert!(store.load()?.is_none());

        // Save a pointer
        let pointer = TrustPointer::new("v0.2.0");
        store.save(&pointer)?;

        // Load it back
        let loaded = store.load()?.unwrap();
        assert_eq!(loaded.trusted_ref, "v0.2.0");

        Ok(())
    }

    #[test]
    fn test_store_get_trusted_ref() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("trust-pointer.json");
        let store = TrustPointerStore::new(&path);

        // Initially None
        assert!(store.get_trusted_ref()?.is_none());

        // Set a reference
        store.set_trusted_ref("v0.3.0")?;

        // Get it back
        assert_eq!(store.get_trusted_ref()?, Some("v0.3.0".to_string()));

        Ok(())
    }

    #[test]
    fn test_store_is_trusted() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("trust-pointer.json");
        let store = TrustPointerStore::new(&path);

        // Set trusted ref
        store.set_trusted_ref("abc123")?;

        // Check matching
        assert!(store.is_trusted("abc123")?);

        // Check non-matching
        assert!(!store.is_trusted("def456")?);

        Ok(())
    }

    #[test]
    fn test_atomic_write() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("trust-pointer.json");
        let store = TrustPointerStore::new(&path);

        // First write
        store.set_trusted_ref("v0.1.0")?;
        assert_eq!(store.get_trusted_ref()?, Some("v0.1.0".to_string()));

        // Second write (should atomic-replace, not corrupt)
        store.set_trusted_ref("v0.2.0")?;
        assert_eq!(store.get_trusted_ref()?, Some("v0.2.0".to_string()));

        Ok(())
    }

    #[test]
    fn test_for_channel_path() {
        let canary_store = TrustPointerStore::for_channel("canary");
        assert_eq!(
            canary_store.path,
            PathBuf::from("/etc/icg/trust-pointer-canary.json")
        );

        let stable_store = TrustPointerStore::for_channel("stable");
        assert_eq!(
            stable_store.path,
            PathBuf::from("/etc/icg/trust-pointer-stable.json")
        );

        let custom_store = TrustPointerStore::for_channel("beta");
        assert_eq!(
            custom_store.path,
            PathBuf::from("/etc/icg/trust-pointer-beta.json")
        );
    }

    #[test]
    fn test_channel_isolation() -> Result<()> {
        let dir = tempdir()?;

        // Create two separate channel stores
        let canary_path = dir.path().join("trust-pointer-canary.json");
        let stable_path = dir.path().join("trust-pointer-stable.json");

        let canary_store = TrustPointerStore::new(&canary_path);
        let stable_store = TrustPointerStore::new(&stable_path);

        // Set different refs for each channel
        canary_store.set_trusted_ref("v0.2.0-canary")?;
        stable_store.set_trusted_ref("v0.1.0-stable")?;

        // Verify they're isolated
        assert_eq!(
            canary_store.get_trusted_ref()?,
            Some("v0.2.0-canary".to_string())
        );
        assert_eq!(
            stable_store.get_trusted_ref()?,
            Some("v0.1.0-stable".to_string())
        );

        Ok(())
    }
}
