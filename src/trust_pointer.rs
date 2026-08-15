//! Trust pointer mechanism (Layer 4 minimal form)
//!
//! Tracks a separately-advancing release reference -- what the fleet currently
//! trusts, distinct from bare "latest". This is the minimal form of Layer 4,
//! separable from how a host actually adopts a release (self-updater).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
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
    /// The trust pointer file is typically stored in:
    /// - `$XDG_CONFIG_HOME/icg/trust-pointer.json`
    /// - `$HOME/.config/icg/trust-pointer.json` (fallback)
    /// - Or a custom path for testing/CI contexts
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    /// Get the default trust pointer file path
    pub fn default_path() -> Result<PathBuf> {
        let config_dir = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|_| {
                let home = std::env::var("HOME")
                    .context("HOME environment variable not set")?;
                Ok::<PathBuf, anyhow::Error>(PathBuf::from(home).join(".config"))
            })?;

        let dir = config_dir.join("icg");

        // Ensure directory exists
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("Failed to create config directory: {}", dir.display()))?;

        Ok(dir.join("trust-pointer.json"))
    }

    /// Load the current trust pointer
    ///
    /// Returns None if the file doesn't exist yet
    pub fn load(&self) -> Result<Option<TrustPointer>> {
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
}
