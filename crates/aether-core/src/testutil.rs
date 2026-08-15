//! Workspace-wide test serialization for env-mutating tests.
//!
//! Several crates' tests point the process-global `AETHER_CONFIG_DIR` at a
//! scratch dir. A per-crate `static Mutex` serializes each crate's own tests
//! but *not* across crates, so `cargo test --workspace` (which runs crate
//! test binaries in parallel) races on the variable and flakes. This single
//! lock is shared by every crate via `aether_core`, so all env-mutating
//! tests in the workspace are serialized against each other.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Acquire the workspace-wide env lock. Every test that mutates
/// `AETHER_CONFIG_DIR` must hold this while it runs.
pub fn lock_env() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Create a fresh scratch dir under the system temp dir and point
/// `AETHER_CONFIG_DIR` at it. Callers must hold the lock from
/// [`lock_env`] for the whole test.
///
/// # Safety
///
/// `set_var` is unsafe in edition 2024; the lock makes it sound here.
pub fn test_env(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("aether-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: caller holds the workspace env lock.
    unsafe { std::env::set_var("AETHER_CONFIG_DIR", &dir) };
    dir
}
