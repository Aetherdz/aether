# Golden Test Harness
## Purpose
Differential oracle: every Rust command output vs the SAME command in the TS
reference (aether-cli dist). This is blueprint §7-risks-5 enforcement.

## How to capture TS golden output (no API key needed for surface commands)
From the repo root, with the TS reference checkout at `../aether-cli` (or set
`TS_CLI_DIR=/path/to/aether-cli`):

  REFRESH_FIXTURES=1 bash golden-tests/run.sh

## Rust harness (run after Phase 0 lands)
  aether <cmd> > golden-tests/out/<cmd>.out
  diff golden-tests/fixtures/<cmd>.golden golden-tests/out/<cmd>.out || echo "MISMATCH"
