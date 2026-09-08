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

/// The directory the fleet's trust pointers live in when deployed for real.
///
/// The write-probe check treats unprivileged write access here as a hard
/// violation rather than a warning; one constant keeps the comparison and the
/// two constructors from drifting apart.
const PRODUCTION_ARTIFACT_DIR: &str = "/etc/icg";

/// The file the write probe creates to learn whether it can write a directory.
const PROBE_FILE_NAME: &str = ".icg-security-test";

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
        let path = PathBuf::from(PRODUCTION_ARTIFACT_DIR).join(filename);
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
        // Safe: `geteuid` is a plain syscall wrapper reading the caller's
        // effective uid; it touches no memory and cannot fail.
        self.verify_artifact_directory_security_with_euid(unsafe { libc::geteuid() })
    }

    /// The same checks with the effective uid supplied by the caller.
    ///
    /// Production always passes the real effective uid. The parameter exists
    /// so tests can drive the root and unprivileged branches without changing
    /// the real uid: a process cannot grant itself uid 0, and this suite has
    /// to pass both in root CI containers and in unprivileged checkouts.
    fn verify_artifact_directory_security_with_euid(&self, euid: u32) -> Result<()> {
        let artifact_dir = self
            .path
            .parent()
            .context("Trust pointer path has no parent directory")?;

        // If the directory doesn't exist yet, we can't verify security yet
        // This is expected during initial setup with sudo
        if !artifact_dir.exists() {
            return Ok(());
        }

        // Check directory metadata
        let metadata = fs::metadata(artifact_dir).with_context(|| {
            format!(
                "Failed to read metadata for directory: {}",
                artifact_dir.display()
            )
        })?;

        // Get ownership information
        let owner = metadata.uid();
        let perms = metadata.permissions().mode();

        // Check if owned by root
        if owner != 0 {
            // If we're using the default /etc/icg path, this is a security issue
            if artifact_dir == Path::new(PRODUCTION_ARTIFACT_DIR) {
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

        // Root can write anywhere, so for root the probe is meaningless: it
        // would trivially succeed and then fail a correctly secured directory.
        // Root is detected by effective uid, never the USER environment
        // variable -- a container running as root normally has USER unset
        // (nothing login-shaped sets it), which made every guarded CI pod
        // look unprivileged here (irrevers-beee1069).
        if euid == 0 {
            return Ok(());
        }

        let can_write = Self::probe_write_access(artifact_dir);
        Self::check_unprivileged_write_access(artifact_dir, can_write)
    }

    /// Learn whether this process can write `directory` by creating and
    /// removing a probe file in it.
    fn probe_write_access(directory: &Path) -> bool {
        let test_file = directory.join(PROBE_FILE_NAME);
        match fs::write(&test_file, b"test") {
            Ok(_) => {
                let _ = fs::remove_file(&test_file); // Clean up
                true
            }
            Err(_) => false,
        }
    }

    /// Interpret an unprivileged process's write access to `artifact_dir`.
    ///
    /// Callers reach this only for a non-zero effective uid -- root skips the
    /// probe entirely, since it can write any directory it can see.
    fn check_unprivileged_write_access(artifact_dir: &Path, can_write: bool) -> Result<()> {
        if !can_write {
            // Write failed as expected - directory is secure from this user
            return Ok(());
        }

        // We successfully wrote - this is a security issue for the default path
        if artifact_dir == Path::new(PRODUCTION_ARTIFACT_DIR) {
            anyhow::bail!(
                "Security violation: Current user can WRITE to artifact directory {}. \
                This reproduces the self-edit gap that org-rule-guard.py has. \
                The guarded agent must NOT be able to modify its own trust configuration. \
                Fix the permissions or run as root to update.",
                artifact_dir.display()
            );
        }

        // For custom paths (testing/CI), just warn
        eprintln!(
            "⚠️  Warning: Current user can write to custom artifact directory {}. \
            This is acceptable for testing but NOT for production.",
            artifact_dir.display()
        );
        Ok(())
    }

    /// Get the default trust pointer file path
    pub fn default_path() -> Result<PathBuf> {
        // Use root-owned system location, not user-writable path
        // See docs/plan/plan.md Architecture 'Deploy location'
        Ok(PathBuf::from(PRODUCTION_ARTIFACT_DIR).join("trust-pointer.json"))
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

        let content = std::fs::read_to_string(&self.path).with_context(|| {
            format!("Failed to read trust pointer from {}", self.path.display())
        })?;

        let pointer: TrustPointer = serde_json::from_str(&content).with_context(|| {
            format!("Failed to parse trust pointer from {}", self.path.display())
        })?;

        Ok(Some(pointer))
    }

    /// Save a trust pointer
    pub fn save(&self, pointer: &TrustPointer) -> Result<()> {
        // Verify security before writing
        self.verify_artifact_directory_security()?;

        // Write to a temporary file first, then atomic rename
        let temp_path = self.path.with_extension("tmp");

        let content =
            serde_json::to_string_pretty(pointer).context("Failed to serialize trust pointer")?;

        std::fs::write(&temp_path, content)
            .with_context(|| format!("Failed to write trust pointer to {}", temp_path.display()))?;

        // Atomic rename
        std::fs::rename(&temp_path, &self.path).with_context(|| {
            format!(
                "Failed to rename trust pointer from {} to {}",
                temp_path.display(),
                self.path.display()
            )
        })?;

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

    fn secure_tempdir() -> Result<tempfile::TempDir> {
        let directory = tempdir()?;
        let mut permissions = fs::metadata(directory.path())?.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(directory.path(), permissions)?;
        Ok(directory)
    }

    /// A directory the current process can write, at a mode that passes the
    /// ownership and world-writable checks (0755, not world-writable).
    ///
    /// Unlike `secure_tempdir`, this leaves the write probe a chance to
    /// succeed, which is what the unprivileged branch looks for.
    fn writable_tempdir() -> Result<tempfile::TempDir> {
        let directory = tempdir()?;
        let mut permissions = fs::metadata(directory.path())?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(directory.path(), permissions)?;
        Ok(directory)
    }

    /// Plant a file where the write probe would put its own, so a test can
    /// tell "the probe ran and consumed it" from "the probe never ran".
    fn plant_probe_sentinel(directory: &Path) -> Result<PathBuf> {
        let sentinel = directory.join(PROBE_FILE_NAME);
        fs::write(&sentinel, b"untouched")?;
        Ok(sentinel)
    }

    /// Temporarily point `USER` where the test wants it, restoring the
    /// ambient value on drop.
    ///
    /// `USER` is process-global and the test binary runs its tests on
    /// parallel threads, so a guard is what keeps a panic from leaking a
    /// mutated value into unrelated tests -- `denial_log` reads `USER` for
    /// attribution.
    struct UserEnvGuard {
        saved: Option<String>,
    }

    impl UserEnvGuard {
        fn unset() -> Self {
            let saved = std::env::var("USER").ok();
            std::env::remove_var("USER");
            Self { saved }
        }

        fn set_to(value: &str) -> Self {
            let saved = std::env::var("USER").ok();
            std::env::set_var("USER", value);
            Self { saved }
        }
    }

    impl Drop for UserEnvGuard {
        fn drop(&mut self) {
            match &self.saved {
                Some(value) => std::env::set_var("USER", value),
                None => std::env::remove_var("USER"),
            }
        }
    }

    /// The CI-pod regression: a root process, whatever `USER` says.
    ///
    /// A container running as root normally has `USER` unset -- nothing
    /// login-shaped sets it -- and the old env-var check treated that as
    /// unprivileged, ran the write probe, and failed a correctly secured
    /// directory on every hook invocation (irrevers-beee1069). The sentinel
    /// measures probe-skip rather than a bare `Ok`, so these tests fail if
    /// the probe runs for root under any `USER` value.
    #[test]
    fn root_euid_skips_the_write_probe_when_user_is_unset() -> Result<()> {
        let dir = writable_tempdir()?;
        let sentinel = plant_probe_sentinel(dir.path())?;
        let store = TrustPointerStore::new(dir.path().join("trust-pointer.json"));

        let _user = UserEnvGuard::unset();
        store.verify_artifact_directory_security_with_euid(0)?;

        assert_eq!(fs::read(&sentinel)?, b"untouched");
        Ok(())
    }

    #[test]
    fn root_euid_skips_the_write_probe_when_user_says_root() -> Result<()> {
        let dir = writable_tempdir()?;
        let sentinel = plant_probe_sentinel(dir.path())?;
        let store = TrustPointerStore::new(dir.path().join("trust-pointer.json"));

        let _user = UserEnvGuard::set_to("root");
        store.verify_artifact_directory_security_with_euid(0)?;

        assert_eq!(fs::read(&sentinel)?, b"untouched");
        Ok(())
    }

    /// The sharpest pin on the detection rule: `USER` claiming unprivileged
    /// must not override an effective uid of 0.
    #[test]
    fn root_euid_wins_over_a_non_root_user_env_value() -> Result<()> {
        let dir = writable_tempdir()?;
        let sentinel = plant_probe_sentinel(dir.path())?;
        let store = TrustPointerStore::new(dir.path().join("trust-pointer.json"));

        let _user = UserEnvGuard::set_to("nobody");
        store.verify_artifact_directory_security_with_euid(0)?;

        assert_eq!(fs::read(&sentinel)?, b"untouched");
        Ok(())
    }

    /// End-to-end through the public entry point, on a runner that really is
    /// root: the real `geteuid()` wiring must land in the same skip.
    #[test]
    fn root_process_passes_a_secured_directory_with_user_unset() -> Result<()> {
        // Safe: a plain syscall wrapper; see the production call site.
        if unsafe { libc::geteuid() } != 0 {
            eprintln!("skip: the runner is not root, so the real-euid branch cannot be reached");
            return Ok(());
        }

        let dir = writable_tempdir()?;
        let sentinel = plant_probe_sentinel(dir.path())?;
        let store = TrustPointerStore::new(dir.path().join("trust-pointer.json"));

        let _user = UserEnvGuard::unset();
        store.verify_artifact_directory_security()?;

        assert_eq!(fs::read(&sentinel)?, b"untouched");
        Ok(())
    }

    /// An unprivileged uid runs the probe. The probe writes with the real
    /// identity, and the test owns the directory, so this holds for whichever
    /// user runs the suite. A successful probe removes its own file, so
    /// "ran" is observed as the sentinel being consumed.
    #[test]
    fn unprivileged_euid_runs_the_write_probe() -> Result<()> {
        let dir = writable_tempdir()?;
        let sentinel = plant_probe_sentinel(dir.path())?;
        let store = TrustPointerStore::new(dir.path().join("trust-pointer.json"));

        store.verify_artifact_directory_security_with_euid(1000)?;

        assert!(
            !sentinel.exists(),
            "the write probe should have written and removed the sentinel"
        );
        Ok(())
    }

    /// An unprivileged caller facing a directory it genuinely cannot write is
    /// accepted. Root writes through DAC-closed directories, so an unwritable
    /// tempdir only exists on a non-root runner.
    #[test]
    fn unprivileged_euid_against_an_unwritable_directory_is_accepted() -> Result<()> {
        // Safe: a plain syscall wrapper; see the production call site.
        if unsafe { libc::geteuid() } == 0 {
            eprintln!("skip: running as root, so no directory is unwritable to probe against");
            return Ok(());
        }

        let dir = secure_tempdir()?;
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555))?;
        let store = TrustPointerStore::new(dir.path().join("trust-pointer.json"));

        store.verify_artifact_directory_security_with_euid(1000)?;

        assert!(
            !dir.path().join(PROBE_FILE_NAME).exists(),
            "a failed probe must not leave its file behind"
        );
        Ok(())
    }

    #[test]
    fn unprivileged_write_to_the_production_directory_is_a_violation() {
        let error = TrustPointerStore::check_unprivileged_write_access(
            Path::new(PRODUCTION_ARTIFACT_DIR),
            true,
        )
        .expect_err("write access to the production directory must fail the check");

        assert!(
            error.to_string().contains("can WRITE"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn unprivileged_without_write_access_is_accepted() {
        TrustPointerStore::check_unprivileged_write_access(
            Path::new(PRODUCTION_ARTIFACT_DIR),
            false,
        )
        .expect("a directory this user cannot write is secure");
    }

    /// Unchanged behaviour: outside the production directory a writable
    /// directory is a warning, not a failure -- the testing/CI contexts rely
    /// on it.
    #[test]
    fn unprivileged_write_to_a_custom_directory_is_accepted_with_a_warning() {
        TrustPointerStore::check_unprivileged_write_access(Path::new("/var/tmp/icg-test"), true)
            .expect("custom artifact directories only warn on write access");
    }

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
        assert_eq!(
            pointer.justification,
            Some("Passed Layer 1/2 gates".to_string())
        );
    }

    #[test]
    fn test_store_save_and_load() -> Result<()> {
        let dir = secure_tempdir()?;
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
        let dir = secure_tempdir()?;
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
        let dir = secure_tempdir()?;
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
        let dir = secure_tempdir()?;
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
        let dir = secure_tempdir()?;

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
