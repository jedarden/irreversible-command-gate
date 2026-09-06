# Examples Coverage Analysis

## Summary of 12 Documented Scenarios

### Operator Scenarios (1-5)

| Scenario | Title | Test File | CLI Commands Documented | CLI Commands Implemented | Status |
|----------|-------|-----------|------------------------|-------------------------|---------|
| 1 | First-time Installation | NONE | `wget`, `tar`, `sudo cp`, `icg --version`, `icg health --check-packs`, `icg health --check-hooks` | `--version`, `health status`, `health --check-packs` | **PARTIAL** - No tests, missing `--check-hooks` |
| 2 | Daily Operations | NONE | `icg status --denials --since 1h`, `icg status --denials --pattern-summary --since 7d`, `icg status --denials --since 1h --format json` | `status` (generic, no `--denials` flag) | **MISSING** - No `--denials` flag, no tests |
| 3 | Handling Denials | NONE | `icg explain --pattern vault-kv-destroy` | `explain --pattern` | **MISSING** - Command exists but no tests |
| 4 | Emergency Response | `emergency_response_tests.rs` | `icg status --health`, `ICG_DISABLED=1` env var, `icg export-denial` | `status`, `health status` | **PARTIAL** - Tests exist but `ICG_DISABLED` and `export-denial` not implemented |
| 5 | Maintenance Tasks | NONE | `icg health --verbose`, `icg status --denials --trend --since 30d`, `icg update --check-only`, `icg backup create`, `icg backup verify` | `health status`, `status`, `update`, `backup create`, `backup verify` | **PARTIAL** - Commands exist but no tests, missing `--trend` and `--check-only` flags |

### Developer Scenarios (6-9)

| Scenario | Title | Test File | CLI Commands Documented | CLI Commands Implemented | Status |
|----------|-------|-----------|------------------------|-------------------------|---------|
| 6 | Creating a New Rule Pack | NONE | `cargo run --new-pack`, `cargo test kubectl`, `icg regression-suite`, `icg check --command` | `new-pack`, `regression-suite`, `check --command` | **PARTIAL** - Commands exist but no scenario-specific tests |
| 7 | Testing Pattern Changes | `regression_suite_tests.rs` | `icg regression-suite`, `icg verify-coverage`, `icg check --command` | `regression-suite`, `check --command` | **MISSING** - Missing `verify-coverage` command |
| 8 | Debugging False Positives | `debugging_false_positives_tests.rs` | `icg check --command --debug` | `check --command` (no `--debug` flag) | **PARTIAL** - Tests exist but `--debug` flag not implemented |
| 9 | Adding Custom Predicates | NONE | (Code changes, not CLI) | N/A - Code examples only | **MISSING** - No tests for predicate patterns |

### Integration Scenarios (10-12)

| Scenario | Title | Test File | CLI Commands Documented | CLI Commands Implemented | Status |
|----------|-------|-----------|------------------------|-------------------------|---------|
| 10 | Migrating from org-rule-guard.py | `coexistence_org_rule_guard_tests.rs` | `icg coverage --list`, hook configuration | `coverage --list`, hook mode | **COMPLETE** - Tests exist |
| 11 | Setting up Multi-Harness Support | `codex_hook_tests.rs` | `icg check --stdin --harness claude-code`, `icg check --stdin --harness codex-cli`, `icg status --denials --by-harness` | `check --stdin`, (no `--harness` flag), (no `--by-harness` flag) | **PARTIAL** - Tests exist but missing harness flags |
| 12 | Configuring Repository Overrides | NONE | `icg override create`, `icg override approve`, `icg override list` | `override create`, `override approve`, `override list` | **MISSING** - Commands exist but no scenario tests |

## Test Files Coverage

### Existing Test Files
1. ✅ `emergency_response_tests.rs` - Scenario 4
2. ✅ `debugging_false_positives_tests.rs` - Scenario 8
3. ✅ `coexistence_org_rule_guard_tests.rs` - Scenario 10
4. ✅ `codex_hook_tests.rs` - Scenario 11 (partial)
5. ✅ `regression_suite_tests.rs` - Scenario 7 (partial)
6. ✅ `documented_commands_tests.rs` - General CLI commands

### Missing Test Files
1. ❌ Scenario 1: First-time Installation
2. ❌ Scenario 2: Daily Operations
3. ❌ Scenario 3: Handling Denials
4. ❌ Scenario 5: Maintenance Tasks
5. ❌ Scenario 6: Creating a New Rule Pack (scenario-specific)
6. ❌ Scenario 7: Testing Pattern Changes (missing `verify-coverage` command)
7. ❌ Scenario 9: Adding Custom Predicates
8. ❌ Scenario 11: Multi-Harness Support (partial coverage)
9. ❌ Scenario 12: Configuring Repository Overrides

## Missing CLI Features

### Documented but Not Implemented
1. `icg health --check-hooks` - Not implemented
2. `icg status --denials` - Not implemented
3. `icg status --denials --pattern-summary` - Not implemented
4. `icg status --denials --trend` - Not implemented
5. `icg status --denials --by-harness` - Not implemented
6. `icg check --stdin --harness` - Not implemented
7. `icg check --command --debug` - Not implemented
8. `icg verify-coverage` - Not implemented
9. `icg update --check-only` - Not implemented
10. `ICG_DISABLED` environment variable bypass - Not implemented
11. `icg export-denial` - Not implemented

## Recommendations

1. **Immediate**: Create test files for scenarios that have implemented commands but no tests
2. **Short-term**: Implement missing CLI flags and commands
3. **Long-term**: Consider whether some documented features should be removed or implemented

## Fix Priority

### High Priority (Scenarios with partial implementation)
- Create tests for Scenarios 1, 3, 5, 6, 12
- These have working commands but no test coverage

### Medium Priority (Missing CLI features)
- Implement `--denials` flag for status command (Scenario 2)
- Implement `--debug` flag for check command (Scenario 8)
- Implement `--harness` flag (Scenario 11)

### Low Priority (Complex features)
- Consider whether `ICG_DISABLED` bypass should be implemented (Scenario 4)
- Consider whether `verify-coverage` command is needed (Scenario 7)
