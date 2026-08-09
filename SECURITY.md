# Security Policy

Aether is a local-first terminal agent that can read files and execute
commands on your behalf. This document is the threat model: what the
tool can and cannot do, where the trust boundaries are, and how to
report a vulnerability.

## Supported versions

Only the latest release on `main` is supported. Security fixes land on
`main` first and are backported only to the most recent tagged release.

| Version | Supported |
|---|---|
| latest release | ✅ |
| older releases | ❌ |

## Reporting a vulnerability

- **Do not open a public issue** for a security problem.
- Email the maintainers, or open a GitHub Security Advisory
  (https://github.com/Aetherdz/aether/security/advisories/new).
- Include: affected version, a minimal reproduction, and your
  assessment of impact.
- You will get an acknowledgement within 48 hours and a fix timeline
  as soon as triage is done.
- We follow coordinated disclosure: public disclosure happens after a
  fix ships.

## Trust model

Aether runs with **your user privileges**. It is a tool you invoke in a
directory you choose — it is not a sandbox and does not pretend to be one.
The guarantees below are about *accidental* harm (a confused model doing
something destructive), not *malicious* code running as your user.

| Asset | Guarantee |
|---|---|
| Files outside the working dir | Protected by path sandboxing in file tools (see below) |
| Files inside the working dir | **Not** protected — the agent is *supposed* to edit them |
| Arbitrary shell commands | **Not** protected — `run_command` is full shell access with a timeout |
| API keys / credentials | Never written to sessions or logs (see below) |
| Network | Not isolated — the agent can make network calls via `run_command` |

### The sandbox boundary

File tools (`read_file`, `write_file`, `list_dir`, `search`) resolve every
path through `safe_join_rel`, which:

- allows multi-level relative paths (`a/b/c`);
- **rejects** absolute paths, `..` escapes, `.`, empty paths, NUL bytes,
  and doubled separators (`a//b`);
- rejects by lexical check first, then canonicalizes where possible;
- never coerces an absolute path by stripping a prefix — rejection is
  rejection, not rewriting.

This boundary protects **the rest of your disk** from a confused agent that
tries to read `/etc/passwd` or `../../.ssh/id_rsa`. It does **not** protect
the working directory itself.

### `run_command` — the explicit escape hatch

`run_command` executes via `/bin/sh -c` with:

- a **30-second timeout** (killed on expiry);
- a **128 KB output cap** (truncated, never streamed unbounded);
- working directory set to the sandbox root;
- **no allow-list, no network isolation, no privilege drop**.

This is a deliberate design: the agent loop is only useful if it can build,
test, and run tools. The cost is that `run_command` can do anything *you*
can do from that shell. **Treat `aether agent` in a directory as granting
shell access to that directory.**

Mitigations in place and planned:

- [x] timeout + output cap
- [x] path sandbox for file tools
- [ ] per-command confirmation prompt before `run_command` (Phase 2)
- [ ] diff preview before `write_file` (Phase 2)
- [ ] snapshot/undo before writes (Phase 2)

## Secret handling

- API keys are read from environment variables (`OPENAI_API_KEY`,
  `ANTHROPIC_API_KEY`, …) or the config file.
- Keys are **never** written into session JSONL, recall indexes, sync
  payloads, or logs.
- The config file permissions are set to user-only on write.

If you find a code path that leaks a key into a session or log, that is a
security bug — report it.

## Threat scenarios

| Scenario | Outcome |
|---|---|
| Agent reads `../../etc/passwd` | ✅ blocked by `safe_join_rel` |
| Agent writes `../victim.txt` | ✅ blocked |
| Agent runs `rm -rf ~` via `run_command` | ❌ **executes** — user has full shell access |
| Agent exfiltrates files via `curl` | ❌ **executes** — network is not isolated |
| Agent leaks API key into a session file | ✅ prevented by design; report if found |
| Malicious crate in dependency tree | ⚠️ audited at release; SBOM published with each release |
| Tampered release binary | ⚠️ releases are signed (sigstore) and built reproducibly |

## Dependencies

Planned hardening (Phase 1 of the roadmap):

- [ ] `cargo audit` added to CI on every push
- [ ] SBOM (Software Bill of Materials) published with every release
- [ ] Release binaries signed via sigstore and built reproducibly
      (same commit → same binary)

Until then, dependency review is manual: the crate set is small and pinned
in `Cargo.lock`.
