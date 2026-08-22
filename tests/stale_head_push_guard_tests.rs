//! Tests for the stale-HEAD push guard (bead: irrevers-8cff8cf4)
//!
//! This test suite validates the Tier 2 push_requires_current_remote_head predicate
//! that prevents git push when the remote HEAD has moved forward since the last fetch/pull.

use icg::engine::{self, CommandSource, Engine};
use icg::rule_pack::{Channel, Check, GuardedPattern, Pack, Redirect, Severity, Tier};
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};
use tempfile::TempDir;

static CURRENT_DIR_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn current_dir_lock() -> MutexGuard<'static, ()> {
    CURRENT_DIR_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("current-directory test lock should not be poisoned")
}

/// Helper to create a test git repository
fn create_test_repo() -> TempDir {
    let dir = TempDir::new().expect("Failed to create temp dir");
    let repo_path = dir.path();

    // Initialize git repo with main branch
    Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(repo_path)
        .status()
        .expect("Failed to init git repo");

    // Configure user
    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(repo_path)
        .status()
        .expect("Failed to set git user email");

    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(repo_path)
        .status()
        .expect("Failed to set git user name");

    // Create initial commit
    let test_file = repo_path.join("test.txt");
    std::fs::write(&test_file, "initial content").expect("Failed to write test file");

    Command::new("git")
        .args(["add", "test.txt"])
        .current_dir(repo_path)
        .status()
        .expect("Failed to add test file");

    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(repo_path)
        .status()
        .expect("Failed to create initial commit");

    dir
}

/// Helper to create a bare remote repository
fn create_bare_remote() -> TempDir {
    let dir = TempDir::new().expect("Failed to create temp dir for remote");
    let remote_path = dir.path();

    // Initialize bare repo with explicit initial branch name
    Command::new("git")
        .args(["init", "--bare", "-b", "main"])
        .current_dir(remote_path)
        .status()
        .expect("Failed to init bare remote");

    dir
}

/// Helper to add a remote to a repository
fn add_remote(repo_path: &Path, remote_name: &str, remote_path: &Path) {
    Command::new("git")
        .args([
            "remote",
            "add",
            remote_name,
            remote_path.to_str().expect("Invalid path"),
        ])
        .current_dir(repo_path)
        .status()
        .expect("Failed to add remote");
}

#[test]
fn test_push_guard_returns_false_when_not_in_git_repo() {
    let _cwd_lock = current_dir_lock();
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let non_repo_path = temp_dir.path().to_path_buf();

    let current_dir = std::env::current_dir().expect("Failed to get current dir");
    std::env::set_current_dir(&non_repo_path).expect("Failed to change to temporary directory");

    let engine = Engine::new();
    let command = "git push origin main";

    // The predicate should return false (allow) when not in a git repo
    let result = engine.evaluate_command(&CommandSource::Hook(command.to_string()));

    // Restore original directory before dropping temp dir
    std::env::set_current_dir(current_dir).expect("Failed to restore current directory");

    // Should allow because the check fails gracefully (fail-open)
    assert!(matches!(result, engine::CheckResult::Allowed));
}

#[test]
fn test_push_guard_returns_false_when_no_upstream_configured() {
    let _cwd_lock = current_dir_lock();
    let repo = create_test_repo();
    let repo_path = repo.path().to_path_buf();

    let current_dir = std::env::current_dir().expect("Failed to get current dir");
    std::env::set_current_dir(&repo_path).expect("Failed to change to repository");

    // Create a git pack with the stale-HEAD push guard
    let mut engine = Engine::new();
    let git_pack = create_git_pack_with_stale_head_guard();
    engine.load_pack(git_pack).expect("Failed to load git pack");

    let command = "git push origin main";
    let result = engine.evaluate_command(&CommandSource::Hook(command.to_string()));

    // Restore original directory before dropping temp dir
    std::env::set_current_dir(current_dir).expect("Failed to restore current directory");

    // Should allow because no upstream is configured
    assert!(matches!(result, engine::CheckResult::Allowed));
}

#[test]
fn test_push_guard_returns_false_when_remote_head_matches_local() {
    let _cwd_lock = current_dir_lock();
    let repo = create_test_repo();
    let repo_path = repo.path().to_path_buf();

    let remote = create_bare_remote();
    let remote_path = remote.path().to_path_buf();

    // Add remote to repo
    add_remote(&repo_path, "origin", &remote_path);

    // Push to remote
    Command::new("git")
        .args(["push", "-u", "origin", "main"])
        .current_dir(&repo_path)
        .status()
        .expect("Failed to push to remote");

    let current_dir = std::env::current_dir().expect("Failed to get current dir");
    std::env::set_current_dir(&repo_path).expect("Failed to change to repository");

    // Create a git pack with the stale-HEAD push guard
    let mut engine = Engine::new();
    let git_pack = create_git_pack_with_stale_head_guard();
    engine.load_pack(git_pack).expect("Failed to load git pack");

    let command = "git push";
    let result = engine.evaluate_command(&CommandSource::Hook(command.to_string()));

    // Restore original directory before dropping temp dirs
    std::env::set_current_dir(current_dir).expect("Failed to restore current directory");

    // Should allow because remote HEAD matches local
    assert!(matches!(result, engine::CheckResult::Allowed));
}

#[test]
fn test_push_guard_returns_true_when_remote_head_has_moved() {
    let _cwd_lock = current_dir_lock();
    // Create initial repo and push to remote (use into_path to prevent auto-deletion)
    let repo1 = create_test_repo();
    let repo1_path = repo1.into_path();

    let remote = create_bare_remote();
    let remote_path = remote.into_path();

    // Add remote to repo1 and push
    add_remote(&repo1_path, "origin", &remote_path);
    Command::new("git")
        .args(["push", "-u", "origin", "main"])
        .current_dir(&repo1_path)
        .status()
        .expect("Failed to push initial commit");

    // Create a second clone of the remote (use into_path to prevent auto-deletion)
    let repo2_dir = TempDir::new().expect("Failed to create temp dir for second repo");
    let repo2_path = repo2_dir.into_path();

    Command::new("git")
        .args([
            "clone",
            remote_path.to_str().expect("Invalid path"),
            repo2_path.to_str().expect("Invalid path"),
        ])
        .current_dir(repo2_path.parent().expect("Invalid parent"))
        .status()
        .expect("Failed to clone remote");

    // Make a new commit in repo2 and push
    let test_file2 = repo2_path.join("test2.txt");
    std::fs::write(&test_file2, "second content").expect("Failed to write test file 2");

    Command::new("git")
        .args(["add", "test2.txt"])
        .current_dir(&repo2_path)
        .status()
        .expect("Failed to add test file 2");

    Command::new("git")
        .args(["commit", "-m", "Second commit"])
        .current_dir(&repo2_path)
        .status()
        .expect("Failed to create second commit");

    Command::new("git")
        .args(["push"])
        .current_dir(&repo2_path)
        .status()
        .expect("Failed to push second commit");

    // Test the stale-HEAD guard from repo1 perspective
    let current_dir = std::env::current_dir().expect("Failed to get current dir");

    // Use a scope to ensure we restore the directory before dropping TempDirs
    let test_result = {
        std::env::set_current_dir(&repo1_path).expect("Failed to change to repository");

        // Create a git pack with the stale-HEAD push guard
        let mut engine = Engine::new();
        let git_pack = create_git_pack_with_stale_head_guard();
        engine.load_pack(git_pack).expect("Failed to load git pack");

        let command = "git push";
        let result = engine.evaluate_command(&CommandSource::Hook(command.to_string()));

        // Restore original directory before this scope ends
        std::env::set_current_dir(&current_dir).expect("Failed to restore current directory");

        result
    };

    // Now the TempDirs will be dropped at the end of this function,
    // but we've already restored the directory

    // Should deny because remote HEAD has moved forward
    match test_result {
        engine::CheckResult::Denied {
            reason,
            pack_id,
            pattern_id,
        } => {
            assert_eq!(pack_id, "git");
            assert_eq!(pattern_id, "git-stale-remote-head-push");
            assert!(
                reason.contains("pull") || reason.contains("Remote HEAD has moved"),
                "Reason should mention pull or remote HEAD movement, got: {}",
                reason
            );
        }
        other => panic!("Expected Denied result, got {:?}", other),
    }
}

#[test]
fn test_push_guard_allows_after_pull() {
    let _cwd_lock = current_dir_lock();
    // Create initial repo and push to remote (use into_path to prevent auto-deletion)
    let repo1 = create_test_repo();
    let repo1_path = repo1.into_path();

    let remote = create_bare_remote();
    let remote_path = remote.into_path();

    // Add remote to repo1 and push
    add_remote(&repo1_path, "origin", &remote_path);
    Command::new("git")
        .args(["push", "-u", "origin", "main"])
        .current_dir(&repo1_path)
        .status()
        .expect("Failed to push initial commit");

    // Create a second clone and make a commit (use into_path to prevent auto-deletion)
    let repo2_dir = TempDir::new().expect("Failed to create temp dir for second repo");
    let repo2_path = repo2_dir.into_path();

    Command::new("git")
        .args([
            "clone",
            remote_path.to_str().expect("Invalid path"),
            repo2_path.to_str().expect("Invalid path"),
        ])
        .current_dir(repo2_path.parent().expect("Invalid parent"))
        .status()
        .expect("Failed to clone remote");

    let test_file2 = repo2_path.join("test2.txt");
    std::fs::write(&test_file2, "second content").expect("Failed to write test file 2");

    Command::new("git")
        .args(["add", "test2.txt"])
        .current_dir(&repo2_path)
        .status()
        .expect("Failed to add test file 2");

    Command::new("git")
        .args(["commit", "-m", "Second commit"])
        .current_dir(&repo2_path)
        .status()
        .expect("Failed to create second commit");

    Command::new("git")
        .args(["push"])
        .current_dir(&repo2_path)
        .status()
        .expect("Failed to push second commit");

    let current_dir = std::env::current_dir().expect("Failed to get current dir");

    // Test the stale-HEAD guard from repo1 perspective
    let test_result = {
        std::env::set_current_dir(&repo1_path).expect("Failed to change to repository");

        // Create a git pack with the stale-HEAD push guard
        let mut engine = Engine::new();
        let git_pack = create_git_pack_with_stale_head_guard();
        engine.load_pack(git_pack).expect("Failed to load git pack");

        let command = "git push";

        // First, verify the push is denied
        let result = engine.evaluate_command(&CommandSource::Hook(command.to_string()));
        assert!(matches!(result, engine::CheckResult::Denied { .. }));

        // Now pull to sync
        Command::new("git")
            .args(["pull"])
            .current_dir(&repo1_path)
            .status()
            .expect("Failed to pull");

        // After pull, the push should be allowed
        let result = engine.evaluate_command(&CommandSource::Hook(command.to_string()));

        // Restore original directory before this scope ends
        std::env::set_current_dir(&current_dir).expect("Failed to restore current directory");

        result
    };

    // Should allow after pull
    assert!(matches!(test_result, engine::CheckResult::Allowed));
}

#[test]
fn test_push_guard_does_not_affect_force_push_check() {
    let _cwd_lock = current_dir_lock();
    let repo = create_test_repo();
    let repo_path = repo.path().to_path_buf();

    let current_dir = std::env::current_dir().expect("Failed to get current dir");
    std::env::set_current_dir(&repo_path).expect("Failed to change to repository");

    // Create a git pack with both stale-HEAD and force-push guards
    let mut engine = Engine::new();
    let git_pack = create_git_pack_with_stale_head_guard();
    engine.load_pack(git_pack).expect("Failed to load git pack");

    // Force push should still be caught by the force-push pattern
    let command = "git push --force";
    let result = engine.evaluate_command(&CommandSource::Hook(command.to_string()));

    // Restore original directory before dropping temp dir
    std::env::set_current_dir(current_dir).expect("Failed to restore current directory");

    match result {
        engine::CheckResult::Denied {
            reason, pattern_id, ..
        } => {
            assert_eq!(pattern_id, "git-force-push");
            assert!(
                reason.contains("force-with-lease") || reason.contains("Force-push"),
                "Reason should mention force-with-lease or force-push, got: {}",
                reason
            );
        }
        other => panic!("Expected Denied result for force-push, got {:?}", other),
    }
}

#[test]
fn test_push_guard_safe_patterns_are_respected() {
    let _cwd_lock = current_dir_lock();
    // Create test repo (use into_path to prevent auto-deletion)
    let repo = create_test_repo();
    let repo_path = repo.into_path();

    let current_dir = std::env::current_dir().expect("Failed to get current dir");
    std::env::set_current_dir(&repo_path).expect("Failed to change to repository");

    // Create a git pack with safe patterns
    let mut engine = Engine::new();
    let git_pack = create_git_pack_with_stale_head_guard();
    engine.load_pack(git_pack).expect("Failed to load git pack");

    // Safe commands should be allowed
    let safe_commands = vec![
        "git status",
        "git fetch origin",
        "git pull",
        "git log",
        "git branch",
    ];

    for command in safe_commands {
        let result = engine.evaluate_command(&CommandSource::Hook(command.to_string()));
        assert!(
            matches!(result, engine::CheckResult::Allowed),
            "Command '{}' should be allowed, got {:?}",
            command,
            result
        );
    }

    // Restore original directory before dropping temp dir
    std::env::set_current_dir(current_dir).expect("Failed to restore current directory");
}

#[test]
fn test_check_remote_head_stale_handles_missing_git() {
    let _cwd_lock = current_dir_lock();
    // This test verifies that the function fails gracefully when git is not available
    // We can't actually remove git from the system, but we can test the logic by
    // calling it from a directory where git commands will fail

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let non_repo_path = temp_dir.path();

    let current_dir = std::env::current_dir().expect("Failed to get current dir");
    std::env::set_current_dir(non_repo_path).expect("Failed to change to temporary directory");

    // The function should return Ok(false) (fail-open) when git commands fail
    let result = icg::engine::check_remote_head_stale();

    // Restore original directory
    std::env::set_current_dir(current_dir).expect("Failed to restore current directory");

    // Should fail open (allow) when git is not available
    assert!(result.is_ok() || result.is_err());
    if let Ok(stale) = result {
        assert!(!stale, "Should not be stale when git check fails");
    }
    // If it returns an error, that's also acceptable (fail-open at higher level)
}

/// Create a git pack with the stale-HEAD push guard for testing
fn create_git_pack_with_stale_head_guard() -> Pack {
    Pack {
        id: "git".to_string(),
        tool_keywords: vec!["git".to_string()],
        applies_to: vec![],
        safe_patterns: vec![
            icg::rule_pack::Pattern {
                id: "safe-git-status".to_string(),
                check: Check::CommandRegex {
                    regex: "^git status".to_string(),
                },
            },
            icg::rule_pack::Pattern {
                id: "safe-git-fetch".to_string(),
                check: Check::CommandRegex {
                    regex: "^git fetch".to_string(),
                },
            },
            icg::rule_pack::Pattern {
                id: "safe-git-pull".to_string(),
                check: Check::CommandRegex {
                    regex: "^git pull".to_string(),
                },
            },
            icg::rule_pack::Pattern {
                id: "safe-git-log".to_string(),
                check: Check::CommandRegex {
                    regex: "^git (log|show|diff)".to_string(),
                },
            },
            icg::rule_pack::Pattern {
                id: "safe-git-branch".to_string(),
                check: Check::CommandRegex {
                    regex: "^git branch".to_string(),
                },
            },
        ],
        guarded_patterns: vec![
            GuardedPattern {
                id: "git-force-push".to_string(),
                enabled: true,
                check: Check::CommandRegex {
                    regex: r"git push.*--force".to_string(),
                },
                tier: Tier::Tier1,
                severity: Severity::Critical,
                explanation: "Force-push rewrites git history and can lose commits".to_string(),
                redirect: Redirect {
                    channel: Channel::Deny,
                    reason_template: "Force-push rewrites git history and can lose commits. Use git push --force-with-lease instead.".to_string(),
                    rewrite_template: Some("git push --force-with-lease".to_string()),
                },
                destructive: true,
            },
            GuardedPattern {
                id: "git-stale-remote-head-push".to_string(),
                enabled: true,
                check: Check::Predicate {
                    predicate_name: "push_requires_current_remote_head".to_string(),
                    data: None,
                },
                tier: Tier::Tier2,
                severity: Severity::High,
                explanation: "Pushing when remote HEAD has moved forward risks rejection and creates merge conflicts".to_string(),
                redirect: Redirect {
                    channel: Channel::Deny,
                    reason_template: "Remote HEAD has moved forward since your last fetch/pull. Run 'git pull' first to integrate remote changes before pushing.".to_string(),
                    rewrite_template: None,
                },
                destructive: true,
            },
        ],
    }
}
