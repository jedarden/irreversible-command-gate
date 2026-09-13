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

/// A security violation detected in the directory that holds a trust pointer.
///
/// Detection is deliberately separate from enforcement.
/// `verify_artifact_directory_security` renders these exactly as it always
/// has -- a hard error for the production directory and for any world-writable
/// directory, a stderr warning for the remaining custom-path cases -- while
/// [`crate::rollback::check_and_rollback`] records every violation as crash
/// evidence in the state store the guarded process owns. That separation is
/// the fix for the nineteen-day blind spot: the check's rendered result is
/// routinely swallowed by its callers (the hook's `if let Ok(...)` trust load
/// drops it entirely, and the rollback boundary only prints it), so stderr
/// was never a durable record. Crash evidence is, and the operator's
/// `icg policy reconcile` turns it into a poison-pill policy event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactDirViolation {
    /// The directory is owned by a non-root uid.
    NotRootOwned { dir: PathBuf, owner: u32 },

    /// The directory is world-writable.
    WorldWritable { dir: PathBuf, mode: u32 },

    /// An unprivileged process can create files in the directory.
    UnprivilegedWrite { dir: PathBuf },
}

impl ArtifactDirViolation {
    /// Stable crash-evidence identifier for this violation.
    ///
    /// The identifier is a signature of the condition, not a unique event id:
    /// a violation that persists re-records the same id on every guarded
    /// invocation. The advancing crash counter proves the environment stayed
    /// insecure (and keeps reconciliation consuming it), while
    /// `PolicyState::record_poison_pill` deduplicates the policy event itself.
    pub fn crash_id(&self) -> String {
        let (kind, dir) = match self {
            Self::NotRootOwned { dir, .. } => ("not-root-owned", dir),
            Self::WorldWritable { dir, .. } => ("world-writable", dir),
            Self::UnprivilegedWrite { dir } => ("unprivileged-write", dir),
        };
        format!("artifact-dir-security:{kind}:{}", dir.display())
    }

    /// Whether this violation hard-fails the security check rather than
    /// warning. The production directory is administrator-owned by design, so
    /// every violation there is fatal; a world-writable directory is fatal
    /// everywhere.
    fn is_fatal(&self) -> bool {
        let production = match self {
            Self::NotRootOwned { dir, .. }
            | Self::WorldWritable { dir, .. }
            | Self::UnprivilegedWrite { dir } => dir == Path::new(PRODUCTION_ARTIFACT_DIR),
        };
        production || matches!(self, Self::WorldWritable { .. })
    }

    /// The operator-facing message. Fatal violations include the
    /// `Security violation:` prefix the hard error has always carried; the
    /// caller adds the `⚠️  Warning: ` prefix for the non-fatal ones.
    fn description(&self) -> String {
        match self {
            Self::NotRootOwned { dir, owner } => {
                if self.is_fatal() {
                    format!(
                        "Security violation: Artifact directory {} is NOT owned by root (owned by uid {}). \
                        This reproduces the self-edit gap that org-rule-guard.py has. \
                        Run: sudo chown root:root {}",
                        dir.display(),
                        owner,
                        dir.display()
                    )
                } else {
                    format!(
                        "Custom artifact directory {} is owned by uid {}, not root. \
                        This is acceptable for testing but NOT for production.",
                        dir.display(),
                        owner
                    )
                }
            }
            Self::WorldWritable { dir, mode } => format!(
                "Security violation: Artifact directory {} is world-writable (mode {:o}). \
                This allows any user to modify trust configuration. \
                Run: sudo chmod o-w {}",
                dir.display(),
                mode,
                dir.display()
            ),
            Self::UnprivilegedWrite { dir } => {
                if self.is_fatal() {
                    format!(
                        "Security violation: Current user can WRITE to artifact directory {}. \
                        This reproduces the self-edit gap that org-rule-guard.py has. \
                        The guarded agent must NOT be able to modify its own trust configuration. \
                        Fix the permissions or run as root to update.",
                        dir.display()
                    )
                } else {
                    format!(
                        "Current user can write to custom artifact directory {}. \
                        This is acceptable for testing but NOT for production.",
                        dir.display()
                    )
                }
            }
        }
    }
}

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
    /// rather than failing -- but every detected violation is also
    /// classifiable through [`Self::detect_artifact_directory_violations`],
    /// and the guarded boundaries record that classification as crash
    /// evidence, so a swallowed warning still reaches policy reconciliation.
    pub fn verify_artifact_directory_security(&self) -> Result<()> {
        // Safe: `geteuid` is a plain syscall wrapper reading the caller's
        // effective uid; it touches no memory and cannot fail.
        self.verify_artifact_directory_security_with_euid(unsafe { libc::geteuid() })
    }

    /// The same checks with the effective uid supplied by the caller.
    fn verify_artifact_directory_security_with_euid(&self, euid: u32) -> Result<()> {
        for violation in self.detect_artifact_directory_violations_with_euid(euid)? {
            if violation.is_fatal() {
                anyhow::bail!("{}", violation.description());
            }
            eprintln!("⚠️  Warning: {}", violation.description());
        }
        Ok(())
    }

    /// Classify every security violation of the directory holding this
    /// store's trust pointer, in check order, without warning or failing.
    ///
    /// This is the detection half of
    /// [`Self::verify_artifact_directory_security`], kept in one function so
    /// the rendered result and the recorded evidence can never drift apart.
    /// The guarded boundaries call it to record durable crash evidence; see
    /// [`ArtifactDirViolation`] for why the rendered result alone is not a
    /// record.
    pub fn detect_artifact_directory_violations(&self) -> Result<Vec<ArtifactDirViolation>> {
        // Safe: `geteuid` is a plain syscall wrapper; see the production call
        // site above.
        self.detect_artifact_directory_violations_with_euid(unsafe { libc::geteuid() })
    }

    /// The same classification with the effective uid supplied by the caller.
    ///
    /// Production always passes the real effective uid. The parameter exists
    /// so tests can drive the root and unprivileged branches without changing
    /// the real uid: a process cannot grant itself uid 0, and this suite has
    /// to pass both in root CI containers and in unprivileged checkouts.
    fn detect_artifact_directory_violations_with_euid(
        &self,
        euid: u32,
    ) -> Result<Vec<ArtifactDirViolation>> {
        let artifact_dir = self
            .path
            .parent()
            .context("Trust pointer path has no parent directory")?;

        // If the directory doesn't exist yet, we can't verify security yet
        // This is expected during initial setup with sudo
        if !artifact_dir.exists() {
            return Ok(Vec::new());
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

        let mut violations = Vec::new();

        // Check if owned by root
        if owner != 0 {
            violations.push(ArtifactDirViolation::NotRootOwned {
                dir: artifact_dir.to_path_buf(),
                owner,
            });
        }

        // Check if world-writable (should not be)
        if perms & 0o002 != 0 {
            violations.push(ArtifactDirViolation::WorldWritable {
                dir: artifact_dir.to_path_buf(),
                mode: perms,
            });
        }

        // Root can write anywhere, so for root the probe is meaningless: it
        // would trivially succeed and then fail a correctly secured directory.
        // Root is detected by effective uid, never the USER environment
        // variable -- a container running as root normally has USER unset
        // (nothing login-shaped sets it), which made every guarded CI pod
        // look unprivileged here (irrevers-beee1069).
        if euid == 0 {
            return Ok(violations);
        }

        let can_write = Self::probe_write_access(artifact_dir);
        if let Some(violation) = Self::unprivileged_write_violation(artifact_dir, can_write) {
            violations.push(violation);
        }

        Ok(violations)
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
    /// probe entirely, since it can write any directory it can see. Kept as a
    /// pure function of the probe result so tests can drive the production
    /// and custom branches without touching the real `/etc/icg`.
    fn unprivileged_write_violation(
        artifact_dir: &Path,
        can_write: bool,
    ) -> Option<ArtifactDirViolation> {
        if !can_write {
            // Write failed as expected - directory is secure from this user
            return None;
        }

        // We successfully wrote - this is a security issue
        Some(ArtifactDirViolation::UnprivilegedWrite {
            dir: artifact_dir.to_path_buf(),
        })
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
        let violation = TrustPointerStore::unprivileged_write_violation(
            Path::new(PRODUCTION_ARTIFACT_DIR),
            true,
        )
        .expect("write access to the production directory must classify as a violation");

        assert!(
            violation.is_fatal(),
            "write access to the production directory must fail the check"
        );
        let description = violation.description();
        assert!(
            description.contains("can WRITE"),
            "unexpected message: {description}"
        );
    }

    #[test]
    fn unprivileged_without_write_access_is_accepted() {
        assert!(
            TrustPointerStore::unprivileged_write_violation(
                Path::new(PRODUCTION_ARTIFACT_DIR),
                false,
            )
            .is_none(),
            "a directory this user cannot write is secure"
        );
    }

    /// Unchanged behaviour: outside the production directory a writable
    /// directory is a warning, not a failure -- the testing/CI contexts rely
    /// on it. It is still a violation, though: the guarded boundaries record
    /// it as crash evidence even though the rendered check only warns, which
    /// is what keeps a custom-path bypass from being invisible to policy
    /// reconciliation (irrevers-91694e78).
    #[test]
    fn unprivileged_write_to_a_custom_directory_warns_but_is_still_a_violation() {
        let violation =
            TrustPointerStore::unprivileged_write_violation(Path::new("/var/tmp/icg-test"), true)
                .expect("custom artifact directories still classify write access as a violation");

        assert!(
            !violation.is_fatal(),
            "custom artifact directories must warn rather than fail"
        );
        let description = violation.description();
        assert!(
            !description.contains("Security violation:"),
            "a non-fatal classification keeps the warning shape: {description}"
        );
    }

    /// Crash evidence identifies the condition, not the event: the same
    /// standing violation re-records the same id, and policy reconciliation
    /// deduplicates on the advancing counter instead.
    #[test]
    fn crash_ids_are_stable_signatures_of_the_condition() {
        let dir = PathBuf::from("/etc/icg");
        assert_eq!(
            ArtifactDirViolation::WorldWritable {
                dir: dir.clone(),
                mode: 0o777
            }
            .crash_id(),
            "artifact-dir-security:world-writable:/etc/icg"
        );
        assert_eq!(
            ArtifactDirViolation::NotRootOwned {
                dir: dir.clone(),
                owner: 1000
            }
            .crash_id(),
            "artifact-dir-security:not-root-owned:/etc/icg"
        );
        assert_eq!(
            ArtifactDirViolation::UnprivilegedWrite { dir }.crash_id(),
            "artifact-dir-security:unprivileged-write:/etc/icg"
        );
    }

    /// Fatality must keep mirroring the rendered check exactly: everything
    /// about the production directory is fatal, world-writability is fatal
    /// wherever it lives, and the remaining custom-path conditions only warn.
    #[test]
    fn fatality_follows_the_rendered_check_rules() {
        let production = PathBuf::from(PRODUCTION_ARTIFACT_DIR);
        let custom = PathBuf::from("/var/tmp/icg-test");

        assert!(ArtifactDirViolation::NotRootOwned {
            dir: production.clone(),
            owner: 1000
        }
        .is_fatal());
        assert!(ArtifactDirViolation::UnprivilegedWrite {
            dir: production.clone()
        }
        .is_fatal());
        assert!(ArtifactDirViolation::WorldWritable {
            dir: custom.clone(),
            mode: 0o777
        }
        .is_fatal());
        assert!(!ArtifactDirViolation::NotRootOwned {
            dir: custom.clone(),
            owner: 1000
        }
        .is_fatal());
        assert!(!ArtifactDirViolation::UnprivilegedWrite { dir: custom }.is_fatal());
    }

    /// A world-writable directory is classified wherever it lives -- the
    /// condition that sat unrecorded on guarded CI pods for nineteen days
    /// (irrevers-beee1069). Metadata violations do not depend on the
    /// effective uid, so the classification is runner-independent; a
    /// non-root-owned fixture merely adds the not-root-owned condition
    /// alongside it.
    #[test]
    fn world_writable_directory_is_classified_as_a_violation() -> Result<()> {
        let dir = writable_tempdir()?;
        let mut permissions = fs::metadata(dir.path())?.permissions();
        permissions.set_mode(0o777);
        fs::set_permissions(dir.path(), permissions)?;
        let store = TrustPointerStore::new(dir.path().join("trust-pointer.json"));

        let violations = store.detect_artifact_directory_violations_with_euid(0)?;

        // The reported mode is stat's full st_mode -- file-type bits
        // included -- so the expectation reads it back from the same source
        // instead of hardcoding bare permission bits.
        let mode = fs::metadata(dir.path())?.permissions().mode();
        let expected = ArtifactDirViolation::WorldWritable {
            dir: dir.path().to_path_buf(),
            mode,
        };
        assert!(
            violations.contains(&expected),
            "the world-writable condition must be classified: {violations:?}"
        );
        assert!(
            mode & 0o002 != 0,
            "the fixture must still be world-writable: {mode:o}"
        );
        assert!(
            expected.is_fatal(),
            "world-writability must be fatal everywhere"
        );
        Ok(())
    }

    /// The unprivileged branch classifies write access in addition to any
    /// metadata conditions, and the write probe runs for it.
    #[test]
    fn unprivileged_euid_classifies_write_access_as_a_violation() -> Result<()> {
        let dir = writable_tempdir()?;
        let sentinel = plant_probe_sentinel(dir.path())?;
        let store = TrustPointerStore::new(dir.path().join("trust-pointer.json"));

        let violations = store.detect_artifact_directory_violations_with_euid(1000)?;

        assert_eq!(
            violations.last().map(|violation| violation.crash_id()),
            Some(format!(
                "artifact-dir-security:unprivileged-write:{}",
                dir.path().display()
            )),
            "the probe result is classified after the metadata conditions: {violations:?}"
        );
        assert!(
            !sentinel.exists(),
            "the write probe should have written and removed the sentinel"
        );
        Ok(())
    }

    /// The root branch never runs the probe, so it can never classify an
    /// unprivileged-write condition -- whatever `USER` claims.
    #[test]
    fn root_euid_detection_skips_the_write_probe() -> Result<()> {
        let dir = writable_tempdir()?;
        let sentinel = plant_probe_sentinel(dir.path())?;
        let store = TrustPointerStore::new(dir.path().join("trust-pointer.json"));

        let _user = UserEnvGuard::set_to("nobody");
        let violations = store.detect_artifact_directory_violations_with_euid(0)?;

        assert!(
            !violations.iter().any(|violation| matches!(
                violation,
                ArtifactDirViolation::UnprivilegedWrite { .. }
            )),
            "root must not classify a write probe it never ran: {violations:?}"
        );
        assert_eq!(fs::read(&sentinel)?, b"untouched");
        Ok(())
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
