//! Capability-based filesystem sandbox built on `cap-std`.
//!
//! [`Sandbox`] holds a [`cap_std::fs::Dir`] capability handle opened **once**
//! on the working root. Every subsequent file operation goes through that
//! handle: on Linux, cap-std resolves paths with `openat2(2)` +
//! `RESOLVE_BENEATH`, so the kernel atomically rejects any path that escapes
//! the root — symlink escapes and TOCTOU races are impossible because the
//! whole resolution happens in a single syscall. There is no "validate then
//! open" window to race.
//!
//! This replaces the previous lexical blacklist (`safe_join_rel` rejecting
//! `..` / absolute paths): that approach was bypassable with a symlink placed
//! *inside* the root pointing outside (e.g. `root/link -> /etc`), or with a
//! TOCTOU swap of a directory for a symlink between validation and open.
//! With a capability handle the open itself is the boundary.

use std::path::{Path, PathBuf};

use cap_std::ambient_authority;
use cap_std::fs::Dir;

use crate::error::{Error, Result};

/// A capability handle to a sandbox root directory.
///
/// `Debug` intentionally reveals nothing about the underlying path.
pub struct Sandbox {
    dir: Dir,
}

impl Clone for Sandbox {
    fn clone(&self) -> Self {
        Self {
            dir: self
                .dir
                .try_clone()
                .expect("capability handle clone must not fail"),
        }
    }
}

/// One entry inside a sandboxed directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxEntry {
    pub name: String,
    pub is_dir: bool,
}

impl Sandbox {
    /// Open a capability handle on `root`. This is the **only** ambient
    /// (unsandboxed) operation; everything after goes through the handle.
    pub fn open(root: &Path) -> Result<Self> {
        let dir = Dir::open_ambient_dir(root, ambient_authority()).map_err(|e| {
            Error::InvalidInput(format!("cannot open sandbox root {}: {e}", root.display()))
        })?;
        Ok(Self { dir })
    }

    /// Open a capability handle on a *subdirectory* of this sandbox,
    /// still confined beneath the root.
    pub fn open_sub(&self, rel: &str) -> Result<Self> {
        let sub = self
            .dir
            .open_dir(rel)
            .map_err(|e| Error::InvalidInput(format!("cannot open sandbox subdir {rel:?}: {e}")))?;
        Ok(Self { dir: sub })
    }

    /// Read a whole file into memory, confined to the sandbox.
    pub fn read(&self, rel: &str) -> Result<Vec<u8>> {
        self.dir
            .read(rel)
            .map_err(|e| Error::InvalidInput(format!("cannot read {rel:?}: {e}")))
    }

    /// Read a file as UTF-8 (lossy, like the previous `std::fs::read_to_string`).
    pub fn read_to_string(&self, rel: &str) -> Result<String> {
        let bytes = self.read(rel)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Metadata for a path inside the sandbox.
    pub fn metadata(&self, rel: &str) -> Result<cap_std::fs::Metadata> {
        self.dir
            .metadata(rel)
            .map_err(|e| Error::InvalidInput(format!("cannot stat {rel:?}: {e}")))
    }

    /// True if the path exists inside the sandbox (never follows an escaping
    /// symlink — the kernel answers for the capability).
    pub fn exists(&self, rel: &str) -> bool {
        self.dir.try_exists(rel).unwrap_or(false)
    }

    /// Create a directory (and parents) inside the sandbox.
    pub fn create_dir_all(&self, rel: &str) -> Result<()> {
        self.dir
            .create_dir_all(rel)
            .map_err(|e| Error::InvalidInput(format!("cannot create dir {rel:?}: {e}")))
    }

    /// List entries of a directory inside the sandbox. `rel == ""` lists the
    /// sandbox root itself.
    pub fn read_dir(&self, rel: &str) -> Result<Vec<SandboxEntry>> {
        let rd = if rel.is_empty() {
            self.dir.entries()
        } else {
            self.dir.read_dir(rel)
        }
        .map_err(|e| Error::InvalidInput(format!("cannot list {rel:?}: {e}")))?;
        let mut out = Vec::new();
        for entry in rd {
            let entry =
                entry.map_err(|e| Error::InvalidInput(format!("list {rel:?} entry: {e}")))?;
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            out.push(SandboxEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir,
            });
        }
        Ok(out)
    }

    /// Atomically write `contents` to `rel` inside the sandbox: write to a
    /// sibling temp file, then rename over the destination. Both operations
    /// are relative to the capability handle, so a temp-file path can never
    /// escape either.
    pub fn atomic_write(&self, rel: &str, contents: &[u8]) -> Result<()> {
        let parent = rel.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
        let name = rel.rsplit('/').next().unwrap_or(rel);
        if name.is_empty() {
            return Err(Error::InvalidInput(format!(
                "invalid empty file name in {rel:?}"
            )));
        }
        self.create_dir_all(parent)?;
        let tmp = if parent.is_empty() {
            format!(".{name}.aether-tmp")
        } else {
            format!("{parent}/.{name}.aether-tmp")
        };
        // cap-std `write` opens with O_CREAT|O_TRUNC beneath the handle.
        self.dir
            .write(&tmp, contents)
            .map_err(|e| Error::InvalidInput(format!("cannot write {rel:?}: {e}")))?;
        self.dir
            .rename(&tmp, &self.dir, rel)
            .map_err(|e| Error::InvalidInput(format!("cannot finalize {rel:?}: {e}")))?;
        Ok(())
    }

    /// Remove a file inside the sandbox.
    pub fn remove_file(&self, rel: &str) -> Result<()> {
        self.dir
            .remove_file(rel)
            .map_err(|e| Error::InvalidInput(format!("cannot remove {rel:?}: {e}")))
    }

    /// The canonical absolute path of the sandbox root, for display and for
    /// `run_command`'s working directory. Resolution is done once here; the
    /// handle itself is the security boundary, not this string.
    pub fn root_path(&self) -> Result<PathBuf> {
        let p = self
            .dir
            .canonicalize("")
            .map_err(|e| Error::InvalidInput(format!("cannot canonicalize sandbox root: {e}")))?;
        Ok(p)
    }

    /// Recursively visit files (not dirs) beneath `rel`, confined to the
    /// sandbox. `f` receives the relative path of each file.
    pub fn walk_files(&self, rel: &str, f: &mut dyn FnMut(&str) -> Result<()>) -> Result<()> {
        let entries = self.read_dir(rel)?;
        for entry in entries {
            let child = if rel.is_empty() {
                entry.name.clone()
            } else {
                format!("{rel}/{}", entry.name)
            };
            if entry.is_dir {
                let name = &entry.name;
                if name == ".git" || name == "target" || name == "node_modules" {
                    continue;
                }
                self.walk_files(&child, f)?;
            } else {
                f(&child)?;
            }
        }
        Ok(())
    }
}

impl std::fmt::Debug for Sandbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sandbox").finish_non_exhaustive()
    }
}
