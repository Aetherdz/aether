//! Persistent undo journal for `write_file`.
//!
//! Every time `write_file` replaces an existing file, the previous content
//! is snapshotted into `<root>/.aether-undo/NNNN.snap` on disk. The journal
//! survives the agent process, so `aether undo` can restore files later.
//!
//! Security: paths are validated with `safe_join_rel` before any read or
//! write, so a snapshot can never escape the sandbox, and the journal is
//! never exposed to the model as an arbitrary file-access tool.

use std::path::{Path, PathBuf};

use aether_core::error::{Error, Result};
use aether_core::fs::{atomic_write, safe_join_rel};
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

/// On-disk snapshot file: `{ "seq": 1, "rel": "a/b.txt", "content": "..." }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SnapFile {
    seq: u32,
    rel: String,
    content: String,
}

/// Disk-backed snapshot journal rooted at `<sandbox_root>/.aether-undo`.
#[derive(Debug, Clone)]
pub struct UndoJournal {
    dir: PathBuf,
}

impl UndoJournal {
    pub fn new(root: &Path) -> Self {
        Self {
            dir: root.join(UNDO_DIR_NAME),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The sandbox root this journal lives under.
    fn root(&self) -> &Path {
        self.dir.parent().expect("undo dir always has a parent")
    }

    fn snap_path(&self, seq: u32) -> PathBuf {
        self.dir.join(format!("{seq:04}.snap"))
    }

    /// Save `content` as the newest snapshot for `rel`. Returns the seq.
    pub fn snapshot(&self, rel: &str, content: &str) -> Result<u32> {
        // Validate the rel path against the sandbox before recording it.
        safe_join_rel(self.root(), rel)?;
        let seq = self.next_seq()?;
        let snap = SnapFile {
            seq,
            rel: rel.to_string(),
            content: content.to_string(),
        };
        let json = serde_json::to_vec(&snap)?;
        atomic_write(&self.snap_path(seq), &json)?;
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
        let target = safe_join_rel(self.root(), rel)?;
        let mut snaps = self.read_all()?;
        snaps.sort_by_key(|s| s.seq);
        let snap = snaps
            .into_iter()
            .rev()
            .find(|s| s.rel == rel)
            .ok_or_else(|| Error::InvalidInput(format!("no snapshot for {rel:?}")))?;
        atomic_write(&target, snap.content.as_bytes())?;
        let restored = RestoredSnapshot {
            seq: snap.seq,
            rel: snap.rel.clone(),
            bytes: snap.content.len(),
        };
        let _ = std::fs::remove_file(self.snap_path(snap.seq));
        Ok(restored)
    }

    /// Read and parse every snapshot file currently on disk.
    fn read_all(&self) -> Result<Vec<SnapFile>> {
        let mut snaps = Vec::new();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(snaps),
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.ends_with(".snap") {
                continue;
            }
            if let Ok(bytes) = std::fs::read(entry.path())
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
        let seq = journal.snapshot("a/b.txt", "v1").unwrap();
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
        journal.snapshot("f.txt", "v0").unwrap(); // seq 1
        std::fs::write(root.join("f.txt"), "v1").unwrap();
        journal.snapshot("f.txt", "v1").unwrap(); // seq 2
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
        journal.snapshot("x.txt", "hello").unwrap();
        journal.snapshot("y.txt", "world").unwrap();
        let snaps = journal.list().unwrap();
        assert_eq!(snaps.len(), 2);
        assert_eq!(snaps[0].rel, "x.txt");
        assert_eq!(snaps[0].bytes, 5);
        assert_eq!(snaps[1].rel, "y.txt");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn restore_rejects_out_of_sandbox_paths() {
        let root = temp_root("escape");
        let journal = UndoJournal::new(&root);
        journal.snapshot("ok.txt", "x").unwrap();
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
        assert!(journal.snapshot("../evil", "x").is_err());
        assert!(journal.snapshot("/etc/passwd", "x").is_err());
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
