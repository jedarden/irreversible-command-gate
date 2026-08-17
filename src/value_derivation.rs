//! Helpers for replacing values that can be determined from the checked input.
//!
//! Rule-pack reasons are deliberately authored as templates so that a rule can
//! explain where a replacement comes from without hard-coding a repository's
//! current value.  The `{derived_value}` placeholder is currently used by the
//! image-tag rules: the value is the semver stored in the matching container's
//! `VERSION` file.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const DERIVED_VALUE_PLACEHOLDER: &str = "{derived_value}";
const DERIVED_VALUE_FALLBACK: &str = "the semver value from containers/<name>/VERSION";

/// Render a reason template with the value derivable from a content check.
///
/// Missing repository files must not turn a deny into an allow.  In that case
/// the placeholder is replaced with an actionable description rather than
/// leaking an unresolved template into the hook response.
pub fn render_reason(template: &str, content: Option<&str>, file_path: Option<&str>) -> String {
    if !template.contains(DERIVED_VALUE_PLACEHOLDER) {
        return template.to_owned();
    }

    let value = content
        .zip(file_path)
        .and_then(|(content, file_path)| derive_image_version(content, file_path))
        .unwrap_or_else(|| DERIVED_VALUE_FALLBACK.to_owned());

    template.replace(DERIVED_VALUE_PLACEHOLDER, &value)
}

/// Find the semver values for unpinned images in `content`.
///
/// The image name is taken from the final path component of an image
/// reference, so both `ronaldraygun/worker:latest` and
/// `registry.example/ronaldraygun/worker:01234567` resolve to
/// `containers/worker/VERSION`.  Repository lookup starts next to the target
/// file and walks upward, then falls back to the current working tree.  This
/// supports both absolute hook paths and the relative paths used by the
/// harnesses.
pub fn derive_image_version(content: &str, file_path: &str) -> Option<String> {
    let image_names = image_names_needing_a_version(content);
    if image_names.is_empty() {
        return None;
    }

    let roots = candidate_roots(file_path);
    let mut versions = Vec::new();
    let mut seen_versions = HashSet::new();
    let mut all_images_resolved = true;

    for image_name in image_names {
        let mut image_resolved = false;
        for root in &roots {
            let version_path = root.join("containers").join(&image_name).join("VERSION");
            let Ok(contents) = fs::read_to_string(version_path) else {
                continue;
            };

            let Some(version) = version_from_file(&contents) else {
                continue;
            };

            if seen_versions.insert(version.clone()) {
                versions.push(version);
            }
            image_resolved = true;
            break;
        }
        all_images_resolved &= image_resolved;
    }

    if !all_images_resolved {
        return None;
    }

    match versions.len() {
        0 => None,
        1 => versions.into_iter().next(),
        _ => Some(versions.join(", ")),
    }
}

fn image_names_needing_a_version(content: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();

    for line in content.lines() {
        let Some(value) = image_value_from_line(line) else {
            continue;
        };
        let Some((repository, tag)) = image_repository_and_tag(value) else {
            continue;
        };
        if tag != "latest" && !is_bare_sha(tag) {
            continue;
        }

        let Some(name) = repository.rsplit('/').next() else {
            continue;
        };
        if !is_safe_container_name(name) || !seen.insert(name.to_owned()) {
            continue;
        }
        names.push(name.to_owned());
    }

    names
}

fn image_value_from_line(line: &str) -> Option<&str> {
    let line = line.trim_start();
    let line = line.strip_prefix('-').map(str::trim_start).unwrap_or(line);
    let value = line.strip_prefix("image:")?.trim();
    let value = value.split('#').next()?.trim();
    let value = value.trim_matches(|character| character == '"' || character == '\'');
    (!value.is_empty()).then_some(value)
}

fn image_repository_and_tag(image: &str) -> Option<(&str, &str)> {
    // Digest references are already pinned and are not candidates for a
    // VERSION-derived tag.
    if image.contains('@') {
        return None;
    }

    let separator = image.rfind(':')?;
    let repository = &image[..separator];
    let tag = &image[separator + 1..];
    if repository.is_empty() || tag.is_empty() || tag.contains('/') {
        return None;
    }
    Some((repository, tag))
}

fn is_bare_sha(value: &str) -> bool {
    (8..=64).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_safe_container_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn version_from_file(contents: &str) -> Option<String> {
    let version = contents
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    if version.split_whitespace().count() != 1 || !is_semver(version) {
        return None;
    }
    Some(version.to_owned())
}

fn is_semver(value: &str) -> bool {
    let value = value.strip_prefix('v').unwrap_or(value);
    let core_end = value
        .find(|character| character == '-' || character == '+')
        .unwrap_or(value.len());
    let core = &value[..core_end];
    let components: Vec<_> = core.split('.').collect();
    if components.len() != 3
        || components.iter().any(|component| {
            component.is_empty() || component.bytes().any(|byte| !byte.is_ascii_digit())
        })
    {
        return false;
    }

    let suffix = &value[core_end..];
    suffix
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'+' | b'.'))
}

fn candidate_roots(file_path: &str) -> Vec<PathBuf> {
    let path = Path::new(file_path);
    let mut roots = Vec::new();

    if path.is_absolute() {
        if let Some(parent) = path.parent() {
            append_ancestors(parent, &mut roots);
        }
    } else if let Ok(current_dir) = std::env::current_dir() {
        let absolute_path = current_dir.join(path);
        if let Some(parent) = absolute_path.parent() {
            append_ancestors(parent, &mut roots);
        }
    }

    if let Ok(current_dir) = std::env::current_dir() {
        append_ancestors(&current_dir, &mut roots);
    }

    roots
}

fn append_ancestors(path: &Path, roots: &mut Vec<PathBuf>) {
    let mut current = path;
    loop {
        if !roots.iter().any(|root| root == current) {
            roots.push(current.to_path_buf());
        }

        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn derives_the_version_for_latest_image_from_absolute_manifest_path() {
        let root = tempfile::tempdir().expect("temporary repository should exist");
        fs::create_dir_all(root.path().join("containers/worker")).expect("container directory");
        fs::write(root.path().join("containers/worker/VERSION"), "1.2.3\n").expect("version file");

        let manifest = root.path().join("deploy/app.yaml");
        let content = "image: ronaldraygun/worker:latest\n";

        assert_eq!(
            derive_image_version(content, manifest.to_str().unwrap()),
            Some("1.2.3".to_owned())
        );
    }

    #[test]
    fn renders_an_actionable_fallback_when_version_cannot_be_read() {
        let rendered = render_reason(
            "Pin this image to {derived_value}.",
            Some("image: ronaldraygun/missing:latest"),
            Some("deploy/app.yaml"),
        );

        assert_eq!(
            rendered,
            "Pin this image to the semver value from containers/<name>/VERSION."
        );
        assert!(!rendered.contains(DERIVED_VALUE_PLACEHOLDER));
    }
}
