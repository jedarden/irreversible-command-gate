# Rule Pack Authoring Best Practices

Comprehensive guide for creating, maintaining, and testing high-quality rule packs for the irreversible command gate (icg).

## Table of Contents

1. [Rule Pack Fundamentals](#rule-pack-fundamentals)
2. [Design Principles](#design-principles)
3. [Pattern Authoring](#pattern-authoring)
4. [Testing and Validation](#testing-and-validation)
5. [Maintenance and Updates](#maintenance-and-updates)
6. [Common Pitfalls](#common-pitfalls)
7. [Advanced Patterns](#advanced-patterns)
8. [Review Process](#review-process)
9. [Examples and Anti-Patterns](#examples-and-anti-patterns)

---

## Rule Pack Fundamentals

### What is a Rule Pack?

A **rule pack** is a JSON file that defines:
- **Safe patterns**: Operations that should always be allowed
- **Guarded patterns**: Operations that should be blocked or redirected
- **Metadata**: ID, keywords, file types, and explanations

### Rule Pack Structure

```json
{
  "id": "pack-id",
  "tool_keywords": ["tool1", "tool2"],
  "applies_to": ["*.yaml", "*.yml"],
  "safe_patterns": [
    {
      "id": "safe-read",
      "type": "command_regex",
      "regex": "^tool read"
    }
  ],
  "guarded_patterns": [
    {
      "id": "dangerous-delete",
      "type": "command_regex",
      "regex": "tool delete.*--force",
      "tier": "tier1",
      "severity": "Critical",
      "explanation": "Permanently deletes data",
      "destructive": true,
      "redirect": {
        "channel": "deny",
        "reason_template": "Use --soft-delete flag instead",
        "rewrite_template": null
      }
    }
  ]
}
```

### Pack Modes

**Command-Mode Packs**:
- Target shell command invocations
- Use `tool_keywords` to match executables
- Work in both hook and wrapper frontends
- Examples: vault, git, kubectl

**Content-Mode Packs**:
- Target file content being written
- Use `applies_to` globs to match file paths
- Hook-frontend only (Write/Edit operations)
- Examples: storage-class, image-tag, beads

**Choosing the Right Mode**:
```bash
# Question: Are you protecting command execution?
# YES → Command-mode pack
# NO  → Content-mode pack

# Question: Does the operation run as a subprocess?
# YES → Command-mode pack
# NO  → Content-mode pack
```

---

## Design Principles

### 1. Start Narrow, Expand Gradually

**Principle**: Begin with the most specific, high-risk patterns. Expand coverage iteratively based on real-world usage.

**Example**:
```json
// ❌ Bad: Too broad from the start
{
  "regex": "kubectl delete"
}

// ✅ Good: Start specific
{
  "regex": "kubectl delete pvc"
}

// ✅ Better: Expand after validation
{
  "regex": "kubectl delete (pvc|pv|deployment) --force"
}
```

**Rationale**: Broad patterns catch false positives, causing unnecessary friction. Specific patterns reduce noise while protecting critical operations.

### 2. Every Guarded Pattern Needs a Safe Alternative

**Principle**: If you block an operation, you must provide a safe way to achieve the same goal.

**Example**:
```json
// ❌ Bad: No alternative provided
{
  "redirect": {
    "channel": "deny",
    "reason_template": "Deleting deployments is dangerous",
    "rewrite_template": null
  }
}

// ✅ Good: Provides specific alternative
{
  "redirect": {
    "channel": "deny",
    "reason_template": "Deleting deployments is dangerous. Use 'kubectl scale deployment --replicas=0' instead to preserve the deployment object.",
    "rewrite_template": "kubectl scale deployment {{deployment_name}} --replicas=0"
  }
}
```

**Rationale**: Users will find a way to do what they need. Better to guide them to a safe approach than force them to bypass the guard.

### 3. Fail-Open Design

**Principle**: When uncertain, allow the operation. A missed violation is recoverable; a stuck fleet is not.

**Implementation**:
```rust
// In evaluation engine
if let Err(e) = parse_command(input) {
    // Log the error but allow the operation
    log::error!("Parse error: {}", e);
    return Verdict::Allow; // Fail-open
}
```

**Rule Pack Implications**:
- Don't create patterns that are so broad they might match unrelated operations
- Don't try to cover every edge case in the first version
- Prefer specific patterns over complex, error-prone ones

### 4. Zero False Positives for Critical Operations

**Principle**: Critical-severity patterns must never block legitimate operations.

**Validation**:
```bash
# Test against your actual workflow
git log --all --oneline | grep "force" | \
  while read commit; do
    echo "$commit" | icg check --stdin
  done

# If any legitimate commits are blocked, the pattern is too broad
```

### 5. Patterns Should Be Self-Documenting

**Principle**: The pattern ID and explanation should be sufficient for someone to understand what's being protected and why.

**Example**:
```json
// ❌ Bad: Cryptic ID and vague explanation
{
  "id": "d1",
  "explanation": "Dangerous operation"
}

// ✅ Good: Descriptive ID and specific explanation
{
  "id": "vault-kv-destroy-permanent",
  "explanation": "vault kv destroy permanently destroys secret data versions and cannot be undone. This is different from vault kv delete, which only removes metadata."
}
```

---

## Pattern Authoring

### Pattern ID Conventions

Use descriptive, hierarchical IDs:

```
<prefix>-<tool>-<operation>-<modifier>

Examples:
vault-kv-destroy-permanent
git-push-force-rewrite-history
kubectl-delete-pvc-data-loss
image-tag-latest-unpinned
storage-class-ssd-rackspace-spot
```

**Components**:
- **Prefix**: pack-id (e.g., vault, git, kubectl)
- **Tool**: specific command or subsystem
- **Operation**: what the command does
- **Modifier**: why it's dangerous or specific condition

### Regex Best Practices

#### 1. Use Anchors for Exact Matches

```json
// ❌ Bad: Matches "vault get" anywhere in command
{
  "regex": "vault kv get"
}

// ✅ Good: Only matches commands starting with vault kv get
{
  "regex": "^vault kv get"
}
```

#### 2. Be Specific About Arguments

```json
// ❌ Bad: Too broad
{
  "regex": "kubectl delete"
}

// ✅ Good: Specific resource types
{
  "regex": "kubectl delete (pvc|pv|persistentvolumeclaim)"
}
```

#### 3. Handle Command Chaining

```json
// Commands can be chained with &&, ||, ;
// Your pattern should still match

// ✅ Good: Handles chaining
{
  "regex": "(?:^|&&|\\|\\||;)\\s*kubectl delete pvc"
}

// This matches:
// kubectl delete pvc data-pvc
// kubectl get pods && kubectl delete pvc data-pvc
// kubectl get pods || kubectl delete pvc data-pvc
// kubectl get pods; kubectl delete pvc data-pvc
```

#### 4. Escape Special Characters

```json
// Special regex characters: . * + ? ^ $ { } [ ] ( ) | \ /

// ❌ Bad: Unescaped dots match any character
{
  "regex": "kubectl.kv.delete"
}

// ✅ Good: Escaped dots match literal dots
{
  "regex": "kubectl\\.kv\\.delete"
}
```

#### 5. Use Non-Capturing Groups

```json
// ❌ Bad: Capturing groups create overhead
{
  "regex": "(kubectl) delete (pvc|pv)"
}

// ✅ Good: Non-capturing groups
{
  "regex": "(?:kubectl) delete (?:pvc|pv)"
}
```

### Severity Assignment

Use this decision tree:

```
Is the damage irreversible?
├─ YES → Critical
│   Examples: vault kv destroy, git push --force
│
└─ NO
    Is the damage significant or hard to reverse?
    ├─ YES → High
    │   Examples: kubectl delete pvc, :latest tags
    │
    └─ NO → Medium
        Examples: kubectl delete pod, deprecated tools
```

**Guidelines**:
- **Critical**: Data loss, history rewriting, state corruption
- **High**: Resource deletion, wrong configuration, service disruption
- **Medium**: Deprecated operations, minor disruptions

### Redirect Templates

**Best Practices**:

1. **Be Specific**: Tell the user exactly what to run
   ```json
   // ❌ Bad: Vague advice
   {
     "reason_template": "Use a safer alternative"
   }

   // ✅ Good: Specific command
   {
     "reason_template": "Use 'kubectl scale deployment --replicas=0' instead"
   }
   ```

2. **Preserve User Intent**: The alternative should accomplish the same goal
   ```json
   // User wants to remove a field
   // Bad alternative: "Don't do it"
   // Good alternative: "Use patch -remove instead"
   ```

3. **Provide Context**: Explain why the alternative is safer
   ```json
   {
     "reason_template": "Use 'vault kv patch' instead. It allows safe field removal without destroying the entire secret version."
   }
   ```

### Explanation Writing

**Structure**:
1. What the operation does
2. Why it's dangerous
3. What gets damaged
4. Why it can't be easily undone

**Example**:
```json
{
  "explanation": "vault kv destroy permanently destroys secret data versions. Unlike vault kv delete (which only removes metadata), destroy makes the data unrecoverable. This operation cannot be undone and should never be used in automated workflows."
}
```

**Tips**:
- Write for someone who's not familiar with the tool
- Avoid jargon when possible
- Explain the difference between similar operations
- Mention recovery options (if any exist)

---

## Testing and Validation

### Unit Testing

Create comprehensive unit tests for every pattern:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_patterns_allow() {
        let pack = load_pack("packs/vault.json").unwrap();

        // Safe operations should be allowed
        assert!(pack.allows("vault kv get secret/test"));
        assert!(pack.allows("vault kv list secret/"));
        assert!(pack.allows("vault status"));
    }

    #[test]
    fn test_guarded_patterns_block() {
        let pack = load_pack("packs/vault.json").unwrap();

        // Dangerous operations should be blocked
        assert!(pack.blocks("vault kv destroy secret/test"));
        assert!(pack.blocks("vault policy delete my-policy"));
    }

    #[test]
    fn test_command_chaining() {
        let pack = load_pack("packs/vault.json").unwrap();

        // Should block even when chained
        assert!(pack.blocks("vault status && vault kv destroy secret/test"));
        assert!(pack.blocks("vault kv get secret/test || vault kv destroy secret/test"));
    }

    #[test]
    fn test_case_sensitivity() {
        let pack = load_pack("packs/vault.json").unwrap();

        // Commands are case-sensitive
        assert!(pack.blocks("vault kv destroy secret/test"));
        assert!(!pack.blocks("VAULT kv destroy secret/test"));
        assert!(!pack.blocks("vault KV DESTROY secret/test"));
    }

    #[test]
    fn test_whitespace_variations() {
        let pack = load_pack("packs/vault.json").unwrap();

        // Should handle various whitespace patterns
        assert!(pack.blocks("vault  kv  destroy  secret/test"));
        assert!(pack.blocks("vault\tkv\tdestroy\tsecret/test"));
    }
}
```

### Integration Testing

Test the pack in realistic scenarios:

```bash
#!/bin/bash
# test-pack.sh

set -e

PACK_PATH="$1"
TEST_COMMANDS="$2"

echo "Testing pack: $PACK_PATH"

# Test safe commands
echo "Testing safe commands..."
while IFS= read -r cmd; do
  echo "  Testing: $cmd"
  result=$(echo "$cmd" | icg check --stdin --pack "$PACK_PATH")
  if [[ "$result" != *"\"verdict\":\"allow\""* ]]; then
    echo "❌ FAIL: Safe command was blocked: $cmd"
    exit 1
  fi
done < "$TEST_COMMANDS/safe.txt"

# Test dangerous commands
echo "Testing dangerous commands..."
while IFS= read -r cmd; do
  echo "  Testing: $cmd"
  result=$(echo "$cmd" | icg check --stdin --pack "$PACK_PATH")
  if [[ "$result" != *"\"verdict\":\"deny\""* ]]; then
    echo "❌ FAIL: Dangerous command was allowed: $cmd"
    exit 1
  fi
done < "$TEST_COMMANDS/dangerous.txt"

echo "✅ All tests passed"
```

### Regression Testing

Generate and maintain regression suites:

```bash
# Generate regression suite for current pack
icg regression-suite \
  packs/vault.json \
  --output tests/fixtures/vault-regression.json

# Verify no coverage narrowing
icg verify-coverage \
  --current tests/fixtures/vault-regression.json \
  --previous tests/fixtures/vault-regression-baseline.json

# Expected output:
# ✓ No coverage narrowing detected
# ✓ All destructive patterns still protected
```

### False Positive Testing

Test against real command histories:

```bash
# Extract real commands from git history
git log --all --pretty=format:"%H" | \
  while read commit; do
    git show "$commit" | grep -E "^(vault|kubectl|git)" | \
      icg check --stdin --pack /etc/icg/packs/vault.json
  done > /tmp/real-command-tests.txt

# Check for false positives
grep "deny" /tmp/real-command-tests.txt | \
  while read line; do
    # Verify this is actually a dangerous command
    # If not, you have a false positive
  done
```

---

## Maintenance and Updates

### Versioning

Follow semantic versioning for rule packs:

```
MAJOR.MINOR.PATCH

MAJOR: Breaking changes to pack structure
MINOR: New patterns, pattern improvements
PATCH: Bug fixes, documentation updates
```

**Example**:
```
vault pack v1.2.3
├─ 1: Major version (stable API)
├─ 2: Added new patterns
└─ 3: Fixed false positive
```

### Change Documentation

Maintain a CHANGELOG.md:

```markdown
# Changelog

## [1.2.0] - 2026-08-16

### Added
- Pattern: vault-kv-patch-remove (safe field removal)
- Pattern: vault-operator-token-safe-read

### Changed
- Improved vault-kv-destroy regex to avoid false positives
- Updated explanations for clarity

### Fixed
- False positive in vault-kv-get when path contains "destroy"
- Fixed regex escaping in vault-policy-delete

## [1.1.0] - 2026-07-15

### Added
- Initial pack release with 8 patterns
```

### Update Process

Before releasing an updated pack:

1. **Run full test suite**:
   ```bash
   cargo test --all
   ```

2. **Generate regression suite**:
   ```bash
   icg regression-suite pack.json --output regression-new.json
   ```

3. **Verify no coverage narrowing**:
   ```bash
   icg verify-coverage \
     --current regression-new.json \
     --previous regression-old.json
   ```

4. **Manual testing**:
   ```bash
   # Test against real commands
   cat real-commands.txt | icg check --stdin --pack pack.json
   ```

5. **Documentation update**:
   ```bash
   # Update README and CHANGELOG
   vim README.md CHANGELOG.md
   ```

6. **Release**:
   ```bash
   # Tag and push
   git tag -a vault-pack-v1.2.0 -m "Release vault pack v1.2.0"
   git push origin vault-pack-v1.2.0
   ```

### Backward Compatibility

**Breaking Changes**:
- Removing or modifying pattern IDs
- Changing pack structure
- Removing safe patterns

**Non-Breaking Changes**:
- Adding new patterns
- Improving regex precision
- Updating explanations
- Bug fixes

**Policy**: When in doubt, create a new pattern rather than modifying an existing one.

---

## Common Pitfalls

### Pitfall 1: Overly Broad Patterns

**Problem**: Pattern matches too many operations, causing false positives.

```json
// ❌ Bad: Matches "kubectl delete" anywhere
{
  "regex": "delete"
}

// ✅ Good: Specific command and resource
{
  "regex": "^kubectl delete pvc"
}
```

**Detection**: High denial rate, user complaints about false positives.

**Prevention**:
- Use anchors (^ and $)
- Be specific about tool names
- Test against real command histories

### Pitfall 2: Missing Command Chaining

**Problem**: Pattern doesn't match when command is chained.

```json
// ❌ Bad: Doesn't handle chaining
{
  "regex": "^kubectl delete pvc"
}

// ✅ Good: Handles chaining
{
  "regex": "(?:^|&&|\\|\\||;)\\s*kubectl delete pvc"
}
```

**Detection**: Users report bypassing guard by chaining safe command before dangerous one.

**Prevention**:
- Always test with &&, ||, and ; chaining
- Use (?:^|&&|\\|\\||;) pattern for command boundaries

### Pitfall 3: No Safe Alternative

**Problem**: Blocking an operation without providing a safe way to accomplish the goal.

```json
// ❌ Bad: Just blocks
{
  "reason_template": "This is dangerous"
}

// ✅ Good: Provides alternative
{
  "reason_template": "Use 'kubectl scale deployment --replicas=0' instead"
}
```

**Detection**: Users bypass guard to accomplish legitimate tasks.

**Prevention**:
- Always provide a specific alternative
- Explain why the alternative is safer
- Test that the alternative actually works

### Pitfall 4: Ignoring Case Sensitivity

**Problem**: Pattern doesn't account for case variations.

```json
// ❌ Bad: Only matches lowercase
{
  "regex": "vault kv destroy"
}

// ✅ Good: Explicitly case-sensitive (commands usually are)
{
  "regex": "(?i)^vault kv destroy"
}

// Or document that commands are case-sensitive
```

**Detection**: Pattern fails when command has different case.

**Prevention**:
- Test with uppercase, lowercase, and mixed case
- Document expected case sensitivity
- Use (?i) flag if case-insensitive matching is desired

### Pitfall 5: Regex Performance Issues

**Problem**: Complex regex causes performance problems.

```json
// ❌ Bad: Catastrophic backtracking
{
  "regex": "(.*+){100,}"
}

// ✅ Good: Efficient pattern
{
  "regex": "^kubectl delete pvc [a-z0-9-]+"
}
```

**Detection**: Slow evaluation, high CPU usage.

**Prevention**:
- Avoid nested quantifiers
- Use specific character classes instead of .
- Benchmark regex performance
- Use ^ and $ anchors when possible

### Pitfall 6: Escaping Issues

**Problem**: Special characters not properly escaped.

```json
// ❌ Bad: Unescaped special chars
{
  "regex": "kubectl.kv.delete"
}

// ✅ Good: Properly escaped
{
  "regex": "kubectl\\.kv\\.delete"
}
```

**Detection**: Pattern matches unexpected commands.

**Prevention**:
- Always escape: . * + ? ^ $ { } [ ] ( ) | \
- Test regex with online validators
- Document special characters in pattern

---

## Advanced Patterns

### Predicates

For checks that can't be done with regex alone:

```json
{
  "id": "beads-shared-checkout-write",
  "type": "predicate",
  "predicate_name": "is_shared_checkout",
  "tier": "tier1",
  "severity": "Critical",
  "explanation": "Writing to .beads/ in a shared checkout risks concurrent corruption",
  "destructive": true,
  "redirect": {
    "channel": "deny",
    "reason_template": "Writing to .beads/ in a shared checkout risks concurrent corruption. Use a worktree instead.",
    "rewrite_template": null
  }
}
```

**Implementing predicates**:
```rust
pub fn is_shared_checkout() -> bool {
    Path::new(".git").is_dir()
}

pub fn has_uncommitted_changes() -> bool {
    Command::new("git")
        .args(&["status", "--porcelain"])
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false)
}
```

### Multi-Condition Patterns

Combine multiple checks:

```json
{
  "id": "git-force-push-on-main",
  "type": "command_regex",
  "regex": "git push.*--force",
  "tier": "tier1",
  "severity": "Critical",
  "additional_checks": [
    {
      "type": "predicate",
      "predicate_name": "is_on_main_branch"
    },
    {
      "type": "predicate",
      "predicate_name": "has_stale_head"
    }
  ],
  "explanation": "Force-pushing to main branch when HEAD is stale rewrites public history",
  "destructive": true,
  "redirect": {
    "channel": "deny",
    "reason_template": "Pull latest changes first: git pull origin main",
    "rewrite_template": null
  }
}
```

### Content-Mode Patterns

For file content inspection:

```json
{
  "id": "storage-class-ssd",
  "type": "content_regex",
  "regex": "storageClassName:\\s*.*ssd",
  "tier": "tier1",
  "severity": "High",
  "explanation": "Using ssd storage class on Rackspace Spot is prohibited",
  "destructive": false,
  "redirect": {
    "channel": "deny",
    "reason_template": "Use 'sata' or 'sata-large' storage class instead of 'ssd'",
    "rewrite_template": "storageClassName: sata"
  }
}
```

### Rewrite Templates

Provide automatically corrected commands:

```json
{
  "redirect": {
    "channel": "updated_input",
    "reason_template": "Replacing :latest with pinned version",
    "rewrite_template": "image: {{image_name}}:{{pinned_version}}"
  }
}
```

**Using templates**:
- Variables extracted from command: `{{image_name}}`, `{{pinned_version}}`
- Must be valid regex capture groups
- Template must produce syntactically valid output

---

## Review Process

### Pre-Merge Checklist

Before submitting a rule pack for review:

- [ ] All patterns have descriptive IDs
- [ ] All patterns have clear explanations
- [ ] All guarded patterns have safe alternatives
- [ ] Unit tests cover all patterns
- [ ] Integration tests pass
- [ ] Regression suite generated
- [ ] Documentation updated (README, CHANGELOG)
- [ ] No false positives in real command history
- [ ] Performance benchmarks acceptable
- [ ] Code reviewed by at least one other person

### Review Criteria

**Technical Review**:
- Regex correctness and efficiency
- Pattern specificity and accuracy
- Test coverage and quality
- Documentation clarity

**Security Review**:
- Severity assignment accuracy
- Adequate protection of dangerous operations
- No bypass opportunities
- Fail-open behavior maintained

**Usability Review**:
- Clear error messages
- Helpful redirect suggestions
- False positive rate
- User documentation

### Approval Workflow

1. **Submit PR** with rule pack changes
2. **Automated checks** run (tests, linting)
3. **Technical review** by maintainers
4. **Security review** for new patterns
5. **Usability review** if significant changes
6. **Approval** and merge
7. **Release** following semantic versioning

---

## Examples and Anti-Patterns

### Example: Well-Structured Pattern

```json
{
  "id": "vault-kv-destroy-permanent",
  "type": "command_regex",
  "regex": "(?:^|&&|\\|\\||;)\\s*vault\\s+kv\\s+destroy",
  "tier": "tier1",
  "severity": "Critical",
  "explanation": "vault kv destroy permanently destroys secret data versions. Unlike vault kv delete (which only removes metadata), destroy makes the data unrecoverable. This operation cannot be undone and should never be used in automated workflows. The only safe alternative is vault kv patch for field-level modifications or vault kv delete for metadata-only removal.",
  "destructive": true,
  "redirect": {
    "channel": "deny",
    "reason_template": "vault kv destroy is permanently destructive and cannot be undone. Use 'vault kv patch <path> -remove=<field>' for safe field removal or 'vault kv delete <path>' for metadata-only deletion.",
    "rewrite_template": "vault kv patch {{path}} -remove={{field}}"
  }
}
```

**Why it's good**:
- Descriptive, hierarchical ID
- Handles command chaining
- Properly escaped regex
- Comprehensive explanation
- Specific alternatives provided
- Template for automatic rewrite

### Anti-Pattern: Overly Broad Pattern

```json
{
  "id": "delete",
  "type": "command_regex",
  "regex": "delete",
  "tier": "tier1",
  "severity": "Critical",
  "explanation": "Deleting is dangerous",
  "destructive": true,
  "redirect": {
    "channel": "deny",
    "reason_template": "Don't delete things",
    "rewrite_template": null
  }
}
```

**Problems**:
- Cryptic ID
- Matches any command containing "delete"
- No context about what's being deleted
- Vague explanation
- Unhelpful redirect
- No tool specificity

**Fix**:
```json
{
  "id": "kubectl-delete-pvc-data-loss",
  "type": "command_regex",
  "regex": "(?:^|&&|\\|\\||;)\\s*kubectl\\s+delete\\s+pvc",
  "tier": "tier1",
  "severity": "Critical",
  "explanation": "Deleting a PersistentVolumeClaim destroys the persistent data volume. This data cannot be recovered and will cause permanent data loss for any application using the PVC.",
  "destructive": true,
  "redirect": {
    "channel": "deny",
    "reason_template": "kubectl delete pvc is permanently destructive. If you need to reclaim the PVC object without data loss, use 'kubectl patch pvc <name> -p \"{\\\"metadata\\\":{\\\"finalizers\\\":null}}}\"' first. For volume data backup, use 'kubectl get pvc <name> -o yaml > backup.yaml' before deletion.",
    "rewrite_template": null
  }
}
```

### Example: Content-Mode Pattern

```json
{
  "id": "image-tag-latest-unpinned",
  "type": "content_regex",
  "regex": "image:\\s*.*:(latest|latest\\s*$)",
  "applies_to": ["*.yaml", "*.yml"],
  "tier": "tier1",
  "severity": "High",
  "explanation": "Using the :latest tag for container images means the image version is not pinned. This makes deployments non-reproducible and can lead to unexpected behavior when a new :latest image is pushed. Always use a specific version tag (e.g., v1.2.3) or a digest pin (e.g., sha256:abc123...).",
  "destructive": false,
  "redirect": {
    "channel": "deny",
    "reason_template": "Image tag :latest is not pinned to a specific version. Use a semantic version tag (e.g., v1.2.3) or commit SHA instead. Example: 'image: myapp:v1.2.3' or 'image: myapp@sha256:abc123...'",
    "rewrite_template": "image: {{image_name}}:{{pinned_version}}"
  }
}
```

**Why it's good**:
- Clear what file types it applies to
- Specific to image tag patterns
- Explains the risk clearly
- Provides concrete alternatives
- Shows examples of correct usage

---

## Resources

### Tools

- **Regex Tester**: https://regex101.com
- **JSON Validator**: https://jsonlint.com
- **icg CLI**: `icg check --command "<cmd>" --debug`

### Documentation

- **Developer Guide**: `docs/developers/README.md`
- **Examples**: `docs/examples/README.md`
- **Architecture**: `docs/plan/plan.md`

### Templates

- **Pack Template**: `packs/template/pack.json`
- **Test Template**: `packs/template/tests.rs`

---

## Appendix

### Regex Reference

| Symbol | Meaning | Example |
|--------|---------|---------|
| `^` | Start of string | `^kubectl` |
| `$` | End of string | `destroy$` |
| `*` | Zero or more | `kubectl.*` |
| `+` | One or more | `kv+` |
| `?` | Zero or one | `force?` |
| `\\s` | Whitespace | `kv\\s+destroy` |
| `\\d` | Digit | `v\\d+\\.\\d+\\.\\d+` |
| `[...]` | Character class | `[pvc]` |
| `(?:...)` | Non-capturing group | `(?:&&|\\|\\|)` |

### Severity Reference

| Severity | When to Use | Examples |
|----------|-------------|----------|
| Critical | Irreversible damage | Data destruction, history rewrite |
| High | Significant damage | Resource deletion, wrong config |
| Medium | Moderate damage | Service disruption |

### Channel Reference

| Channel | When to Use | Behavior |
|---------|-------------|----------|
| deny | Block entirely | Critical/high severity |
| updated_input | Provide alternative | Future feature |
| additional_context | Warn only | Tier 3 patterns |

---

**Best Practices Guide Version**: 1.0
**Last Updated**: 2026-08-16
**For**: icg v0.1.0+
