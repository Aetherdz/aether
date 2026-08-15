//! Maturity benchmark — real-repo tool exercise (deterministic, no LLM).
//!
//! Proves the agent's file tools work against real-world codebases, not just
//! synthetic fixtures. Each repo gets a realistic agent-style workload:
//! read, list, search, write+undo round-trip, and a benign shell command.
//! The repos are cloned by `benchmark/maturity.sh` (not by this test) so the
//! suite stays hermetic in CI; when the repo dir is absent the test skips.

use std::path::{Path, PathBuf};

use aether_agent::tools::Tools;
use serde_json::json;

fn repo_root(name: &str) -> PathBuf {
    let env = std::env::var("AETHER_MATURITY_REPOS")
        .unwrap_or_else(|_| "/tmp/opencode/maturity-bench".into());
    PathBuf::from(env).join(name)
}

fn require_repo(name: &str) -> Option<PathBuf> {
    let p = repo_root(name);
    if p.is_dir() {
        // Clean journal + scratch state left by a previous failed run so the
        // suite is re-runnable (idempotent). Real repos are never modified
        // outside .aether-* paths.
        let _ = std::fs::remove_dir_all(p.join(".aether-undo"));
        let _ = std::fs::remove_dir_all(p.join(".aether-bench"));
        Some(p)
    } else {
        None
    }
}

#[test]
fn real_repos_read_search_list() {
    let mut checked = 0usize;
    for name in ["express", "requests", "ripgrep", "jq", "bootstrap"] {
        let Some(root) = require_repo(name) else {
            eprintln!("SKIP {name}: repo not present (run benchmark/maturity.sh)");
            continue;
        };
        let tools = Tools::new(root.clone());
        checked += 1;

        let res = tools
            .call("list_dir", &json!({"path": ""}))
            .expect("list_dir on real repo root");
        assert!(res.ok, "{name}: list_dir failed: {}", res.text);

        let readme = ["README.md", "Readme.md", "readme.md"]
            .iter()
            .find(|f| root.join(f).is_file())
            .copied()
            .unwrap_or("README.md");
        let res = tools
            .call("read_file", &json!({"path": readme}))
            .expect("read_file README");
        assert!(res.ok, "{name}: read_file failed: {}", res.text);
        assert!(res.text.len() > 40, "{name}: README too short");

        let res = tools
            .call("search", &json!({"needle": "TODO", "path": ""}))
            .expect("search TODO");
        assert!(res.ok, "{name}: search failed: {}", res.text);
    }
    assert!(checked >= 4, "expected >=4 repos present, got {checked}");
}

#[test]
fn real_repos_write_undo_round_trip() {
    let mut checked = 0usize;
    for name in ["express", "requests", "ripgrep", "jq", "bootstrap"] {
        let Some(root) = require_repo(name) else {
            eprintln!("SKIP {name}: repo not present (run benchmark/maturity.sh)");
            continue;
        };
        let tools = Tools::new(root.clone());
        checked += 1;

        let scratch = ".aether-bench/notes.txt";
        let res = tools
            .call(
                "write_file",
                &json!({"path": scratch, "content": "maturity probe v1"}),
            )
            .expect("write_file into repo");
        assert!(res.ok, "{name}: write_file failed: {}", res.text);

        let res = tools.call("undo", &json!({"file": scratch})).expect("undo");
        assert!(res.ok, "{name}: undo failed: {}", res.text);
        assert!(
            res.text.contains(".aether-bench"),
            "{name}: undo did not mention the file"
        );

        let after = root.join(scratch);
        assert!(!after.exists(), "{name}: file still present after undo");
    }
    assert!(checked >= 4, "expected >=4 repos present, got {checked}");
}

#[test]
fn real_repos_sandbox_blocks_escape() {
    let mut checked = 0usize;
    for name in ["express", "requests", "ripgrep", "jq", "bootstrap"] {
        let Some(root) = require_repo(name) else {
            eprintln!("SKIP {name}: repo not present (run benchmark/maturity.sh)");
            continue;
        };
        let tools = Tools::new(root.clone());
        checked += 1;

        for bad in ["../outside.txt", "/etc/passwd", "sub/../../escape.txt"] {
            let res = tools.call("write_file", &json!({"path": bad, "content": "x"}));
            assert!(res.is_err(), "{name}: sandbox must reject {bad}");
        }
        assert!(!Path::new("/tmp/aether-bench-escape").exists());
    }
    assert!(checked >= 4, "expected >=4 repos present, got {checked}");
}

#[test]
fn real_repos_benign_command_runs() {
    let mut checked = 0usize;
    for name in ["express", "requests", "ripgrep", "jq", "bootstrap"] {
        let Some(root) = require_repo(name) else {
            eprintln!("SKIP {name}: repo not present (run benchmark/maturity.sh)");
            continue;
        };
        let tools = Tools::new(root.clone());
        checked += 1;

        let res = tools
            .call("run_command", &json!({"cmd": "ls -A | wc -l"}))
            .expect("run_command ls");
        assert!(res.ok, "{name}: run_command failed: {}", res.text);
        let count_line = res
            .text
            .lines()
            .find(|l| !l.starts_with('$') && !l.starts_with("[exit") && !l.starts_with("[stderr]"))
            .unwrap_or("");
        let n: usize = count_line.trim().parse().unwrap_or(0);
        assert!(
            n > 0,
            "{name}: empty directory listing (got {count_line:?})"
        );
    }
    assert!(checked >= 4, "expected >=4 repos present, got {checked}");
}
