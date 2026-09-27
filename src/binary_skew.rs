//! Binary-versus-trusted-release skew reporting.
//!
//! The self-updater advances the rule-pack artifact and reads the trust
//! pointer; it deliberately never writes to the root-owned binary
//! (`/usr/local/bin/icg`), so a host's executable can drift from the
//! release its own trust pointer names: packs advance, the binary stays
//! put, and nothing reported the gap — the binary's version did not
//! appear anywhere in `icg status`. This module is the comparison: the
//! running binary's version against the pointer's trusted reference,
//! rendered as the first-class **Binary Version** section of `icg status`
//! with the sanctioned remedy named for each direction of the skew.

use std::cmp::Ordering;

/// The version of the binary this code was compiled into.
pub fn running_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// A release reference parsed into a comparable version triple.
///
/// Accepts the tag shapes the trust pointer carries — `v0.1.71`,
/// `0.1.71`, `v0.1`, `1`. Commit SHAs, channel names, prerelease
/// suffixes and anything else are not versions:
/// [`ReleaseVersion::parse`] returns `None` for them, which the
/// comparison reports as [`BinarySkew::NotComparable`] rather than
/// guessing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseVersion {
    major: u64,
    minor: u64,
    patch: u64,
}

impl ReleaseVersion {
    /// Parse a release reference into a version triple.
    pub fn parse(reference: &str) -> Option<Self> {
        let reference = reference.trim();
        let reference = reference.strip_prefix(['v', 'V']).unwrap_or(reference);
        if reference.is_empty() || reference.chars().any(|c| c.is_whitespace()) {
            return None;
        }
        let mut parts = [0u64; 3];
        for (count, component) in reference.split('.').enumerate() {
            if component.is_empty() || !component.bytes().all(|b| b.is_ascii_digit()) || count == 3
            {
                return None;
            }
            parts[count] = component.parse().ok()?;
        }
        Some(Self {
            major: parts[0],
            minor: parts[1],
            patch: parts[2],
        })
    }

    /// The `(major, minor, patch)` triple.
    pub fn parts(&self) -> (u64, u64, u64) {
        (self.major, self.minor, self.patch)
    }
}

impl std::fmt::Display for ReleaseVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "v{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The skew between the running binary and the trust pointer's trusted
/// reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinarySkew {
    /// The binary's version equals the trusted release's version.
    InSync,
    /// The running binary is older than the trusted release: the packs
    /// may have advanced past what this binary understands.
    BinaryOlder {
        binary: ReleaseVersion,
        trusted: ReleaseVersion,
    },
    /// The running binary is newer than the trusted release: the pointer
    /// lags the deployed executable.
    BinaryNewer {
        binary: ReleaseVersion,
        trusted: ReleaseVersion,
    },
    /// The trusted reference is not a version (commit SHA, channel name,
    /// prerelease tag), so no comparison is possible.
    NotComparable,
    /// No trust pointer is configured.
    NoTrustPointer,
}

/// Compare a binary version against the pointer's trusted reference.
pub fn assess(binary_version: &str, trusted_ref: Option<&str>) -> BinarySkew {
    let Some(trusted_ref) = trusted_ref else {
        return BinarySkew::NoTrustPointer;
    };
    let (Some(binary), Some(trusted)) = (
        ReleaseVersion::parse(binary_version),
        ReleaseVersion::parse(trusted_ref),
    ) else {
        return BinarySkew::NotComparable;
    };
    match binary.parts().cmp(&trusted.parts()) {
        Ordering::Equal => BinarySkew::InSync,
        Ordering::Less => BinarySkew::BinaryOlder { binary, trusted },
        Ordering::Greater => BinarySkew::BinaryNewer { binary, trusted },
    }
}

/// The value of `icg status`'s **Binary Skew** field.
pub fn skew_summary(skew: &BinarySkew, binary_version: &str, trusted_ref: Option<&str>) -> String {
    match skew {
        BinarySkew::InSync => format!(
            "in sync — running binary v{binary_version} matches trusted release {}",
            trusted_ref.unwrap_or_default()
        ),
        BinarySkew::BinaryOlder { binary, trusted } => format!(
            "SKEWED — running binary {binary} is older than trusted release \
             {trusted}; rule packs may have advanced past what this binary \
             enforces"
        ),
        BinarySkew::BinaryNewer { binary, trusted } => format!(
            "SKEWED — running binary {binary} is newer than trusted release \
             {trusted}; the trust pointer lags the deployed executable"
        ),
        BinarySkew::NotComparable => format!(
            "unknown — trusted reference {} is not a release version, so it \
             cannot be compared to binary v{binary_version}",
            trusted_ref
                .map(|r| format!("`{r}`"))
                .unwrap_or_else(|| "(none)".to_string())
        ),
        BinarySkew::NoTrustPointer => {
            "unknown — no trust pointer is configured, so there is no release \
             to compare against"
                .to_string()
        }
    }
}

/// The lines of `icg status`'s **Binary Version** section.
pub fn status_lines(binary_version: &str, trusted_ref: Option<&str>) -> Vec<String> {
    let skew = assess(binary_version, trusted_ref);
    let mut lines = vec![
        format!("  **Running Binary:** v{binary_version}"),
        format!(
            "  **Trusted Release:** {}",
            trusted_ref
                .map(|r| format!("`{r}`"))
                .unwrap_or_else(|| "(not configured)".to_string())
        ),
        format!(
            "  **Binary Skew:** {}",
            skew_summary(&skew, binary_version, trusted_ref)
        ),
    ];
    match skew {
        BinarySkew::BinaryOlder { .. } => {
            lines.push(
                "  Upgrade the executable per docs/operators/deployment-guide.md".to_string(),
            );
            lines.push(
                "  (\"Upgrade the executable from source\"); `sudo icg update` \
                 never upgrades it."
                    .to_string(),
            );
        }
        BinarySkew::BinaryNewer { .. } => {
            lines.push("  Advance the pointer only through a release record per".to_string());
            lines.push("  docs/runbooks/release-cutting.md.".to_string());
        }
        BinarySkew::InSync | BinarySkew::NotComparable | BinarySkew::NoTrustPointer => {}
    }
    lines
}

/// Print the **Binary Version** section of `icg status`.
pub fn print_status_section(trusted_ref: Option<&str>) {
    for line in status_lines(running_version(), trusted_ref) {
        println!("{line}");
    }
}
