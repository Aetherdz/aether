//! Tool registry for the build model: filesystem + command execution,
//! sandboxed to a working directory.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use aether_core::error::{Error, Result};
use aether_core::fs::safe_join_rel;
use aether_provider::ToolDef;
use serde_json::Value;

/// Cap on bytes read from any single file (protects the model context).
pub const MAX_READ_BYTES: u64 = 256 * 1024;
/// Cap on command output captured (stdout + stderr combined).
pub const MAX_OUTPUT_BYTES: usize = 128 * 1024;
/// Default command timeout.
pub const CMD_TIMEOUT: Duration = Duration::from_secs(30);

/// A tool execution result rendered as text for the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub ok: bool,
    pub text: String,
}

/// Filesystem + command tools bound to a working directory.
#[derive(Debug, Clone)]
pub struct Tools {
    root: PathBuf,
}

/// Parse tool arguments as a JSON object.
fn args_obj(args: &Value) -> Result<&serde_json::Map<String, Value>> {
    args.as_object().ok_or_else(|| {
        Error::InvalidInput("tool arguments must be a JSON object".to_string())
    })
}

/// Read a string field from tool arguments.
fn arg_str<'a>(obj: &'a serde_json::Map<String, Value>, name: &str) -> Result<&'a str> {
    obj.get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::InvalidInput(format!("missing string arg \"{name}\"")))
}

impl Tools {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
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
                    description: "Write a text file relative to the working directory, overwriting if present. Creates parent directories.".to_string(),
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
        ]
    }

    /// Execute one tool call by name with JSON arguments.
    pub fn call(&self, name: &str, args: &Value) -> Result<ToolResult> {
        let obj = args_obj(args)?;
        let text = match name {
            "read_file" => self.read_file(arg_str(obj, "path")?)?,
            "write_file" => {
                let path = arg_str(obj, "path")?;
                let content = arg_str(obj, "content")?;
                self.write_file(path, content)?
            }
            "list_dir" => self.list_dir(arg_str(obj, "path")?)?,
            "run_command" => self.run_command(arg_str(obj, "cmd")?)?,
            "search" => {
                let needle = arg_str(obj, "needle")?;
                let path = arg_str(obj, "path")?;
                self.search(needle, path)?
            }
            other => return Err(Error::InvalidInput(format!("unknown tool \"{other}\""))),
        };
        Ok(ToolResult { ok: true, text })
    }

    /// Resolve a relative path inside the sandbox.
    fn resolve(&self, rel: &str) -> Result<PathBuf> {
        safe_join_rel(&self.root, rel)
    }

    fn read_file(&self, rel: &str) -> Result<String> {
        let path = self.resolve(rel)?;
        let meta = std::fs::metadata(&path)?;
        if meta.len() > MAX_READ_BYTES {
            return Err(Error::InvalidInput(format!(
                "file too large ({} bytes, cap {MAX_READ_BYTES})",
                meta.len()
            )));
        }
        let bytes = std::fs::read(&path)?;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        Ok(format!(
            "== {rel} ({} bytes) ==\n{text}",
            bytes.len()
        ))
    }

    fn write_file(&self, rel: &str, content: &str) -> Result<String> {
        let path = self.resolve(rel)?;
        if let Some(parent) = path.parent() {
            aether_core::fs::ensure_dir(parent)?;
        }
        std::fs::write(&path, content)?;
        Ok(format!("wrote {} bytes to {rel}", content.len()))
    }

    fn list_dir(&self, rel: &str) -> Result<String> {
        let path = if rel.is_empty() {
            self.root.clone()
        } else {
            self.resolve(rel)?
        };
        let mut entries: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(&path)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let kind = entry
                .file_type()
                .map(|t| if t.is_dir() { "dir" } else { "file" })
                .unwrap_or("?");
            entries.push(format!("{kind}\t{name}"));
        }
        entries.sort();
        if entries.is_empty() {
            return Ok(format!("(empty dir: {rel})"));
        }
        Ok(entries.join("\n"))
    }

    fn run_command(&self, cmd: &str) -> Result<String> {
        let output = run_with_timeout(cmd, &self.root, CMD_TIMEOUT)?;
        Ok(render_command_output(cmd, &output))
    }

    fn search(&self, needle: &str, rel: &str) -> Result<String> {
        let path = if rel.is_empty() {
            self.root.clone()
        } else {
            self.resolve(rel)?
        };
        let mut matches: Vec<String> = Vec::new();
        walk(&path, &mut |file| {
            if matches.len() >= 200 {
                return Ok(());
            }
            let text = std::fs::read_to_string(file).unwrap_or_default();
            for (idx, line) in text.lines().enumerate() {
                if line.contains(needle) {
                    let rel_path = file
                        .strip_prefix(&self.root)
                        .unwrap_or(file)
                        .display();
                    matches.push(format!("{rel_path}:{}:{line}", idx + 1));
                    if matches.len() >= 200 {
                        break;
                    }
                }
            }
            Ok(())
        })?;
        if matches.is_empty() {
            return Ok(format!("no matches for {needle:?} under {rel}"));
        }
        Ok(matches.join("\n"))
    }
}

/// Recursively visit files (not dirs) under `root`.
fn walk(root: &Path, f: &mut dyn FnMut(&Path) -> Result<()>) -> Result<()> {
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let is_dir = entry.file_type()?.is_dir();
        if is_dir {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".git" || name == "target" || name == "node_modules" {
                continue;
            }
            walk(&path, f)?;
        } else {
            f(&path)?;
        }
    }
    Ok(())
}

/// Run a shell command with a timeout, capturing capped stdout+stderr.
fn run_with_timeout(cmd: &str, cwd: &Path, timeout: Duration) -> Result<CommandOutput> {
    let mut child = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let (mut stdout, mut stderr) = match (child.stdout.take(), child.stderr.take()) {
        (Some(o), Some(e)) => (o, e),
        _ => return Err(Error::InvalidInput("failed to capture command pipes".to_string())),
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
fn wait_with_timeout(child: &mut std::process::Child, timeout: Duration) -> Option<std::process::ExitStatus> {
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
        let res = tools.call("write_file", &serde_json::json!({"path": "a/b.txt", "content": "hello"})).unwrap();
        assert!(res.ok);
        assert!(res.text.contains("wrote 5 bytes"));
        let res = tools.call("read_file", &serde_json::json!({"path": "a/b.txt"})).unwrap();
        assert!(res.text.contains("hello"));
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
        let res = tools.call("list_dir", &serde_json::json!({"path": ""})).unwrap();
        assert!(res.text.contains("file\ta.txt"));
        assert!(res.text.contains("file\tb.txt"));
        assert!(res.text.find("a.txt").unwrap() < res.text.find("b.txt").unwrap());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_command_ok_and_fail() {
        let root = temp_root("cmd");
        let tools = Tools::new(root.clone());
        let res = tools.call("run_command", &serde_json::json!({"cmd": "echo hi"})).unwrap();
        assert!(res.text.contains("hi"));
        assert!(res.text.contains("[exit 0]"));
        let res = tools.call("run_command", &serde_json::json!({"cmd": "false"})).unwrap();
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
        let res = tools.call("search", &serde_json::json!({"needle": "aether_run", "path": ""})).unwrap();
        assert!(res.text.contains("src/main.rs:1"));
        let res = tools.call("search", &serde_json::json!({"needle": "zzz", "path": ""})).unwrap();
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
}
