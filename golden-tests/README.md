# Golden Test Harness
## Purpose
Differential oracle: every Rust command output vs the SAME command in the TS
reference (aether-cli dist). This is blueprint §7-risks-5 enforcement.

## How to capture TS golden output (no API key needed for surface commands)
  cd /home/abdozaik720/aether-cli && node dist/index.js <cmd> > ../aetherdz/golden-tests/fixtures/<cmd>.golden 2>&1
Capture: --help, sessions list (empty), sync status (no setup), models (static list).

## Rust harness (run after Phase 0 lands)
  aether <cmd> > golden-tests/out/<cmd>.out
  diff golden-tests/fixtures/<cmd>.golden golden-tests/out/<cmd>.out || echo "MISMATCH"
