# Aether benchmark

Reproducible performance measurements for Aether vs. competing CLI agents.

## Status

Under construction — the benchmark scripts and results will land here in
Phase 1 of the roadmap. The README currently makes **architectural claims**
(one native binary, no interpreter cold start) rather than measured numbers.

## What will be measured

- **Startup time** (ms): time from process spawn to first output, for
  `aether --help` vs `aider --help` / `claude --help` equivalents, on the
  same machine, averaged over N runs.
- **Memory footprint** (RSS): peak resident set during idle and during a
  short session.
- **Binary size**: `ls -lh` of the installed artifact.
- **Dependency footprint**: `du -sh` of the installed dependency tree
  (`cargo tree` vs `pip show`/`npm ls`).

## Methodology commitments

- Same machine, same conditions, multiple runs, median reported.
- The exact script is committed here so anyone can re-run it.
- No cherry-picked numbers: the script prints all raw measurements.
