//! Persistent undo journal for `write_file`.
//!
//! Every time `write_file` replaces an existing file, the previous content
//! is snapshotted into `<root>/.aether-undo/NNNN.snap` on disk. The journal
//! survives the agent process, so `aether undo` can restore files later.
//!
//! Security: the journal operates exclusively through a [`Sandbox`]
//! capability handle opened on the sandbox root. On Linux that handle
//! resolves with `openat2(RESOLVE_BENEATH)`, so a snapshot path or a
//! restore target can never escape the root — symlink tricks and TOCTOU
//! races are rejected by the kernel at the open itself. `safe_join_rel`
//! remains as a *lexical pre-check* purely to produce a clear
//! [`Error::PathTraversal`] message early; it is not the security boundary.
//! The journal is never exposed to the model as an arbitrary file-access
//! tool.

use std::path::{Path, PathBuf};

use aether_core::error::{Error, Result};
use aether_core::fs::safe_join_rel;
use aether_core::sandbox::Sandbox;
use serde::{Deserialize, Serialize};

/// Directory (relative to the sandbox root) that holds snapshots.
pub const UNDO_DIR_NAME: &str = ".aether-undo";

/// Metadata about one stored snapshot (no content).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotMeta {
    pub seq: u32,
    pub rel: String,
    pub bytes: usize,
}

/// Result of a successful restore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoredSnapshot {
    pub seq: u32,
    pub rel: String,
    pub bytes: usize,
}

/// On-disk snapshot file: `{ "seq": 1, "rel": "a/b.txt", "content": "...", "was_new": false }`.
///
/// `was_new` records whether the file did not exist before the write; undo
/// then *deletes* it instead of restoring a phantom previous content.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SnapFile {
    seq: u32,
    rel: String,
    content: String,
    #[serde(default)]
    was_new: bool,
}

/// Disk-backed snapshot journal rooted at `<sandbox_root>/.aether-undo`.
///
/// All filesystem access goes through a capability [`Sandbox`]; the
/// `dir` field is kept only for human-readable display.
#[derive(Debug, Clone)]
pub struct UndoJournal {
    sandbox: Sandbox,
    dir: PathBuf,
}

impl UndoJournal {
    /// Open the journal for the sandbox rooted at `root` (which must exist).
    pub fn new(root: &Path) -> Self {
        let sandbox =
            Sandbox::open(root).expect("undo journal requires an existing, openable sandbox root");
        Self {
            dir: root.join(UNDO_DIR_NAME),
            sandbox,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The sandbox root this journal lives under.
    fn root(&self) -> &Path {
        self.dir.parent().expect("undo dir always has a parent")
    }

    /// Relative path of a snapshot file inside the sandbox.
    fn snap_rel(&self, seq: u32) -> String {
        format!("{UNDO_DIR_NAME}/{seq:04}.snap")
    }

    /// Save `content` as the newest snapshot for `rel`. Returns the seq.
    pub fn snapshot(&self, rel: &str, content: &str, was_new: bool) -> Result<u32> {
        // Lexical pre-check for a clear error message; the capability handle
        // below is the actual boundary.
        safe_join_rel(self.root(), rel)?;
        let seq = self.next_seq()?;
        let snap = SnapFile {
            seq,
            rel: rel.to_string(),
            content: content.to_string(),
            was_new,
        };
        let json = serde_json::to_vec(&snap)?;
        self.sandbox.atomic_write(&self.snap_rel(seq), &json)?;
        Ok(seq)
    }

    /// List all snapshots, oldest first.
    pub fn list(&self) -> Result<Vec<SnapshotMeta>> {
        let mut snaps = self.read_all()?;
        snaps.sort_by_key(|s| s.seq);
        Ok(snaps
            .into_iter()
            .map(|s| SnapshotMeta {
                seq: s.seq,
                rel: s.rel,
                bytes: s.content.len(),
            })
            .collect())
    }

    /// Restore `rel` to its most recent snapshot (and pop that snapshot so
    /// repeated undos walk back through history).
    pub fn restore(&self, rel: &str) -> Result<RestoredSnapshot> {
        // Lexical pre-check for a clear error message; the capability handle
        // below is the actual boundary.
        safe_join_rel(self.root(), rel)?;
        let mut snaps = self.read_all()?;
        snaps.sort_by_key(|s| s.seq);
        let snap = snaps
            .into_iter()
            .rev()
            .find(|s| s.rel == rel)
            .ok_or_else(|| Error::InvalidInput(format!("no snapshot for {rel:?}")))?;
        if snap.was_new {
            self.sandbox.remove_file(rel)?;
        } else {
            self.sandbox.atomic_write(rel, snap.content.as_bytes())?;
        }
        let restored = RestoredSnapshot {
            seq: snap.seq,
            rel: snap.rel.clone(),
            bytes: snap.content.len(),
        };
        self.sandbox.remove_file(&self.snap_rel(snap.seq))?;
        Ok(restored)
    }

    /// Read and parse every snapshot file currently on disk.
    fn read_all(&self) -> Result<Vec<SnapFile>> {
        let mut snaps = Vec::new();
        if !self.sandbox.exists(UNDO_DIR_NAME) {
            return Ok(snaps);
        }
        let entries = self.sandbox.read_dir(UNDO_DIR_NAME)?;
        for entry in entries {
            if !entry.name.ends_with(".snap") {
                continue;
            }
            let rel = format!("{UNDO_DIR_NAME}/{}", entry.name);
            if let Ok(bytes) = self.sandbox.read(&rel)
                && let Ok(snap) = serde_json::from_slice::<SnapFile>(&bytes)
            {
                snaps.push(snap);
            }
        }
        Ok(snaps)
    }

    /// The next sequence number = max existing + 1 (or 1 when empty).
    fn next_seq(&self) -> Result<u32> {
        let max = self
            .read_all()?
            .into_iter()
            .map(|s| s.seq)
            .max()
            .unwrap_or(0);
        Ok(max + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aether-undo-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn snapshot_and_restore_round_trip() {
        let root = temp_root("roundtrip");
        let journal = UndoJournal::new(&root);
        let seq = journal.snapshot("a/b.txt", "v1", false).unwrap();
        assert_eq!(seq, 1);
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::write(root.join("a/b.txt"), "v2").unwrap();
        let restored = journal.restore("a/b.txt").unwrap();
        assert_eq!(restored.seq, 1);
        assert_eq!(restored.rel, "a/b.txt");
        assert_eq!(std::fs::read_to_string(root.join("a/b.txt")).unwrap(), "v1");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn restore_pops_most_recent_snapshot() {
        let root = temp_root("pop");
        let journal = UndoJournal::new(&root);
        std::fs::write(root.join("f.txt"), "v0").unwrap();
        journal.snapshot("f.txt", "v0", false).unwrap(); // seq 1
        std::fs::write(root.join("f.txt"), "v1").unwrap();
        journal.snapshot("f.txt", "v1", false).unwrap(); // seq 2
        std::fs::write(root.join("f.txt"), "v2").unwrap();

        let first = journal.restore("f.txt").unwrap();
        assert_eq!(first.seq, 2);
        assert_eq!(std::fs::read_to_string(root.join("f.txt")).unwrap(), "v1");

        let second = journal.restore("f.txt").unwrap();
        assert_eq!(second.seq, 1);
        assert_eq!(std::fs::read_to_string(root.join("f.txt")).unwrap(), "v0");

        assert!(journal.restore("f.txt").is_err()); // exhausted
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn list_reports_metadata() {
        let root = temp_root("list");
        let journal = UndoJournal::new(&root);
        journal.snapshot("x.txt", "hello", false).unwrap();
        journal.snapshot("y.txt", "world", false).unwrap();
        let snaps = journal.list().unwrap();
        assert_eq!(snaps.len(), 2);
        assert_eq!(snaps[0].rel, "x.txt");
        assert_eq!(snaps[0].bytes, 5);
        assert_eq!(snaps[1].rel, "y.txt");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn restore_was_new_deletes_the_file() {
        let root = temp_root("wasnew");
        let journal = UndoJournal::new(&root);
        journal.snapshot("new.txt", "", true).unwrap();
        std::fs::write(root.join("new.txt"), "agent wrote this").unwrap();
        assert!(root.join("new.txt").exists());
        let restored = journal.restore("new.txt").unwrap();
        assert_eq!(restored.seq, 1);
        assert!(
            !root.join("new.txt").exists(),
            "was_new undo must delete the file"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn restore_rejects_out_of_sandbox_paths() {
        let root = temp_root("escape");
        let journal = UndoJournal::new(&root);
        journal.snapshot("ok.txt", "x", false).unwrap();
        for bad in ["../etc/passwd", "/etc/passwd", "a/../../b"] {
            assert!(
                matches!(journal.restore(bad), Err(Error::PathTraversal(_))),
                "expected traversal rejection for {bad:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn snapshot_rejects_out_of_sandbox_paths() {
        let root = temp_root("snapesc");
        let journal = UndoJournal::new(&root);
        assert!(journal.snapshot("../evil", "x", false).is_err());
        assert!(journal.snapshot("/etc/passwd", "x", false).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn restore_unknown_file_is_error() {
        let root = temp_root("unknown");
        let journal = UndoJournal::new(&root);
        assert!(journal.restore("nope.txt").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
