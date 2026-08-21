# Irreversible Command Gate (icg) - Developer Guide

Welcome to the developer documentation for the Irreversible Command Gate (icg). This guide explains how to extend icg with new rule packs, contribute to the core engine, and understand the system architecture.

## Table of Contents

1. [Architecture Overview](#architecture-overview)
2. [Development Environment](#development-environment)
3. [Understanding Rule Packs](#understanding-rule-packs)
4. [Creating a New Rule Pack](#creating-a-new-rule-pack)
5. [Front-End Integration](#front-end-integration)
6. [Testing and Validation](#testing-and-validation)
7. [Release Process](#release-process)
8. [Code Organization](#code-organization)
9. [Common Patterns](#common-patterns)

---

## Architecture Overview

### System Components

icg consists of several key components:

```
┌─────────────────────────────────────────────────────────────┐
│                      AI Agent                                │
└───────────────────────────┬─────────────────────────────────┘
                            │ attempts operation
                            ▼
┌─────────────────────────────────────────────────────────────┐
│                    Front-End Layer                           │
│  ┌──────────────────────┐  ┌──────────────────────────┐   │
│  │  Claude Code Hook    │  │  Codex CLI Hook           │   │
│  │  (PreToolUse JSON)   │  │  (PreToolUse JSON)        │   │
│  └──────────────────────┘  └──────────────────────────┘   │
│  ┌──────────────────────┐                                    │
│  │  PATH Wrapper        │                                    │
│  │  (Symlink shadows)   │                                    │
│  └──────────────────────┘                                    │
└───────────────────────────┬─────────────────────────────────┘
                            │ parsed input
                            ▼
┌─────────────────────────────────────────────────────────────┐
│                    Evaluation Engine                         │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  Input Parser (command-mode & content-mode)          │  │
│  └──────────────────────────────────────────────────────┘  │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  Pack Dispatcher (matches tool_keywords/applies_to)   │  │
│  └──────────────────────────────────────────────────────┘  │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  Pattern Evaluator (safe_patterns → guarded_patterns)  │  │
│  └──────────────────────────────────────────────────────┘  │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  Redirect Handler (deny/updated_input/context)       │  │
│  └──────────────────────────────────────────────────────┘  │
└───────────────────────────┬─────────────────────────────────┘
                            │ decision + redirect
                            ▼
┌─────────────────────────────────────────────────────────────┐
│                    Output Layer                              │
│  ┌──────────────────────┐  ┌──────────────────────────┐   │
│  │  Structured Denial    │  │  Telemetry/Logging       │   │
│  │  (JSON to stdout)     │  │  (Denial log, metrics)   │   │
│  └──────────────────────┘  └──────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

### Key Design Principles

1. **Fail-Open by Default**: Any parse error or exception allows the operation to proceed
   - A missed violation is recoverable; a stuck fleet is not
   - Only the guard process crashing graduates to fail-closed after reliability validation

2. **Zero Network I/O**: Core evaluation doesn't make network calls
   - Ensures deterministic behavior
   - Prevents cascading failures
   - Exception: `git push` stale-HEAD check (already a network operation)

3. **Modular Rule Packs**: Each tool gets its own pack
   - Easy to add new rules without touching core code
   - Clear separation of concerns
   - Per-tool release cycles

4. **Two-Frontend Design**:
   - **Hook Frontend**: Claude Code & Codex CLI (PreToolUse JSON)
   - **Wrapper Frontend**: Symlink shadows in `$PATH`

5. **Redirect-Not-Just-Block**: Every denial explains what to do instead
   - `reason_template`: Why it's blocked
   - `rewrite_template`: Safe alternative (when available)
   - `channel`: deny, updated_input, or additional_context

---

## Development Environment

### Prerequisites

- **Rust**: 1.70+ (2021 edition)
- **Cargo**: Built-in build system
- **Git**: For version control
- **jq**: For JSON testing (optional but recommended)

### Setup

```bash
# Clone the repository
git clone https://github.com/jedarden/irreversible-command-gate.git
cd irreversible-command-gate

# Verify dependencies
cargo check

# Run tests
cargo test

# Build the binary
cargo build --release
```

### Project Structure

```
irreversible-command-gate/
├── src/
│   ├── main.rs              # CLI entry point, command routing
│   ├── lib.rs               # Library exports
│   ├── engine.rs            # Core evaluation engine
│   ├── rule_pack.rs         # Rule pack schema and loader
│   ├── state_store.rs       # Persistent state (Phase 2)
│   ├── telemetry.rs         # Metrics and logging
│   ├── health.rs            # Health checks
│   ├── denial_log.rs        # Denial history
│   ├── overrides.rs         # Per-repository overrides
│   ├── regression.rs        # Regression suite generation
│   ├── new_pack.rs          # New rule pack scaffolding
│   ├── update.rs            # Rule pack update system
│   └── trust_pointer.rs     # Trust on first use (TOFU) infrastructure
├── tests/
│   └── fixtures/            # Test rule pack fixtures
├── docs/
│   ├── developers/          # This documentation
│   ├── operators/           # Operator guides
│   ├── notes/               # Design decisions
│   ├── research/            # Prior art
│   └── plan/                # Implementation roadmap
└── Cargo.toml               # Rust dependencies
```

---

## Understanding Rule Packs

### Rule Pack Schema

A rule pack is a JSON file defining patterns for a specific tool or domain:

```json
{
  "id": "pack-id",
  "tool_keywords": ["tool1", "tool2"],
  "applies_to": ["*.yaml", "*.yml"],
  "safe_patterns": [...],
  "guarded_patterns": [...]
}
```

### Pack Modes

**Command-Mode Packs** (inspect shell invocations):
- Use `tool_keywords` to match executables
- Use `command_regex` for pattern matching
- Examples: `vault`, `git`, `misc`, `tmux`
- Work in both hook and wrapper frontends

**Content-Mode Packs** (inspect file writes):
- Use `applies_to` globs to match file paths
- Use `content_regex` for pattern matching
- Examples: `storage-class`, `image-tag`, `beads`
- Hook-frontend only (Write/Edit never reaches wrapper)

**Hybrid Packs**:
- `secrets` pack: Hook-only (scans entire Bash command string)
- Uses `command_regex` but unconditionally (no tool_keywords filter)

### Pattern Structure

#### Safe Pattern

```json
{
  "id": "safe-read",
  "type": "command_regex",
  "regex": "vault kv get"
}
```

#### Guarded Pattern

```json
{
  "id": "vault-kv-destroy",
  "type": "command_regex",
  "regex": "vault kv destroy",
  "tier": "tier1",
  "severity": "Critical",
  "explanation": "Permanently destroys vault data versions",
  "destructive": true,
  "redirect": {
    "channel": "deny",
    "reason_template": "vault kv destroy is permanently destructive and cannot be undone",
    "rewrite_template": null
  }
}
```

### Check Types

1. **CommandRegex**: Match against shell command tokens
   ```json
   {
     "type": "command_regex",
     "regex": "git push.*--force"
   }
   ```

2. **ContentRegex**: Match against file content
   ```json
   {
     "type": "content_regex",
     "regex": "storageClassName:.*ssd"
   }
   ```

3. **Predicate**: Custom check function (future)
   ```json
   {
     "type": "predicate",
     "predicate_name": "is_shared_checkout"
   }
   ```

### Severity Levels

- **Critical**: Immediate, irreversible damage (vault destroy, git force-push)
- **High**: Significant damage or hard to reverse (policy delete, ssd storage)
- **Medium**: Moderate damage with workarounds

### Response Channels

- **Deny**: Block the operation entirely (critical/high severity)
- **UpdatedInput**: Provide a safe alternative (future feature)
- **AdditionalContext**: Warn without blocking (Tier 3 patterns only)

---

## Creating a New Rule Pack

### Step 1: Identify the Domain

Determine what you're protecting:

- **Tool**: Command-mode pack (e.g., `kubectl`, `docker`)
- **File format**: Content-mode pack (e.g., `terraform`, `cloudformation`)
- **Domain**: Hybrid pack (e.g., `secrets`, `beads`)

### Step 2: Scaffold the Pack

Use the built-in scaffolding tool:

```bash
cargo run --new-pack \
  --id "kubectl" \
  --mode command \
  --keywords "kubectl,kubecfg"
```

Or manually create the JSON:

```bash
# Create pack directory
mkdir -p packs/kubectl

# Create pack manifest
cat > packs/kubectl/pack.json <<'EOF'
{
  "id": "kubectl",
  "tool_keywords": ["kubectl", "kubecfg"],
  "applies_to": [],
  "safe_patterns": [],
  "guarded_patterns": []
}
EOF
```

### Step 3: Define Safe Patterns

List operations that should always be allowed:

```json
{
  "safe_patterns": [
    {
      "id": "safe-get",
      "type": "command_regex",
      "regex": "kubectl get"
    },
    {
      "id": "safe-describe",
      "type": "command_regex",
      "regex": "kubectl describe"
    },
    {
      "id": "safe-logs",
      "type": "command_regex",
      "regex": "kubectl logs"
    }
  ]
}
```

**Best Practices for Safe Patterns**:
- Start specific, relax gradually
- Use `^` and `$` anchors for exact matches
- Test against real command sequences
- Consider command chaining (`&&`, `||`, `;`)

### Step 4: Define Guarded Patterns

Identify dangerous operations:

```json
{
  "guarded_patterns": [
    {
      "id": "kubectl-delete-deployment",
      "type": "command_regex",
      "regex": "kubectl delete deployment",
      "tier": "tier1",
      "severity": "High",
      "explanation": "Deleting a deployment removes all running pods",
      "destructive": true,
      "redirect": {
        "channel": "deny",
        "reason_template": "kubectl delete deployment is destructive. Use 'kubectl scale deployment --replicas=0' instead to preserve the deployment object.",
        "rewrite_template": null
      }
    }
  ]
}
```

**Best Practices for Guarded Patterns**:
- Start narrow, expand coverage iteratively
- Every pattern needs a clear explanation
- Provide alternatives when possible
- Mark as `destructive: true` if it causes data loss
- Use `tier: "tier1"` for stateless checks (Phase 1)

### Step 5: Write Tests

Create test cases for your patterns:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kubectl_safe_patterns() {
        let pack = load_pack("packs/kubectl/pack.json").unwrap();

        // Test safe operations
        assert!(pack.allows("kubectl get pods"));
        assert!(pack.allows("kubectl describe deployment myapp"));
        assert!(pack.allows("kubectl logs -f pod/mypod"));
    }

    #[test]
    fn test_kubectl_guarded_patterns() {
        let pack = load_pack("packs/kubectl/pack.json").unwrap();

        // Test dangerous operations
        assert!(pack.blocks("kubectl delete deployment myapp"));
    }
}
```

### Step 6: Generate Regression Suite

Generate a regression suite for CI:

```bash
cargo run --bin icg -- regression-suite \
  packs/kubectl/pack.json \
  --output tests/fixtures/kubectl-regression.json
```

This generates one validated deny case per enabled `guarded_pattern`.

### Step 7: Test Locally

Test the pack before integrating:

```bash
# Test specific command
cargo run --bin icg -- check \
  --command "kubectl delete deployment myapp" \
  --pack packs/kubectl/pack.json

# Run full test suite
cargo test

# Run integration tests
cargo test --test integration
```

### Step 8: Document the Pack

Create documentation for operators:

```markdown
# kubectl Pack

## Overview
Protects against destructive kubectl operations.

## Safe Operations
- `kubectl get` - Read resources
- `kubectl describe` - View resource details
- `kubectl logs` - View pod logs

## Protected Operations
- `kubectl delete deployment` - Deletes deployments
- `kubectl delete svc` - Deletes services

## Severity Levels
- High: Deleting managed resources
- Medium: Deleting unmanaged resources
```

---

## Front-End Integration

### Hook Frontend (Claude Code & Codex)

The hook frontend integrates with Claude Code and Codex CLI via the PreToolUse JSON interface.

#### How It Works

1. Agent calls a tool (Bash, Write, Edit, apply_patch)
2. Harness sends PreToolUse JSON to hook stdin
3. icg parses JSON, evaluates against rule packs
4. icg outputs JSON decision to stdout
5. Harness reads decision, blocks or allows the operation

#### Input Format

```json
{
  "toolName": "Bash",
  "toolInput": {
    "command": "vault kv destroy secret/test"
  },
  "id": "tool-use-123",
  "timestamp": "2026-08-16T10:30:00Z",
  "sessionId": "session-456"
}
```

#### Output Format

**Deny Response**:
```json
{
  "verdict": "deny",
  "packId": "vault",
  "patternId": "vault-kv-destroy",
  "severity": "Critical",
  "reason": "vault kv destroy is permanently destructive and cannot be undone",
  "rewrite": null,
  "telemetryId": "den-abc123"
}
```

**Allow Response**:
```json
{
  "verdict": "allow",
  "telemetryId": "all-def456"
}
```

### Wrapper Frontend (PATH Symlinks)

The wrapper frontend shadows binaries via symlinks in `$PATH`.

#### How It Works

1. Agent runs `vault kv destroy secret/test`
2. Shell resolves `$PATH` to icg symlink at `/usr/local/bin/vault`
3. icg intercepts argv, evaluates against rule packs
4. icg outputs decision to stderr
5. icg execs the real `vault` binary if allowed
6. icg exits with error code if denied

#### Installation

```bash
# Add wrapper directory to PATH
export PATH="/opt/icg/wrapper:$PATH"

# Create symlinks
ln -s /opt/icg/bin/icg /opt/icg/wrapper/vault
ln -s /opt/icg/bin/icg /opt/icg/wrapper/git
ln -s /opt/icg/bin/icg /opt/icg/wrapper/kubectl
```

#### Wrapper Detection

icg knows it's in wrapper mode when:
- `argv[0]` is a symlink to the icg binary
- Symlink basename matches a tool_keyword in some pack

### Adding a New Frontend

To add support for a new AI harness (e.g., a future Codex variant):

1. **Define the Hook Interface**:
   ```rust
   // src/adapters/new_harness.rs
   pub struct NewHarnessAdapter;

   impl HarnessAdapter for NewHarnessAdapter {
       fn parse_input(&self, stdin: &str) -> Result<Input> {
           // Parse harness-specific JSON format
       }

       fn format_output(&self, decision: &Decision) -> String {
           // Format decision for harness consumption
       }
   }
   ```

2. **Register the Adapter**:
   ```rust
   // src/main.rs
   match harness_type {
       "claude-code" => ClaudeCodeAdapter,
       "codex-cli" => CodexAdapter,
       "new-harness" => NewHarnessAdapter,
       _ => return Err(anyhow!("Unknown harness")),
   }
   ```

3. **Add Tests**:
   ```rust
   #[test]
   fn test_new_harness_adapter() {
       let adapter = NewHarnessAdapter;
       let input = /* harness-specific input */;
       let decision = adapter.evaluate(&input).unwrap();
       assert_eq!(decision.verdict, Verdict::Deny);
   }
   ```

---

## Testing and Validation

### Unit Tests

Test individual components:

```bash
# Run all unit tests
cargo test

# Run specific test
cargo test test_pattern_matching

# Run with output
cargo test -- --nocapture

# Run tests matching a pattern
cargo test pack::
```

### Integration Tests

Test end-to-end workflows:

```bash
# Run integration tests
cargo test --test integration

# Run with specific rule pack
ICG_PACK_PATH=./packs/kubectl/pack.json cargo test
```

### Regression Tests

Validate that destructive patterns remain protected:

```bash
# Generate regression suite
cargo run --bin icg -- regression-suite \
  packs/kubectl/pack.json \
  --output kubectl-regression.json

# Run regression tests
cargo test --test regression

# Verify no coverage narrowing
cargo run --bin icg -- verify-coverage \
  --current kubectl-regression.json \
  --previous previous-kubectl-regression.json
```

### Manual Testing

Test with real commands:

```bash
# Test a specific command
ICG_PACK_PATH=./packs/vault/pack.json \
  cargo run --bin icg -- check \
  --command "vault kv destroy secret/test"

# Test with hook input
echo '{"toolName":"Bash","toolInput":{"command":"vault kv destroy secret/test"}}' | \
  cargo run --bin icg -- check --stdin

# Test in wrapper mode
ln -sf $(cargo root)/target/release/icg /tmp/vault
/tmp/vault kv destroy secret/test
```

### Performance Testing

Measure evaluation latency:

```bash
# Benchmark evaluation
cargo bench --bench evaluation

# Profile hot paths
cargo flamegraph --bin icg -- check \
  --command "vault kv get secret/test"

# Check memory usage
valgrind --tool=massif \
  cargo run --bin icg -- check \
  --command "git log --oneline"
```

---

## Release Process

### Versioning

icg follows Semantic Versioning:

- **Major**: Breaking changes to rule pack schema or evaluation engine
- **Minor**: New features, new rule packs
- **Patch**: Bug fixes, documentation updates

### Release Checklist

1. **Update Version**:
   ```bash
   # Update Cargo.toml
   version = "0.2.0"
   ```

2. **Run Full Test Suite**:
   ```bash
   cargo test --all-features
   cargo clippy --all-targets
   cargo fmt --check
   ```

3. **Generate Regression Suite**:
   ```bash
   cargo run --bin icg -- regression-suite \
     packs/*.json \
     --output regression-suite.json
   ```

4. **Build Release Binary**:
   ```bash
   cargo build --release
   ```

5. **Create Release Notes**:
   ```markdown
   ## Release v0.2.0 (2026-08-16)

   ### Added
   - kubectl rule pack (destructive operations)
   - Terraform content-mode pack
   - Regression suite generation CLI

   ### Changed
   - Improved error messages for pattern matching
   - Updated documentation

   ### Fixed
   - Fixed false positive in git force-push detection
   ```

6. **Tag and Push**:
   ```bash
   git tag -a v0.2.0 -m "Release v0.2.0"
   git push origin v0.2.0
   ```

7. **Publish to GitHub**:
   - Create GitHub Release
   - Upload binary artifacts
   - Attach regression suite

---

## Code Organization

### Module Responsibilities

- **`main.rs`**: CLI entry point, command routing
- **`engine.rs`**: Core evaluation logic, pattern matching
- **`rule_pack.rs`**: Rule pack schema, serialization
- **`state_store.rs`**: Persistent state (session history, Phase 2)
- **`telemetry.rs`**: Metrics, denial logging
- **`health.rs`**: Health check endpoints
- **`denial_log.rs`**: Denial history and trend analysis
- **`overrides.rs`**: Per-repository overrides
- **`regression.rs`**: Regression suite generation and validation
- **`new_pack.rs`**: Rule pack scaffolding CLI
- **`update.rs`**: Rule pack update system
- **`trust_pointer.rs`**: TOFU infrastructure for rule pack updates

### Adding a New Module

1. **Create the module file**:
   ```bash
   touch src/my_module.rs
   ```

2. **Export from lib.rs**:
   ```rust
   // src/lib.rs
   pub mod my_module;
   ```

3. **Write tests**:
   ```rust
   // src/my_module.rs
   #[cfg(test)]
   mod tests {
       use super::*;

       #[test]
       fn test_my_function() {
           // Test implementation
       }
   }
   ```

4. **Document public API**:
   ```rust
   /// Performs a specific operation
   ///
   /// # Arguments
   ///
   /// * `input` - The input to process
   ///
   /// # Returns
   ///
   /// Result containing the output or an error
   pub fn my_function(input: &str) -> Result<String> {
       // Implementation
   }
   ```

---

## Common Patterns

### Pattern: Command Regex Matching

Match shell command tokens:

```rust
use regex::Regex;

fn match_command_regex(pattern: &str, command: &str) -> bool {
    let regex = Regex::new(pattern).unwrap();
    regex.is_match(command)
}
```

### Pattern: Content Regex Matching

Match file content being written:

```rust
fn match_content_regex(pattern: &str, content: &str) -> bool {
    let regex = Regex::new(pattern).unwrap();
    regex.is_match(content)
}
```

### Pattern: File Glob Matching

Match file paths against globs:

```rust
fn matches_glob(path: &str, glob: &str) -> bool {
    // Normalize paths
    let path = path.replace('\\', "/");
    let glob = glob.replace('\\", "/");

    // Handle simple globs (*)
    if glob.contains('*') {
        let parts: Vec<&str> = glob.split('*').collect();
        // Check each part matches
    }

    // Handle recursive globs (**)
    // Handle path separators
    // Handle relative vs absolute paths
}
```

### Pattern: Predicate Evaluation

Custom check functions (future):

```rust
fn evaluate_predicate(name: &str, context: &Context) -> bool {
    match name {
        "is_shared_checkout" => {
            // Check if .git is a directory
            std::path::Path::new(".git").is_dir()
        }
        "has_staged_changes" => {
            // Run git diff --cached
            Command::new("git")
                .args(&["diff", "--cached"])
                .output()
                .map(|o| !o.stdout.is_empty())
                .unwrap_or(false)
        }
        _ => false,
    }
}
```

---

## Contributing

### How to Contribute

1. **Fork the repository**
2. **Create a feature branch**:
   ```bash
   git checkout -b feature/my-rule-pack
   ```
3. **Make your changes**
4. **Add tests**
5. **Update documentation**
6. **Submit a pull request**

### Code Review Process

All contributions go through code review:

1. **Automated Checks**: CI runs tests, clippy, fmt
2. **Peer Review**: Another developer reviews your changes
3. **Architecture Review**: For significant changes
4. **Documentation Review**: Ensure docs are updated

### Coding Standards

- **Rust 2021 Edition**
- **Use `Result` for errors**: Never silently fail
- **Document public APIs**: All public functions need rustdoc
- **Write tests**: Aim for >80% coverage
- **Format code**: Use `cargo fmt`
- **Lint**: Pass `cargo clippy`

---

## Getting Help

### Resources

- **Operator Documentation**: `docs/operators/README.md`
- **Architecture Plan**: `docs/plan/plan.md`
- **Design Notes**: `docs/notes/`
- **GitHub Issues**: https://github.com/jedarden/irreversible-command-gate/issues

### Asking Questions

1. **Search existing issues** first
2. **Create a minimal reproduction** for bugs
3. **Include context**: icg version, OS, rule pack version
4. **Be specific**: What you tried, what you expected, what happened

---

**Developer Documentation Version**: 1.0
**Last Updated**: 2026-08-16
**For**: icg v0.1.0+
