# Aether documentation

This folder holds the design and migration documents for the Aether project.
Read them in this order for the full picture.

## Start here

| Doc | What it covers | When to read |
|---|---|---|
| [rust-migration-blueprint.md](rust-migration-blueprint.md) | Architecture, crate layout, phased migration plan, risks | First — the master plan |
| [build-plan-v2.md](build-plan-v2.md) | Second-pass build plan with sequencing and status | After the blueprint |
| [inspiration-extraction.md](inspiration-extraction.md) | Design notes extracted from the original AETHER | Optional background |

## Phase specs

Each phase spec defines the behavior contract for one slice of the system.
Golden tests verify implementations against these contracts.

| Phase | Doc | Delivers |
|---|---|---|
| 1 | [phase1-session-spec.md](phase1-session-spec.md) | JSONL sessions, auto-title, recall, usage ledger |
| 2 | [phase2-sync-spec.md](phase2-sync-spec.md) | Gist/folder sync, line-level merge |
| 3 | [phase3-mcp-spec.md](phase3-mcp-spec.md) | MCP server (stdio + Streamable HTTP) |
| 4 | [phase4-tools-agent-auth-tui-spec.md](phase4-tools-agent-auth-tui-spec.md) | Sandboxed tools, 3-model agent loop, TUI |

## Related

- [SECURITY.md](../SECURITY.md) — threat model and security policy
- [CONTRIBUTING.md](../CONTRIBUTING.md) — how to contribute
- [golden-tests/README.md](../golden-tests/README.md) — behavior verification
