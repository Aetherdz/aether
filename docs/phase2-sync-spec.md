# Phase 2 — aetherdz-sync: THE FLAGSHIP — Ready-to-fire spec
*jcode has ZERO cross-device sync (verified: jcode-storage/src = lib.rs + active_pids.rs only; grep gist/dropbox = 0 hits). This is our #1 differentiator (blueprint §9.1).*

## Reference TS (READ BEFORE CODING)
- /home/abdozaik720/aether-cli/src/sync.ts (294 LOC) — THE contract: gist (GitHub API) + folder backends, sync.json state, device-id, bundle v1 `{v, deviceId, sessions}`, timestamp-sorted line de-dup merge (mergeLines), mergeBundles, write-new + merge-existing.
- Blueprint §4 (aetherdz-sync crate), §2.2 item 4, §10 CLI: `aether sync setup gist|folder / push / pull / status`.
- jcode-storage append pattern (lib.rs:672-685) for integrity.

## Data model (copy EXACTLY from TS — coexist with TS version)
- Bundle v1 JSON: `{"v":1,"deviceId":"<uuid>","sessions":{...}}` — read sync.ts for exact field shape.
- State file: same path TS uses (`sync.json` — find exact path in sync.ts).
- Merge semantics: timestamp-sorted, per-line de-dup (mergeLines) — the ORDER + dedup rules must match TS byte-for-byte semantics (this is the test oracle).

## Backends
1. **Gist**: GitHub REST API via reqwest (no SDK). Create/update gist, auth via `GITHUB_TOKEN` env or existing TS key store — read the TS to see exact API calls + list of gist id / sync.json storage.
2. **Folder**: plain local dir (useful: two dirs on same machine — makes round-trip test possible offline).

## Crate: aetherdz-sync
Public API:
- `Sync::push(backend) -> Result<()>`
- `Sync::pull(backend) -> Result<SyncReport>`
- `Merge::three_way / merge_bundles(local, remote) -> MergedBundle` (line-level)
- `Backend::{Gist{id}, Folder{path}}`
- `Sync::status() -> SyncStateView`

## Tests (gate: cargo test + clippy -D warnings)
1. merge_bundles: local-only lines + remote-only lines + same-line dedup → union, no dupes (golden: compare to a hand-written expected JSONL)
2. push→pull round-trip on TWO folder backends preserves all lines in order
3. state file after push == TS-equivalent shape (coexist test: if TS version can read it — describe how to verify)
4. device-id stable across runs

## CLI (extend aetherdz-cli)
`aether sync setup gist|folder <arg>` / `aether sync push` / `aether sync pull` / `aether sync status`

## Security gates
- Gist token: never logged/printed; redact in debug; no shell.
- Path traversal: folder backend paths validated (isInside pattern from TS tools/files.ts).
- Rate-limit aware: GitHub API 5000/h unauth — set reqwest timeout + handle 429 with backoff (TS has hardcoded 8s/15s/30s — reuse).
- Bundle integrity: JSON parse errors → clear error, never partial-write (atomic via core fs).

## MUST-run at end
1. cargo build --release (0 errors)
2. cargo test (≥4 pass)
3. clippy -D warnings
4. Run: `aether sync setup folder /tmp/synctest-a` + `push` + `pull` from a second dir → verify merge output matches expected JSONL
5. Print file tree + `aether sync --help` + verification transcript.