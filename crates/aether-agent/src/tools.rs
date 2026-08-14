//! Tool registry for the build model: filesystem + command execution,
//! sandboxed to a working directory.
//!
//! Safety layers:
//! - paths resolve through [`safe_join_rel`] (rejects traversal/absolute/NUL)
//! - `write_file` shows a diff preview and asks for confirmation unless the
//!   confirm gate is disabled or `--yes` was given; on a non-TTY stdin with
//!   no `--yes` it refuses to write
//! - every replaced file is snapshotted into `<root>/.aether-undo/` so the
//!   `undo` tool (and `aether undo` CLI) can restore it

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use aether_core::error::{Error, Result};
use aether_core::fs::safe_join_rel;
use aether_core::sandbox::Sandbox;
use aether_provider::ToolDef;
use serde_json::Value;

use crate::diff::{DiffLine, DiffLineKind, diff_lines, render_diff};
use crate::undo::{UNDO_DIR_NAME, UndoJournal};

/// Cap on bytes read from any single file (protects the model context).
pub const MAX_READ_BYTES: u64 = 256 * 1024;
/// Cap on command output captured (stdout + stderr combined).
pub const MAX_OUTPUT_BYTES: usize = 128 * 1024;
/// Default command timeout.
pub const CMD_TIMEOUT: Duration = Duration::from_secs(30);
/// Maximum diff lines shown in a write preview.
pub const MAX_DIFF_LINES: usize = 40;

/// How `write_file` handles the confirmation gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmPolicy {
    /// Gate disabled — writes proceed without asking.
    Disabled,
    /// Auto-approve every write (non-interactive `--yes`).
    AutoApprove,
    /// Show the diff and require y/N; refuse when stdin is not a TTY.
    Prompt,
}

/// A tool execution result rendered as text for the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub ok: bool,
    pub text: String,
    /// True when the call actually modified the filesystem (write/undo).
    pub modified: bool,
    /// Rendered +/- diff preview when the call changed file contents
    /// (populated by `write_file` only; `None` for every other tool).
    pub diff: Option<String>,
}

/// Filesystem + command tools bound to a working directory.
///
/// Filesystem operations go through a capability [`Sandbox`] handle: on
/// Linux, cap-std resolves every open with `openat2(RESOLVE_BENEATH)`, so a
/// path can never escape the working directory — symlink tricks and TOCTOU
/// races are rejected by the kernel at the open itself. `resolve` remains as
/// a *lexical* pre-check purely to emit a clear [`Error::PathTraversal`]
/// message early; it is not the security boundary.
#[derive(Debug, Clone)]
pub struct Tools {
    root: PathBuf,
    sandbox: Sandbox,
    confirm: ConfirmPolicy,
    undo: UndoJournal,
}

/// Parse tool arguments as a JSON object.
fn args_obj(args: &Value) -> Result<&serde_json::Map<String, Value>> {
    args.as_object()
        .ok_or_else(|| Error::InvalidInput("tool arguments must be a JSON object".to_string()))
}

/// Read a string field from tool arguments.
fn arg_str<'a>(obj: &'a serde_json::Map<String, Value>, name: &str) -> Result<&'a str> {
    obj.get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::InvalidInput(format!("missing string arg \"{name}\"")))
}

impl Tools {
    pub fn new(root: PathBuf) -> Self {
        let sandbox = Sandbox::open(&root)
            .unwrap_or_else(|e| panic!("cannot open sandbox for {}: {e}", root.display()));
        Self {
            confirm: ConfirmPolicy::Disabled,
            undo: UndoJournal::new(&root),
            sandbox,
            root,
        }
    }

    /// Enable or disable the confirmation gate for writes.
    ///
    /// The environment variable `AETHER_AGENT_CONFIRM=0` forces the gate off
    /// (used by non-interactive tests/CI). `true` means prompt interactively.
    pub fn with_confirm(mut self, enabled: bool) -> Self {
        self.confirm = if enabled {
            if std::env::var("AETHER_AGENT_CONFIRM").as_deref() == Ok("0") {
                ConfirmPolicy::Disabled
            } else {
                ConfirmPolicy::Prompt
            }
        } else {
            ConfirmPolicy::Disabled
        };
        self
    }

    /// Auto-approve all writes (non-interactive `--yes`).
    pub fn with_yes(mut self) -> Self {
        self.confirm = ConfirmPolicy::AutoApprove;
        self
    }

    /// The OpenAI `tools[]` descriptors for the registry.
    pub fn defs() -> Vec<ToolDef> {
        vec![
            ToolDef {
                kind: "function".to_string(),
                function: aether_provider::ToolFunction {
                    name: "read_file".to_string(),
                    description: "Read a text file relative to the working directory. Returns up to 256 KB.".to_string(),
                    parameters: Some(serde_json::json!({
                        "type": "object",
                        "properties": {"path": {"type": "string", "description": "relative path, e.g. src/main.rs"}},
                        "required": ["path"]
                    })),
                },
            },
            ToolDef {
                kind: "function".to_string(),
                function: aether_provider::ToolFunction {
                    name: "write_file".to_string(),
                    description: "Write a text file relative to the working directory, overwriting if present. Creates parent directories. The previous content is snapshotted and may require confirmation.".to_string(),
                    parameters: Some(serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": {"type": "string", "description": "relative path"},
                            "content": {"type": "string", "description": "full file content"}
                        },
                        "required": ["path", "content"]
                    })),
                },
            },
            ToolDef {
                kind: "function".to_string(),
                function: aether_provider::ToolFunction {
                    name: "list_dir".to_string(),
                    description: "List directory entries relative to the working directory. Use \"\" for the root.".to_string(),
                    parameters: Some(serde_json::json!({
                        "type": "object",
                        "properties": {"path": {"type": "string", "description": "relative directory path, or empty for root"}},
                        "required": ["path"]
                    })),
                },
            },
            ToolDef {
                kind: "function".to_string(),
                function: aether_provider::ToolFunction {
                    name: "run_command".to_string(),
                    description: "Run a shell command in the working directory. Returns stdout+stderr (capped at 128 KB) with the exit code. Timeout 30s.".to_string(),
                    parameters: Some(serde_json::json!({
                        "type": "object",
                        "properties": {"cmd": {"type": "string", "description": "shell command line"}},
                        "required": ["cmd"]
                    })),
                },
            },
            ToolDef {
                kind: "function".to_string(),
                function: aether_provider::ToolFunction {
                    name: "search".to_string(),
                    description: "Case-sensitive substring search over files under a directory (max 200 matches, 128 KB output).".to_string(),
                    parameters: Some(serde_json::json!({
                        "type": "object",
                        "properties": {
                            "needle": {"type": "string", "description": "substring to find"},
                            "path": {"type": "string", "description": "relative directory to search, or empty for root"}
                        },
                        "required": ["needle", "path"]
                    })),
                },
            },
            ToolDef {
                kind: "function".to_string(),
                function: aether_provider::ToolFunction {
                    name: "undo".to_string(),
                    description: "Restore a file to its most recent pre-write snapshot. Pass {\"file\":\"rel/path\"} to restore that file, or {} to list available snapshots.".to_string(),
                    parameters: Some(serde_json::json!({
                        "type": "object",
                        "properties": {
                            "file": {"type": "string", "description": "relative path to restore; omit to list snapshots"}
                        },
                        "required": []
                    })),
                },
            },
        ]
    }

    /// Execute one tool call by name with JSON arguments.
    pub fn call(&self, name: &str, args: &Value) -> Result<ToolResult> {
        let obj = args_obj(args)?;
        match name {
            "read_file" => self.read_file(arg_str(obj, "path")?),
            "write_file" => {
                let path = arg_str(obj, "path")?;
                let content = arg_str(obj, "content")?;
                self.write_file(path, content)
            }
            "list_dir" => self.list_dir(arg_str(obj, "path")?),
            "run_command" => self.run_command(arg_str(obj, "cmd")?),
            "search" => {
                let needle = arg_str(obj, "needle")?;
                let path = arg_str(obj, "path")?;
                self.search(needle, path)
            }
            "undo" => self.undo_tool(obj),
            other => Err(Error::InvalidInput(format!("unknown tool \"{other}\""))),
        }
    }

    /// Resolve a relative path inside the sandbox.
    fn resolve(&self, rel: &str) -> Result<PathBuf> {
        safe_join_rel(&self.root, rel)
    }

    fn read_file(&self, rel: &str) -> Result<ToolResult> {
        let _ = self.resolve(rel)?;
        let meta = self.sandbox.metadata(rel)?;
        if meta.len() > MAX_READ_BYTES {
            return Err(Error::InvalidInput(format!(
                "file too large ({} bytes, cap {MAX_READ_BYTES})",
                meta.len()
            )));
        }
        let bytes = self.sandbox.read(rel)?;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        Ok(ToolResult {
            ok: true,
            text: format!("== {rel} ({} bytes) ==\n{text}", bytes.len()),
            modified: false,
            diff: None,
        })
    }

    fn write_file(&self, rel: &str, content: &str) -> Result<ToolResult> {
        let _ = self.resolve(rel)?;
        self.reject_undo_path(rel)?;

        // Existing content (if any) for diff + snapshot purposes.
        let old = if self.sandbox.exists(rel) {
            self.sandbox.read_to_string(rel).ok()
        } else {
            None
        };

        let diff = diff_lines(old.as_deref().unwrap_or(""), content);
        let changes = diff.iter().any(|l| l.kind != DiffLineKind::Same);
        if changes {
            match self.confirm {
                ConfirmPolicy::Disabled | ConfirmPolicy::AutoApprove => {}
                ConfirmPolicy::Prompt => self.confirm_change(rel, &diff)?,
            }
        }

        // Snapshot the file being replaced BEFORE overwriting it.
        if let Some(old_content) = &old {
            self.undo.snapshot(rel, old_content)?;
        }

        self.sandbox.atomic_write(rel, content.as_bytes())?;
        Ok(ToolResult {
            ok: true,
            text: format!("wrote {} bytes to {rel}", content.len()),
            modified: true,
            diff: if changes {
                Some(render_diff(&diff, MAX_DIFF_LINES))
            } else {
                None
            },
        })
    }

    /// Interactive confirm gate: show the diff on stderr, require y/N.
    /// Refuses when stdin is not a TTY (caller should have checked, but we
    /// double-check here so a non-interactive process can never hang).
    fn confirm_change(&self, rel: &str, diff: &[DiffLine]) -> Result<()> {
        use std::io::Write;
        if !std::io::stdin().is_terminal() {
            return Err(Error::InvalidInput(format!(
                "refusing to write {rel}: stdin is not a TTY and no --yes flag was given \
                 (re-run with --yes or from an interactive terminal)"
            )));
        }
        eprintln!("--- {rel}");
        eprintln!("+++ {rel} (proposed)");
        eprint!("{}", render_diff(diff, MAX_DIFF_LINES));
        eprint!("Write {rel}? [y/N] ");
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        match line.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => Ok(()),
            _ => Err(Error::InvalidInput(format!(
                "write to {rel} refused by user"
            ))),
        }
    }

    /// The `undo` tool: restore a file to its most recent snapshot, or list
    /// snapshots when no `file` argument is given.
    fn undo_tool(&self, obj: &serde_json::Map<String, Value>) -> Result<ToolResult> {
        match obj.get("file").and_then(Value::as_str) {
            Some(rel) => {
                let restored = self.undo.restore(rel)?;
                Ok(ToolResult {
                    ok: true,
                    text: format!(
                        "restored {rel} from snapshot {} ({} bytes)",
                        restored.seq, restored.bytes
                    ),
                    modified: true,
                    diff: None,
                })
            }
            None => {
                let snaps = self.undo.list()?;
                if snaps.is_empty() {
                    return Ok(ToolResult {
                        ok: true,
                        text: "no snapshots".to_string(),
                        modified: false,
                        diff: None,
                    });
                }
                let text = snaps
                    .iter()
                    .map(|s| format!("{}  {}  ({} bytes)", s.seq, s.rel, s.bytes))
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok(ToolResult {
                    ok: true,
                    text,
                    modified: false,
                    diff: None,
                })
            }
        }
    }

    /// The undo journal is agent-internal state; the model must not write
    /// into it (that would let it forge snapshots).
    fn reject_undo_path(&self, rel: &str) -> Result<()> {
        if rel == UNDO_DIR_NAME
            || rel
                .strip_prefix(UNDO_DIR_NAME)
                .is_some_and(|rest| rest.starts_with('/'))
        {
            return Err(Error::InvalidInput(format!(
                "refusing to write into {UNDO_DIR_NAME}/ (agent-internal)"
            )));
        }
        Ok(())
    }

    fn list_dir(&self, rel: &str) -> Result<ToolResult> {
        if !rel.is_empty() {
            self.resolve(rel)?;
        }
        let mut entries: Vec<String> = Vec::new();
        for entry in self.sandbox.read_dir(rel)? {
            if entry.name == UNDO_DIR_NAME {
                continue;
            }
            let kind = if entry.is_dir { "dir" } else { "file" };
            entries.push(format!("{kind}\t{}", entry.name));
        }
        entries.sort();
        let text = if entries.is_empty() {
            format!("(empty dir: {rel})")
        } else {
            entries.join("\n")
        };
        Ok(ToolResult {
            ok: true,
            text,
            modified: false,
            diff: None,
        })
    }

    fn run_command(&self, cmd: &str) -> Result<ToolResult> {
        let output = run_with_timeout(cmd, &self.root, CMD_TIMEOUT)?;
        Ok(ToolResult {
            ok: true,
            text: render_command_output(cmd, &output),
            modified: false,
            diff: None,
        })
    }

    fn search(&self, needle: &str, rel: &str) -> Result<ToolResult> {
        if !rel.is_empty() {
            self.resolve(rel)?;
        }
        let mut matches: Vec<String> = Vec::new();
        self.sandbox.walk_files(rel, &mut |file| {
            if matches.len() >= 200 {
                return Ok(());
            }
            if file.starts_with(UNDO_DIR_NAME) || file.contains(&format!("/{UNDO_DIR_NAME}/")) {
                return Ok(());
            }
            let text = self.sandbox.read_to_string(file).unwrap_or_default();
            for (idx, line) in text.lines().enumerate() {
                if line.contains(needle) {
                    matches.push(format!("{file}:{}:{line}", idx + 1));
                    if matches.len() >= 200 {
                        break;
                    }
                }
            }
            Ok(())
        })?;
        let text = if matches.is_empty() {
            format!("no matches for {needle:?} under {rel}")
        } else {
            matches.join("\n")
        };
        Ok(ToolResult {
            ok: true,
            text,
            modified: false,
            diff: None,
        })
    }
}

/// Build a shell invocation for the current platform.
/// `/bin/sh -c` on Unix, `cmd /C` on Windows.
fn platform_shell(cmd: &str) -> std::process::Command {
    let mut c = std::process::Command::new(if cfg!(windows) { "cmd" } else { "/bin/sh" });
    if cfg!(windows) {
        c.arg("/C").arg(cmd);
    } else {
        c.arg("-c").arg(cmd);
    }
    c
}

/// Run a shell command with a timeout, capturing capped stdout+stderr.
fn run_with_timeout(cmd: &str, cwd: &Path, timeout: Duration) -> Result<CommandOutput> {
    let mut child = platform_shell(cmd)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let (mut stdout, mut stderr) = match (child.stdout.take(), child.stderr.take()) {
        (Some(o), Some(e)) => (o, e),
        _ => {
            return Err(Error::InvalidInput(
                "failed to capture command pipes".to_string(),
            ));
        }
    };

    let reader_stdout = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match std::io::Read::read(&mut stdout, &mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    if buf.len() >= MAX_OUTPUT_BYTES {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        buf
    });
    let reader_stderr = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match std::io::Read::read(&mut stderr, &mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    if buf.len() >= MAX_OUTPUT_BYTES {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        buf
    });

    let status = match wait_with_timeout(&mut child, timeout) {
        Some(status) => status,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader_stdout.join();
            let _ = reader_stderr.join();
            return Err(Error::InvalidInput(format!(
                "command timed out after {}s: {cmd}",
                timeout.as_secs()
            )));
        }
    };

    let out_bytes = reader_stdout.join().unwrap_or_default();
    let err_bytes = reader_stderr.join().unwrap_or_default();
    let stdout = String::from_utf8_lossy(&out_bytes).into_owned();
    let stderr = String::from_utf8_lossy(&err_bytes).into_owned();

    Ok(CommandOutput {
        code: status.code(),
        stdout,
        stderr,
    })
}

/// Wait for `child` with a deadline; returns `None` on timeout.
fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Option<std::process::ExitStatus> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().ok().flatten() {
            return Some(status);
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[derive(Debug)]
struct CommandOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Render a command result as a compact text block for the model.
fn render_command_output(cmd: &str, out: &CommandOutput) -> String {
    let mut text = String::new();
    text.push_str(&format!("$ {cmd}\n"));
    if !out.stdout.is_empty() {
        text.push_str(&out.stdout);
        if !text.ends_with('\n') {
            text.push('\n');
        }
    }
    if !out.stderr.is_empty() {
        text.push_str("[stderr]\n");
        text.push_str(&out.stderr);
        if !text.ends_with('\n') {
            text.push('\n');
        }
    }
    match out.code {
        Some(0) => text.push_str("[exit 0]\n"),
        Some(code) => text.push_str(&format!("[exit {code}]\n")),
        None => text.push_str("[killed]\n"),
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aether-tools-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn read_write_round_trip() {
        let root = temp_root("rw");
        let tools = Tools::new(root.clone());
        let res = tools
            .call(
                "write_file",
                &serde_json::json!({"path": "a/b.txt", "content": "hello"}),
            )
            .unwrap();
        assert!(res.ok);
        assert!(res.modified);
        assert!(res.text.contains("wrote 5 bytes"));
        let res = tools
            .call("read_file", &serde_json::json!({"path": "a/b.txt"}))
            .unwrap();
        assert!(res.text.contains("hello"));
        assert!(!res.modified);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn traversal_blocked() {
        let root = temp_root("traverse");
        let tools = Tools::new(root.clone());
        let res = tools.call("read_file", &serde_json::json!({"path": "../etc/passwd"}));
        assert!(res.is_err());
        let res = tools.call("read_file", &serde_json::json!({"path": "/etc/passwd"}));
        assert!(res.is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn list_dir_sorted() {
        let root = temp_root("list");
        std::fs::write(root.join("b.txt"), "x").unwrap();
        std::fs::write(root.join("a.txt"), "y").unwrap();
        let tools = Tools::new(root.clone());
        let res = tools
            .call("list_dir", &serde_json::json!({"path": ""}))
            .unwrap();
        assert!(res.text.contains("file\ta.txt"));
        assert!(res.text.contains("file\tb.txt"));
        assert!(res.text.find("a.txt").unwrap() < res.text.find("b.txt").unwrap());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_command_ok_and_fail() {
        let root = temp_root("cmd");
        let tools = Tools::new(root.clone());
        let res = tools
            .call("run_command", &serde_json::json!({"cmd": "echo hi"}))
            .unwrap();
        assert!(res.text.contains("hi"));
        assert!(res.text.contains("[exit 0]"));
        let res = tools
            .call("run_command", &serde_json::json!({"cmd": "exit 1"}))
            .unwrap();
        assert!(res.text.contains("[exit 1]"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn search_finds_substring() {
        let root = temp_root("search");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() { aether_run(); }\n").unwrap();
        std::fs::write(root.join("README.md"), "no matches here\n").unwrap();
        let tools = Tools::new(root.clone());
        let res = tools
            .call(
                "search",
                &serde_json::json!({"needle": "aether_run", "path": ""}),
            )
            .unwrap();
        assert!(res.text.contains("main.rs:1"));
        let res = tools
            .call("search", &serde_json::json!({"needle": "zzz", "path": ""}))
            .unwrap();
        assert!(res.text.contains("no matches"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unknown_tool_rejected() {
        let root = temp_root("unknown");
        let tools = Tools::new(root.clone());
        let res = tools.call("rm_rf", &serde_json::json!({}));
        assert!(res.is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn write_then_undo_restores_previous_content() {
        let root = temp_root("undo");
        let tools = Tools::new(root.clone());
        tools
            .call(
                "write_file",
                &serde_json::json!({"path": "f.txt", "content": "v1"}),
            )
            .unwrap();
        tools
            .call(
                "write_file",
                &serde_json::json!({"path": "f.txt", "content": "v2"}),
            )
            .unwrap();
        let res = tools
            .call("undo", &serde_json::json!({"file": "f.txt"}))
            .unwrap();
        assert!(res.ok);
        assert!(res.modified);
        assert!(res.text.contains("restored f.txt"));
        let res = tools
            .call("read_file", &serde_json::json!({"path": "f.txt"}))
            .unwrap();
        assert!(res.text.contains("v1"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn write_file_carries_rendered_diff() {
        let root = temp_root("diffcarry");
        let tools = Tools::new(root.clone());
        std::fs::write(root.join("f.txt"), "old line\n").unwrap();
        let res = tools
            .call(
                "write_file",
                &serde_json::json!({"path": "f.txt", "content": "old line\nnew line\n"}),
            )
            .unwrap();
        assert!(res.ok);
        assert!(res.modified);
        let d = res.diff.expect("write_file must carry a diff");
        assert!(d.contains("+ new line"), "added line missing: {d}");
        assert!(d.contains("  old line"), "context line missing: {d}");
        assert!(!d.contains("- old line"), "context shown as removal: {d}");
        let res = tools
            .call("read_file", &serde_json::json!({"path": "f.txt"}))
            .unwrap();
        assert_eq!(res.diff, None, "read_file must not carry a diff");
        let res = tools
            .call("run_command", &serde_json::json!({"cmd": "true"}))
            .unwrap();
        assert_eq!(res.diff, None, "run_command must not carry a diff");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn undo_lists_snapshots() {
        let root = temp_root("undolist");
        let tools = Tools::new(root.clone());
        std::fs::write(root.join("f.txt"), "v0").unwrap();
        tools
            .call(
                "write_file",
                &serde_json::json!({"path": "f.txt", "content": "v1"}),
            )
            .unwrap();
        let res = tools.call("undo", &serde_json::json!({})).unwrap();
        assert!(res.ok);
        assert!(!res.modified);
        assert!(res.text.contains("f.txt"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn undo_rejects_out_of_sandbox_paths() {
        let root = temp_root("undoescape");
        let tools = Tools::new(root.clone());
        std::fs::write(root.join("f.txt"), "v0").unwrap();
        tools
            .call(
                "write_file",
                &serde_json::json!({"path": "f.txt", "content": "v1"}),
            )
            .unwrap();
        for bad in ["../etc/passwd", "/etc/passwd", "a/../../b"] {
            let res = tools.call("undo", &serde_json::json!({"file": bad}));
            assert!(res.is_err(), "expected rejection for {bad:?}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn undo_dir_hidden_from_list_and_search() {
        let root = temp_root("hidden");
        let tools = Tools::new(root.clone());
        std::fs::write(root.join("f.txt"), "v0").unwrap();
        tools
            .call(
                "write_file",
                &serde_json::json!({"path": "f.txt", "content": "v1"}),
            )
            .unwrap();
        // Snapshots were persisted.
        assert!(root.join(".aether-undo").is_dir());
        // ...but the model cannot see the journal.
        let res = tools
            .call("list_dir", &serde_json::json!({"path": ""}))
            .unwrap();
        assert!(!res.text.contains(".aether-undo"));
        let res = tools
            .call("search", &serde_json::json!({"needle": "v0", "path": ""}))
            .unwrap();
        assert!(!res.text.contains(".aether-undo"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn cannot_write_into_undo_dir() {
        let root = temp_root("guard");
        let tools = Tools::new(root.clone());
        let res = tools.call(
            "write_file",
            &serde_json::json!({"path": ".aether-undo/0001.snap", "content": "x"}),
        );
        assert!(res.is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Deterministic core of the confirm gate: Prompt + non-TTY => refuse.
    #[test]
    fn confirm_gate_refuses_when_not_tty() {
        // In a normal test run stdin is not a TTY, so this exercises the
        // real refusal path. If a harness attaches a TTY we skip rather
        // than block on read_line.
        if std::io::stdin().is_terminal() {
            eprintln!("skipping: stdin is a TTY in this environment");
            return;
        }
        let root = temp_root("confirm");
        std::fs::write(root.join("f.txt"), "old content").unwrap();
        let tools = Tools::new(root.clone()).with_confirm(true);
        let res = tools.call(
            "write_file",
            &serde_json::json!({"path": "f.txt", "content": "new content"}),
        );
        assert!(res.is_err());
        let err = res.unwrap_err().to_string();
        assert!(
            err.contains("refusing to write f.txt"),
            "unexpected error: {err}"
        );
        // File untouched.
        assert_eq!(
            std::fs::read_to_string(root.join("f.txt")).unwrap(),
            "old content"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// --yes (AutoApprove) writes without a TTY.
    #[test]
    fn yes_flag_writes_without_tty() {
        let root = temp_root("yes");
        std::fs::write(root.join("f.txt"), "old").unwrap();
        let tools = Tools::new(root.clone()).with_yes();
        let res = tools.call(
            "write_file",
            &serde_json::json!({"path": "f.txt", "content": "new"}),
        );
        assert!(res.is_ok());
        assert_eq!(std::fs::read_to_string(root.join("f.txt")).unwrap(), "new");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn write_snapshots_before_overwrite() {
        let root = temp_root("snap");
        let tools = Tools::new(root.clone());
        tools
            .call(
                "write_file",
                &serde_json::json!({"path": "f.txt", "content": "v1"}),
            )
            .unwrap();
        tools
            .call(
                "write_file",
                &serde_json::json!({"path": "f.txt", "content": "v2"}),
            )
            .unwrap();
        tools
            .call(
                "write_file",
                &serde_json::json!({"path": "f.txt", "content": "v3"}),
            )
            .unwrap();
        let res = tools
            .call("undo", &serde_json::json!({"file": "f.txt"}))
            .unwrap();
        assert!(res.text.contains("restored f.txt"));
        let res = tools
            .call("read_file", &serde_json::json!({"path": "f.txt"}))
            .unwrap();
        assert!(res.text.contains("v2"), "expected v2, got: {}", res.text);
        let _ = std::fs::remove_dir_all(&root);
    }

    // ------------------------------------------------------------------
    // Adversarial sandbox-escape tests.
    //
    // Each test *actually attempts* to escape the sandbox root and asserts
    // the attempt fails. The security boundary is the cap-std capability
    // handle (openat2 RESOLVE_BENEATH on Linux): the kernel resolves the
    // full path relative to the handle fd, so escaping symlinks and TOCTOU
    // swaps are rejected at the open itself, with no validate-then-open
    // window to race.
    // ------------------------------------------------------------------

    // 1. Absolute path must never escape the root.
    #[test]
    fn escape_absolute_path_blocked() {
        let root = temp_root("esc-abs");
        let tools = Tools::new(root.clone());
        for path in ["/etc/passwd", "/etc", "/"] {
            let res = tools.call("read_file", &serde_json::json!({"path": path}));
            assert!(res.is_err(), "absolute path {path:?} must be rejected");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    // 2. `..` traversal must never escape the root.
    #[test]
    fn escape_dotdot_blocked() {
        let root = temp_root("esc-dotdot");
        let tools = Tools::new(root.clone());
        for path in [
            "..",
            "../",
            "../etc/passwd",
            "a/../../b",
            "a/../../../etc/passwd",
        ] {
            let res = tools.call("read_file", &serde_json::json!({"path": path}));
            assert!(res.is_err(), "dotdot path {path:?} must be rejected");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    // 3. External symlink inside the root pointing at `/etc` must not be
    /// followable out of the sandbox.
    #[cfg(unix)]
    #[test]
    fn escape_external_symlink_blocked() {
        use std::os::unix::fs::symlink;
        let root = temp_root("esc-symlink");
        symlink("/etc", root.join("evil")).unwrap();
        let tools = Tools::new(root.clone());
        for path in ["evil/passwd", "evil/hostname", "evil/"] {
            let res = tools.call("read_file", &serde_json::json!({"path": path}));
            assert!(res.is_err(), "symlink escape {path:?} must be rejected");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    // 4. TOCTOU: a directory swapped for a symlink between validation and
    /// open. The kernel answers for the capability fd, so the swap cannot
    /// widen access.
    #[cfg(unix)]
    #[test]
    fn escape_toctou_dir_swap_blocked() {
        use std::os::unix::fs::symlink;
        let root = temp_root("esc-toctou");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/file.txt"), "inside").unwrap();
        let tools = Tools::new(root.clone());
        assert!(
            tools
                .call("read_file", &serde_json::json!({"path": "sub/file.txt"}))
                .is_ok()
        );
        // Attacker swaps `sub` for a symlink to /etc AFTER the handle opened.
        std::fs::remove_dir_all(root.join("sub")).unwrap();
        symlink("/etc", root.join("sub")).unwrap();
        let res = tools.call("read_file", &serde_json::json!({"path": "sub/passwd"}));
        assert!(res.is_err(), "TOCTOU swap must not escape");
        let res = tools.call(
            "write_file",
            &serde_json::json!({"path": "sub/new.txt", "content": "x"}),
        );
        assert!(res.is_err(), "TOCTOU write must not escape");
        assert!(!std::path::Path::new("/etc/new.txt").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    // 5. Env-var CWD manipulation (e.g. `PWD=/` or chdir) must not re-root
    /// the sandbox: the capability handle is independent of process CWD.
    #[test]
    fn escape_cwd_env_does_not_reroot() {
        let root = temp_root("esc-cwd");
        std::fs::write(root.join("in.txt"), "hello").unwrap();
        let tools = Tools::new(root.clone());
        let orig_cwd = std::env::current_dir().unwrap();
        // Attacker changes the process working directory far away.
        std::env::set_current_dir("/").unwrap();
        let res = tools.call("read_file", &serde_json::json!({"path": "in.txt"}));
        std::env::set_current_dir(&orig_cwd).unwrap();
        assert!(res.is_ok(), "sandbox must stay rooted even after chdir");
        // And still no escape.
        let res = tools.call("read_file", &serde_json::json!({"path": "../etc/passwd"}));
        assert!(res.is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    // 6. Unicode homoglyph of `..` (e.g. U+2024) must not be treated as
    /// traversal, and must not escape.
    #[test]
    fn escape_unicode_homoglyph_blocked() {
        let root = temp_root("esc-unicode");
        std::fs::write(root.join("safe.txt"), "x").unwrap();
        let tools = Tools::new(root.clone());
        for path in [
            "\u{2024}\u{2024}/etc/passwd",
            "a\u{2024}b",
            "\u{FF0E}\u{FF0E}/passwd",
        ] {
            let res = tools.call("read_file", &serde_json::json!({"path": path}));
            // Either a clean miss (no such file) or rejection — never content
            // from outside the sandbox.
            if let Ok(r) = res {
                assert!(!r.text.contains("root:"), "homoglyph leaked /etc/passwd");
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    // 7. NUL byte in a path must be rejected outright.
    #[test]
    fn escape_nul_byte_blocked() {
        let root = temp_root("esc-nul");
        let tools = Tools::new(root.clone());
        let res = tools.call("read_file", &serde_json::json!({"path": "a\0b"}));
        assert!(res.is_err(), "NUL byte path must be rejected");
        let _ = std::fs::remove_dir_all(&root);
    }

    // 8. Backslash as a separator (Windows-style) must not escape on any
    /// platform.
    #[test]
    fn escape_backslash_blocked() {
        let root = temp_root("esc-bs");
        let tools = Tools::new(root.clone());
        for path in [r"..\..\etc\passwd", r"a\..\b", r"C:\Windows\system32"] {
            let res = tools.call("read_file", &serde_json::json!({"path": path}));
            assert!(res.is_err(), "backslash path {path:?} must be rejected");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    // 9. `a/./../b` — embedded `.`/`..` segments after a valid prefix must
    /// not collapse back out of the root.
    #[test]
    fn escape_embedded_dotdot_blocked() {
        let root = temp_root("esc-embedded");
        std::fs::create_dir_all(root.join("a")).unwrap();
        let tools = Tools::new(root.clone());
        for path in [
            "a/./../etc/passwd",
            "a/../a/../../etc/passwd",
            "./../etc/passwd",
        ] {
            let res = tools.call("read_file", &serde_json::json!({"path": path}));
            assert!(res.is_err(), "embedded dotdot {path:?} must be rejected");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    // 10. Symlink loop inside the root must error (ELOOP), not hang or
    /// escape.
    #[cfg(unix)]
    #[test]
    fn escape_symlink_loop_blocked() {
        use std::os::unix::fs::symlink;
        let root = temp_root("esc-loop");
        symlink("b", root.join("a")).unwrap();
        symlink("a", root.join("b")).unwrap();
        let tools = Tools::new(root.clone());
        let res = tools.call("read_file", &serde_json::json!({"path": "a"}));
        assert!(res.is_err(), "symlink loop must error, not escape");
        let _ = std::fs::remove_dir_all(&root);
    }
}
