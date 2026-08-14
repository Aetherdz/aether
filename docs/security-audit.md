# Security Audit — Aether

Date: 2026-08-14
Scope: full workspace (8 crates), `aether-agent` sandbox deep-dive.
Toolchain: rustc 1.97.1, cargo 1.97.1, Linux kernel 6.6.143.

---

## 1. Dependency audit — `cargo audit`

Command: `cargo audit --json` (cargo-audit v0.22.2)

| Metric | Value |
|---|---|
| Crate dependencies scanned | 357 |
| Advisory database size | 1,216 advisories |
| Database last updated | 2026-08-12 |
| Vulnerabilities found | **0** |
| Informational warnings (unmaintained/unsound/notice) | 0 |

Result: **clean**. No known vulnerable or unmaintained dependency in the
lockfile. No `cargo update` or dependency pin change was required.

Reproduce:

```sh
cargo audit --json
```

---

## 2. Static analysis — `cargo clippy`

Command:

```sh
cargo clippy --workspace --all-targets -- -D warnings
```

Result: **exit 0, zero warnings** across all 8 crates including all test
targets. `-D warnings` makes every lint a hard error, so the build is
lint-clean by construction.

Reproduce:

```sh
cargo clippy --workspace --all-targets -- -D warnings
```

---

## 3. Sandbox review — `aether-agent` filesystem isolation

### 3.1 Finding (fixed)

**Vulnerability class: sandbox escape via path tricks (symlink + TOCTOU).**

Before this audit, the "sandbox" was a lexical blacklist in
`aether_core::fs::safe_join_rel`: it rejected `..`, absolute paths and NUL
bytes by string inspection, then handed the joined `PathBuf` to `std::fs`.

That model is bypassable in two ways:

1. **Symlink escape.** A symlink *inside* the root pointing outside
   (e.g. `root/evil -> /etc`) passes the lexical check (`evil/passwd`
   contains no `..`), and `std::fs::read(root/evil/passwd)` follows it out
   of the sandbox.
2. **TOCTOU (time-of-check/time-of-use).** The check (`safe_join_rel`) and
   the use (`std::fs::open`) are separate syscalls. An attacker who can
   swap a directory for a symlink between the two wins the race and the
   "validated" path resolves outside.

### 3.2 Fix — capability handle (`cap-std`)

`aether_core::sandbox::Sandbox` opens the working root **once** as a
`cap_std::fs::Dir` capability handle. On Linux, cap-std resolves every
operation with `openat2(2)` + `RESOLVE_BENEATH`: the kernel walks the whole
path relative to the handle's fd and atomically rejects any component that
escapes the root. Consequences:

- There is **no validate-then-open window** to race — resolution and the
  open are a single syscall (TOCTOU eliminated by construction).
- An escaping symlink is rejected **by the kernel at open time**, so the
  blacklist gap is closed.
- The lexical `safe_join_rel` check is retained **only** to produce a clear
  `Error::PathTraversal` message early; it is no longer the security
  boundary.

All `aether-agent` filesystem tools now route through the handle:

| Tool | Before | After |
|---|---|---|
| `read_file` | `std::fs::metadata` + `std::fs::read` on joined path | `Sandbox::metadata` + `Sandbox::read` |
| `write_file` | `atomic_write` on joined path | `Sandbox::atomic_write` (temp + rename via handle) |
| `list_dir` | `std::fs::read_dir` on joined path | `Sandbox::read_dir` |
| `search` | recursive `std::fs` walk on joined path | `Sandbox::walk_files` |
| `undo` journal | `safe_join_rel` + `std::fs` | `Sandbox` (snapshot write, restore, list, delete) |

Platform note: the TOCTOU-free guarantee is strongest on Linux (openat2).
On macOS/Windows cap-std falls back to capability-aware `openat` semantics
with per-component checking — still confined to the handle, but the
kernel-level single-syscall guarantee is Linux-specific. `run_command` is
**not** filesystem-sandboxed (it spawns `/bin/sh -c` with `current_dir` =
root); process-level isolation and its approval flow are tracked separately
(see §5).

### 3.3 Adversarial test suite — 10 escape attempts, all rejected

All tests live in `crates/aether-agent/src/tools.rs` (`escape_*`) and each
one *actually attempts* the escape through the public tool API. Result:
**10/10 passed** (each attempt fails).

| # | Attack vector | Test |
|---|---|---|
| 1 | Absolute path (`/etc/passwd`, `/etc`, `/`) | `escape_absolute_path_blocked` |
| 2 | `..` traversal (`../`, `a/../../b`, `a/../../../etc/passwd`) | `escape_dotdot_blocked` |
| 3 | External symlink inside root → `/etc` (`evil/passwd`) | `escape_external_symlink_blocked` |
| 4 | TOCTOU dir→symlink swap after handle open (read + write) | `escape_toctou_dir_swap_blocked` |
| 5 | Process CWD/env manipulation (`chdir("/")` then read) | `escape_cwd_env_does_not_reroot` |
| 6 | Unicode homoglyph of `..` (U+2024, U+FF0E) | `escape_unicode_homoglyph_blocked` |
| 7 | NUL byte (`a\0b`) | `escape_nul_byte_blocked` |
| 8 | Windows backslash separator (`..\..\etc\passwd`, `C:\...`) | `escape_backslash_blocked` |
| 9 | Embedded `.`/`..` after valid prefix (`a/./../etc/passwd`) | `escape_embedded_dotdot_blocked` |
| 10 | Symlink loop inside root (`a↔b`) — must error, not hang/escape | `escape_symlink_loop_blocked` |

Reproduce:

```sh
cargo test -p aether-agent --lib escape_
```

---

## 4. Residual risks (honest)

1. **`run_command` is not filesystem-sandboxed.** A shell command can
   `cd .. && cat /etc/passwd`. The filesystem *tools* are now capability-
   confined; command execution is a separate boundary. Mitigation shipped:
   a classification-based approval gate for dangerous commands (destructive
   `rm` outside the project, `sudo`/`su`, system control, `curl|sh`
   remote-exec, publish actions) that pauses for `y/N` confirmation under
   the interactive policy and refuses on a non-TTY stdin without `--yes`.
   Per-project bypass via `.aether-allowlist` (one command prefix per line)
   or `AETHER_AGENT_ALLOW` (comma/newline separated). The gate is a policy
   layer, not a kernel boundary — a user-`y` (or allowlisted) command still
   has full shell power, so treat `--yes` / allowlist entries as explicit
   trust grants.
2. **Non-Linux platforms** get cap-std's capability-aware fallback, not the
   kernel-level `openat2` guarantee. Same confinement, weaker race
   guarantees; tests above run on Linux CI.
3. **No secrets scanning / SAST in CI yet.** `cargo audit` is a supply-chain
   check, not a code-audit. Recommend adding `cargo deny` (licenses +
   advisories) and a secrets scanner to CI.

---

## 5. Verdict

Dependency and lint posture is clean. The previously exploitable
sandbox (lexical blacklist) has been replaced with a capability handle
whose boundary is enforced by the kernel at open time, and 10 distinct
escape attempts are now covered by tests that fail on any regression.