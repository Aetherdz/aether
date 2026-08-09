//! Filesystem utilities: safe path joining, directory creation, atomic writes.
//!
//! Security: every function that builds a path from user input goes through
//! [`safe_join`], which rejects path traversal (`..`, separators, absolute
//! paths, NUL bytes).

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Join `name` onto `base`, rejecting any path-traversal attempt.
///
/// Rejected inputs: empty, `.`, `..`, anything containing `/`, `\` or a NUL
/// byte. This guarantees the result stays inside `base`.
pub fn safe_join(base: &Path, name: &str) -> Result<PathBuf> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return Err(Error::PathTraversal(name.to_string()));
    }
    Ok(base.join(name))
}

/// Create a directory (and parents) if it does not exist.
pub fn ensure_dir(path: &Path) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(path)?;
    Ok(())
}

/// Atomically write `contents` to `path`.
///
/// Writes to a temporary sibling file, fsyncs it, then renames over the
/// destination so readers never observe a partially-written file.
pub fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| Error::InvalidInput(format!("path has no parent: {}", path.display())))?;
    ensure_dir(dir)?;
    let file_name = path
        .file_name()
        .ok_or_else(|| Error::InvalidInput(format!("path has no file name: {}", path.display())))?;
    let tmp = dir.join(format!(".{}.aether-tmp", file_name.to_string_lossy()));
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(contents)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{atomic_write, ensure_dir, safe_join};
    use crate::error::Error;
    use std::path::Path;

    #[test]
    fn safe_join_rejects_traversal() {
        let base = Path::new("/tmp/aether-test");
        for bad in ["..", "../x", "a/b", "a\\b", "/etc/passwd", ".", "", "a\0b"] {
            assert!(
                matches!(safe_join(base, bad), Err(Error::PathTraversal(_))),
                "expected traversal rejection for {bad:?}"
            );
        }
        assert_eq!(safe_join(base, "ok.txt").unwrap(), base.join("ok.txt"));
    }

    #[test]
    fn atomic_write_round_trips() {
        let dir = std::env::temp_dir().join(format!("aether-fs-test-{}", std::process::id()));
        let path = dir.join("out.txt");
        atomic_write(&path, b"hello").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
        // Overwrite works too.
        atomic_write(&path, b"world").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "world");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_dir_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("aether-ensure-{}", std::process::id()));
        ensure_dir(&dir).unwrap();
        ensure_dir(&dir).unwrap();
        assert!(dir.is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
