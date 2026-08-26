//! Cryptographically verifiable manifests for the modular release pack asset.
//!
//! The manifest binds release CI to the exact `packs/*.json` bytes that will
//! be placed in `icg-packs.tar.gz`. It records byte hashes as well as parsed
//! metadata, so a pack that still parses but has changed after review fails.

use crate::rule_pack::Pack;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Version of the pack manifest format.
pub const MANIFEST_VERSION: &str = "v1";

/// Deterministic index of a modular rule-pack directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackManifest {
    pub version: String,
    pub generated_at: String,
    pub pack_count: usize,
    pub aggregate_stats: AggregateStats,
    /// Entries are keyed and serialized in pack-ID order.
    pub packs: BTreeMap<String, PackEntry>,
}

/// Aggregate metadata for all indexed pack files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateStats {
    pub total_safe_patterns: usize,
    pub total_guarded_patterns: usize,
    pub total_bytes: u64,
    pub pack_ids: Vec<String>,
}

/// The metadata and byte hash for one pack file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackEntry {
    pub id: String,
    pub relative_path: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub safe_pattern_count: usize,
    pub guarded_pattern_count: usize,
    #[serde(default)]
    pub tool_keywords: Vec<String>,
    #[serde(default)]
    pub applies_to: Vec<String>,
}

impl PackManifest {
    /// Generate an index for every immediate JSON file in `packs_dir`.
    pub fn from_dir<P: AsRef<Path>>(packs_dir: P) -> Result<Self> {
        let packs_dir = packs_dir.as_ref();
        let mut paths = fs::read_dir(packs_dir)
            .with_context(|| format!("failed to read packs directory: {}", packs_dir.display()))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?
            .into_iter()
            .filter(|path| path.is_file())
            .filter(|path| {
                path.extension().and_then(|extension| extension.to_str()) == Some("json")
            })
            .collect::<Vec<_>>();
        paths.sort();

        if paths.is_empty() {
            anyhow::bail!("no pack files found in directory: {}", packs_dir.display());
        }

        let mut packs = BTreeMap::new();
        let mut total_safe_patterns = 0;
        let mut total_guarded_patterns = 0;
        let mut total_bytes = 0_u64;

        for path in paths {
            let contents = fs::read(&path)
                .with_context(|| format!("failed to read pack file: {}", path.display()))?;
            let pack: Pack = serde_json::from_slice(&contents)
                .with_context(|| format!("failed to parse pack JSON: {}", path.display()))?;
            let filename = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            let expected_filename = format!("{}.json", pack.id);
            if filename != expected_filename {
                anyhow::bail!(
                    "pack ID '{}' does not match filename '{}'; expected '{}'",
                    pack.id,
                    filename,
                    expected_filename
                );
            }

            let size_bytes = contents.len() as u64;
            let entry = PackEntry {
                id: pack.id.clone(),
                relative_path: path
                    .strip_prefix(packs_dir)
                    .expect("pack path was read from its pack directory")
                    .display()
                    .to_string(),
                sha256: sha256_hex(&contents),
                size_bytes,
                safe_pattern_count: pack.safe_patterns.len(),
                guarded_pattern_count: pack.guarded_patterns.len(),
                tool_keywords: pack.tool_keywords.clone(),
                applies_to: pack.applies_to.clone(),
            };

            if packs.insert(pack.id.clone(), entry).is_some() {
                anyhow::bail!("duplicate pack ID '{}' in {}", pack.id, packs_dir.display());
            }
            total_safe_patterns += pack.safe_patterns.len();
            total_guarded_patterns += pack.guarded_patterns.len();
            total_bytes += size_bytes;
        }

        let pack_ids = packs.keys().cloned().collect::<Vec<_>>();
        Ok(Self {
            version: MANIFEST_VERSION.to_string(),
            generated_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            pack_count: packs.len(),
            aggregate_stats: AggregateStats {
                total_safe_patterns,
                total_guarded_patterns,
                total_bytes,
                pack_ids,
            },
            packs,
        })
    }

    /// Verify that every indexed file is unchanged and no JSON pack was added.
    pub fn verify_dir<P: AsRef<Path>>(&self, packs_dir: P) -> Result<()> {
        let packs_dir = packs_dir.as_ref();
        if self.pack_count != self.packs.len() {
            anyhow::bail!(
                "manifest pack count {} does not match {} indexed packs",
                self.pack_count,
                self.packs.len()
            );
        }

        for (pack_id, expected) in &self.packs {
            let path = packs_dir.join(&expected.relative_path);
            let contents = fs::read(&path)
                .with_context(|| format!("failed to read pack file: {}", path.display()))?;
            let actual_sha256 = sha256_hex(&contents);
            if actual_sha256 != expected.sha256 {
                anyhow::bail!(
                    "pack '{}' has mismatched SHA-256: expected {}, got {}",
                    pack_id,
                    expected.sha256,
                    actual_sha256
                );
            }
            if contents.len() as u64 != expected.size_bytes {
                anyhow::bail!(
                    "pack '{}' has mismatched size: expected {}, got {}",
                    pack_id,
                    expected.size_bytes,
                    contents.len()
                );
            }
        }

        for entry in fs::read_dir(packs_dir)
            .with_context(|| format!("failed to read packs directory: {}", packs_dir.display()))?
        {
            let path = entry.context("failed to read directory entry")?.path();
            if path.is_file()
                && path.extension().and_then(|extension| extension.to_str()) == Some("json")
            {
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default();
                let pack_id = name.strip_suffix(".json").unwrap_or(name);
                if !self.packs.contains_key(pack_id) {
                    anyhow::bail!("unexpected pack file '{}' is not in the manifest", name);
                }
            }
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).context("failed to serialize pack manifest")
    }

    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let contents = fs::read_to_string(path)
            .with_context(|| format!("failed to read manifest file: {}", path.display()))?;
        let manifest: Self = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse manifest JSON: {}", path.display()))?;
        if manifest.version != MANIFEST_VERSION {
            anyhow::bail!(
                "unsupported manifest version '{}'; expected '{}'",
                manifest.version,
                MANIFEST_VERSION
            );
        }
        Ok(manifest)
    }
}

fn sha256_hex(contents: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(contents);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_round_trips_and_detects_byte_mutation() {
        let directory = tempfile::tempdir().expect("temporary packs directory");
        let fixture = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/previous-release.json"
        );
        let contents = fs::read(fixture).expect("fixture pack should be readable");
        let pack: Pack = serde_json::from_slice(&contents).expect("fixture pack should parse");
        let pack_path = directory.path().join(format!("{}.json", pack.id));
        fs::write(&pack_path, contents).expect("fixture pack should be written");

        let manifest = PackManifest::from_dir(directory.path()).expect("manifest should generate");
        let parsed: PackManifest =
            serde_json::from_str(&manifest.to_json().unwrap()).expect("manifest should round-trip");
        parsed
            .verify_dir(directory.path())
            .expect("unchanged directory should verify");

        fs::write(&pack_path, "{}\n").expect("fixture pack should mutate");
        assert!(parsed.verify_dir(directory.path()).is_err());
    }
}
