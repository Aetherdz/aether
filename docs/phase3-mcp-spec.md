# Phase 3 — aetherdz-mcp: stdio + Streamable HTTP + SSE — Ready-to-fire spec
*jcode weakness #1 (verified): MCP stdio-only, HTTP/SSE recognized & skipped (protocol.rs:667-676). We ship 3 transports — our edge (blueprint §9.2).*

## Reference TS (READ FIRST)
- /home/abdozaik720/aether-cli/src/mcp.ts (273 LOC) — the current lean stdio JSON-RPC client (proto 2024-11-05), zero deps: config at ~/.config/aether/mcp.json, spawn/handshake/tools/list/call, 8s timeout, allow-list, buildMcpTools → ai-sdk tool().
- Blueprint §4 (aetherdz-mcp), §5 (rmcp decision + the SSE gap note), §10 CLI: `aether mcp` lists servers.
- jcode reference (what we EXTEND): crates/jcode-base/src/mcp/protocol.rs:188-249 (config struct + is_stdio), client.rs (stdio handle).

## Stack (blueprint §5 decision — final)
- **rmcp 3.x** (official modelcontextprotocol/rust-sdk) for stdio + Streamable HTTP client (reqwest) + SSE parsing (`client-side-sse` feature).
- Legacy 2024-11-05 two-endpoint HTTP+SSE: thin hand-rolled transport **only if** a config entry needs it — read rmcp docs; if rmcp covers streamable-http's SSE streaming semantics, legacy SSE is a small adapter struct implementing rmcp's Transport trait.
- tokio, serde, reqwest 0.12 rustls.

## Crate: aetherdz-mcp
Public API:
- `McpClient::connect_stdio(command)` — spawn child, JSON-RPC over stdin/stdout (port TS handshake: initialize → tools/list → tools/call; 8s timeout; allowlist).
- `McpClient::connect_streamable_http(url)` (rmcp).
- `McpClient::connect_sse(url)` (legacy adapter).
- `McpRegistry::load(config_path)` — parse mcp.json (copy TS field shape), validate allowlist.
- `McpRegistry::list_tools(server) -> Vec<ToolSpec>`.

## Config compat (coexist with TS)
- Read SAME mcp.json layout from blueprints § ≥10 (server name, type: stdio|http|sse, command, args, env, allowlist). If TS config has only stdio entries, our HTTP/SSE reader adds fields WITHOUT breaking TS parse.

## Tests (gate: cargo test + clippy -D warnings)
1. stdio connect to a **fake local MCP server** (test fixture: a small script that answers initialize + tools/list JSON-RPC) — handshake succeeds, tools listed. NO live network in unit tests.
2. allowlist enforcement: tool not in allowlist → error, tool call aborted.
3. 8s timeout: fake server that sleeps 10s → timeout error, no hang.
4. config parse: TS-shaped mcp.json parses; bad JSON → clear error.
5. (integration, optional) connect to one REAL public stdio MCP server if network available (mark #[ignore]).

## CLI
`aether mcp` (list servers+status), `aether mcp call <server> <tool> [args]` (debug).

## Security gates
- Allowlist is the boundary: never auto-approve tools not present; no shell interpolation in command args (Vec<String> exec, no shell).
- Child process: kill on drop (RAII), no zombie; timeout enforced.
- No secrets logged; env passed to child = config env only, redact in logs.
- URL validation for HTTP/SSE: https only (except localhost http for dev), reject other schemes.