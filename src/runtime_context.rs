//! Runtime context shared by the operational sinks.
//!
//! The host's operational cache (`/var/cache/icg/`) is shared state: on an
//! instrumented host it is live traffic, written by every guarded tool call
//! from every session. Two suites of records have ended up in it from
//! `cargo test`:
//!
//! - **2026-09-07, denials**: a practice trial found 14 of its 72 denial
//!   records were produced by tests; a fixture exercising a *real* pattern id
//!   is indistinguishable from a real denial after the fact, and the whole
//!   trial window had to be re-baselined. Guarded in
//!   [`crate::denial_log::operational_log_path`].
//! - **2026-09-13, health and telemetry**: the production crash history was
//!   382 records, and every retained record carried a test context --
//!   `could not find the real \`fake_tool\` binary` from wrapper tests,
//!   `guard availability failure during evaluation` from hook tests -- because
//!   test-spawned guard invocations fell through
//!   [`crate::health::HealthStore::from_environment_or_default`] to the
//!   production path. Guarded in that constructor and
//!   [`crate::telemetry::operational_store_path`].
//!
//! Both guards share this module's detection: per-test discipline (set the
//! variable in every deny-path test) had already been tried and leaked -- the
//! refusal has to live at the one place every operational write passes
//! through, not in each of the dozens of test files that can trigger one.

use std::path::PathBuf;
use std::sync::OnceLock;

/// Is the current process a Rust test binary, or one cargo launched for a
/// test binary?
///
/// `cargo test` drives two kinds of process that can reach an operational
/// write:
///
/// 1. **The test binary itself** -- a unit test compiled into this crate, or
///    an integration test under `tests/` that calls into the library
///    in-process. It runs with cargo's environment and libtest's argv.
/// 2. **The binaries an integration test spawns** through
///    `CARGO_BIN_EXE_icg`. Those are unmodified production binaries invoked
///    as `icg hook` or `icg wrapper`, so no compile-time `cfg` distinguishes
///    them -- but they inherit cargo's environment, and cargo sets
///    `CARGO_BIN_EXE_<name>` only while running that crate's tests.
///
/// Detection deliberately stops short of "the `CARGO` variable is set":
/// `cargo run -- check --command ...` is documented developer usage of the
/// real front end and should keep recording like one.
pub(crate) fn process_is_test_driven() -> bool {
    // Unit tests of this crate, compiled with the library.
    if cfg!(test) {
        return true;
    }
    // This crate's integration-test harness, and every child it spawns.
    if std::env::var_os("CARGO_BIN_EXE_icg").is_some() {
        return true;
    }
    // A test binary cargo built for another crate that links this library as
    // a dependency: cargo puts test executables in target/<profile>/deps/,
    // installed and `cargo run` binaries are never there, and libtest's flags
    // never appear in a hook or wrapper argv. The argv scan is `args_os`
    // because a hook invocation may carry non-UTF-8 arguments and this must
    // never panic on the production path.
    const LIBTEST_FLAGS: [&str; 3] = ["--nocapture", "--show-output", "--list"];
    let is_libtest_flag = |arg: std::ffi::OsString| {
        let arg: &str = &arg.to_string_lossy();
        // libtest accepts both `--test-threads 2` and `--test-threads=2`; the
        // latter is how this repo's own suites pass the flag.
        LIBTEST_FLAGS.contains(&arg) || arg.starts_with("--test-threads")
    };
    if std::env::args_os().any(is_libtest_flag) {
        return true;
    }
    std::env::args_os()
        .next()
        .is_some_and(|argv0| argv0.to_string_lossy().contains("/deps/"))
}

/// Return a per-process scratch path for an operational file when the current
/// process is test-driven, or [`None`] for a real deployment.
///
/// A test-driven caller that has not named an explicit path gets this scratch
/// path instead of the host's live cache. Relocation rather than refusal:
/// unlike a denial record, health tracking's value depends on a *sequence* of
/// writes (run markers, crash recovery, clean-exit counting), so the test
/// harness gets a sink that behaves like the real one -- it just lives under
/// the platform temp directory where it cannot pollute operator data. Each
/// process gets its own directory (pid plus a startup timestamp), so a test
/// binary and the `icg` children it spawns never share run markers, and no
/// run of the suite inherits a previous run's stale-marker crash.
///
/// The directory is deliberately not cleaned up on exit: a guard process that
/// vanishes mid-run must leave exactly the kind of durable evidence the
/// run-marker recovery expects, and a few kilobytes per suite run in the
/// platform temp directory is the price of keeping that behavior honest.
pub(crate) fn test_operational_path(file_name: &str) -> Option<PathBuf> {
    if !process_is_test_driven() {
        return None;
    }
    Some(test_operational_root().join(file_name))
}

/// The per-process scratch directory, resolved once per process.
fn test_operational_root() -> &'static PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        std::env::temp_dir().join(format!(
            "icg-test-operational-{}-{nanos}",
            std::process::id()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unit_test_process_is_test_driven() {
        // Compiled into the library under cfg(test): the first branch fires.
        assert!(process_is_test_driven());
    }

    #[test]
    fn test_operational_path_lands_under_the_temp_dir() {
        let path = test_operational_path("health-state.json").expect("unit tests are test-driven");
        assert!(path.starts_with(std::env::temp_dir()));
        assert!(path.ends_with("health-state.json"));
    }

    #[test]
    fn the_scratch_root_is_stable_within_a_process() {
        let first = test_operational_path("a.json").expect("unit tests are test-driven");
        let second = test_operational_path("b.json").expect("unit tests are test-driven");
        assert_eq!(first.parent(), second.parent());
    }
}
