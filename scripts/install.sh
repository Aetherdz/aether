#!/usr/bin/env bash
# aether — official install script.
# Downloads the prebuilt binary for your OS/arch from GitHub Releases,
# verifies its SHA-256 against the published checksums, and installs it.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/Aetherdz/aether/main/scripts/install.sh | bash
#
# Overrides (env vars):
#   AETHER_VERSION    release tag to install (default: latest)
#   AETHER_INSTALL_DIR  install directory (default: ~/.local/bin, falls back to /usr/local/bin)
set -euo pipefail

REPO="Aetherdz/aether"
VERSION="${AETHER_VERSION:-latest}"
INSTALL_DIR="${AETHER_INSTALL_DIR:-}"

log()  { printf '\033[1;32m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m==>\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31m==>\033[0m %s\n' "$*" >&2; exit 1; }

# --- detect OS ---------------------------------------------------------------
case "$(uname -s)" in
  Linux)  OS="linux" ;;
  Darwin) OS="macos" ;;
  MINGW*|MSYS*|CYGWIN*) OS="windows" ;;
  *) die "unsupported OS: $(uname -s)" ;;
esac

# --- detect arch -------------------------------------------------------------
case "$(uname -m)" in
  x86_64|amd64)  ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
  *) die "unsupported architecture: $(uname -m)" ;;
esac

ASSET="aether-${OS}-${ARCH}"
[ "$OS" = "windows" ] && ASSET="${ASSET}.exe"

# --- resolve version ---------------------------------------------------------
if [ "$VERSION" = "latest" ]; then
  VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)"
  [ -n "$VERSION" ] || die "could not resolve latest release"
fi

BASE="https://github.com/${REPO}/releases/download/${VERSION}"
log "installing aether ${VERSION} (${OS}/${ARCH})"

# --- download + verify -------------------------------------------------------
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

curl -fsSL "${BASE}/${ASSET}" -o "$TMP/aether.bin" \
  || die "download failed: ${BASE}/${ASSET}"
curl -fsSL "${BASE}/SHA256SUMS.txt" -o "$TMP/SHA256SUMS.txt" \
  || die "could not fetch checksums"

EXPECTED="$(awk -v a="$ASSET" '$2 == a { print $1 }' "$TMP/SHA256SUMS.txt")"
[ -n "$EXPECTED" ] || die "no checksum found for ${ASSET}"

if command -v sha256sum >/dev/null 2>&1; then
  ACTUAL="$(sha256sum "$TMP/aether.bin" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
  ACTUAL="$(shasum -a 256 "$TMP/aether.bin" | awk '{print $1}')"
else
  die "no sha256 tool found (install coreutils or run `brew install coreutils`)"
fi

[ "$ACTUAL" = "$EXPECTED" ] || die "checksum mismatch (expected ${EXPECTED}, got ${ACTUAL})"
log "checksum verified (${ACTUAL:0:16}…)"

# --- install -----------------------------------------------------------------
if [ -z "$INSTALL_DIR" ]; then
  if [ -d "$HOME/.local/bin" ] || [ -w "$HOME" ]; then
    INSTALL_DIR="$HOME/.local/bin"
  else
    INSTALL_DIR="/usr/local/bin"
  fi
fi
mkdir -p "$INSTALL_DIR"

DEST="${INSTALL_DIR}/aether"
[ "$OS" = "windows" ] && DEST="${DEST}.exe"
install -m 0755 "$TMP/aether.bin" "$DEST"

log "installed to ${DEST}"
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) warn "${INSTALL_DIR} is not on your PATH; add it with:"
     warn "  export PATH=\"${INSTALL_DIR}:\$PATH\"" ;;
esac

"$DEST" --version
log "done — try: aether ask \"hello\" (no API key needed)"
