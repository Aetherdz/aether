#!/usr/bin/env bash
# aether — official install/update script.
# Downloads the prebuilt binary for your OS/arch from GitHub Releases,
# verifies its SHA-256 against the published checksums, and installs it.
#
# Running this script again acts as an update:
#   - same version installed  -> exits early, nothing to do
#   - older version installed -> upgrades in place
#   - newer version installed -> warns, keeps the newer one unless pinned
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/Aetherdz/aether/main/scripts/install.sh | bash
#
# Overrides (env vars):
#   AETHER_VERSION      release tag to install/update to (default: latest)
#   AETHER_INSTALL_DIR  install directory (default: ~/.local/bin, falls back to /usr/local/bin)
#   AETHER_FORCE        set to 1 to force a reinstall even when the installed
#                       version already matches the target (default: unset)
#
# Safety:
#   This script ONLY replaces the `aether` binary. Your config and sessions
#   live in ~/.config/aether/ (or $AETHER_CONFIG_DIR) and are NEVER touched.
set -euo pipefail

REPO="Aetherdz/aether"
VERSION="${AETHER_VERSION:-latest}"
INSTALL_DIR="${AETHER_INSTALL_DIR:-}"

log()  { printf '\033[1;32m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m==>\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31m==>\033[0m %s\n' "$*" >&2; exit 1; }

command -v curl >/dev/null 2>&1 || die "curl is required but was not found on PATH"
command -v mktemp >/dev/null 2>&1 || die "mktemp is required but was not found on PATH"

# --- compare dotted versions (strips a leading 'v'); echoes -1 | 0 | 1 --------
ver_cmp() {
  local a="${1#v}" b="${2#v}" ra="$1" rb="$2" pa pb
  ra="${ra#v}"; rb="${rb#v}"
  while :; do
    pa="${ra%%.*}"; pb="${rb%%.*}"
    [ -n "$pa" ] || pa=0
    [ -n "$pb" ] || pb=0
    if [ "$pa" -gt "$pb" ] 2>/dev/null; then echo 1; return; fi
    if [ "$pa" -lt "$pb" ] 2>/dev/null; then echo -1; return; fi
    case "$ra" in *.*) ra="${ra#*.}";; *) ra="" ;; esac
    case "$rb" in *.*) rb="${rb#*.}";; *) rb="" ;; esac
    if [ -z "$ra" ] && [ -z "$rb" ]; then echo 0; return; fi
    if [ -z "$ra" ]; then echo -1; return; fi
    if [ -z "$rb" ]; then echo 1; return; fi
  done
}

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
# Prefer the release-page redirect (no API, no rate limit): GitHub answers
# https://github.com/<repo>/releases/latest with a 302 to .../releases/tag/<tag>.
# Fall back to the API only if that redirect cannot be read.
if [ "$VERSION" = "latest" ]; then
  VERSION="$(
    curl -fsSI --max-time 20 "https://github.com/${REPO}/releases/latest" 2>/dev/null \
      | awk 'tolower($1)=="location:" {print $2}' \
      | sed -n 's#.*/tag/##p' \
      | tr -d '\r'
  )"
  if [ -z "$VERSION" ]; then
    warn "could not resolve latest via release redirect, falling back to GitHub API"
    VERSION="$(curl -fsSL --max-time 20 "https://api.github.com/repos/${REPO}/releases/latest" \
      | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)"
  fi
  [ -n "$VERSION" ] || die "could not resolve latest release"
fi

# Normalize to the tag convention (releases are tagged vX.Y.Z).
case "$VERSION" in
  v*) ;;
  *) VERSION="v${VERSION}" ;;
esac

BASE="https://github.com/${REPO}/releases/download/${VERSION}"
log "aether ${VERSION} (${OS}/${ARCH})"

# --- decide install dir (before downloading: update check needs it) -----------
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

# --- update check --------------------------------------------------------------
if [ -x "$DEST" ]; then
  INSTALLED_VER="$(
    { "$DEST" --version 2>/dev/null || true; } \
      | head -1 \
      | sed -E 's/^[^0-9]*//; s/ .*$//'
  )"
  if [ -n "$INSTALLED_VER" ]; then
    case "$(ver_cmp "$INSTALLED_VER" "$VERSION")" in
      0)
        if [ "${AETHER_FORCE:-}" = "1" ]; then
          log "aether ${INSTALLED_VER} already installed — forcing reinstall"
        else
          log "aether ${INSTALLED_VER} is already installed and up to date"
          log "re-run with AETHER_VERSION=<tag> to switch to a specific version"
          log "or AETHER_FORCE=1 to reinstall the same version"
          exit 0
        fi
        ;;
      1)
        warn "installed version ${INSTALLED_VER} is newer than target ${VERSION}"
        if [ "${AETHER_FORCE:-}" = "1" ]; then
          warn "AETHER_FORCE=1 — replacing it anyway"
        else
          warn "keeping the newer binary (set AETHER_FORCE=1 to replace it)"
          exit 0
        fi
        ;;
    esac
    log "updating aether ${INSTALLED_VER} -> ${VERSION}"
  fi
fi

# --- download + verify -------------------------------------------------------
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

curl -fsSL --max-time 120 "${BASE}/${ASSET}" -o "$TMP/aether.bin" \
  || die "download failed: ${BASE}/${ASSET}"
curl -fsSL --max-time 60 "${BASE}/SHA256SUMS.txt" -o "$TMP/SHA256SUMS.txt" \
  || die "could not fetch checksums"

EXPECTED="$(awk -v a="$ASSET" '$2 == a { print $1 }' "$TMP/SHA256SUMS.txt")"
[ -n "$EXPECTED" ] || die "no checksum found for ${ASSET}"

if command -v sha256sum >/dev/null 2>&1; then
  ACTUAL="$(sha256sum "$TMP/aether.bin" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
  ACTUAL="$(shasum -a 256 "$TMP/aether.bin" | awk '{print $1}')"
else
  die "no sha256 tool found (install coreutils or run \`brew install coreutils\`)"
fi

[ "$ACTUAL" = "$EXPECTED" ] || die "checksum mismatch (expected ${EXPECTED}, got ${ACTUAL})"
log "checksum verified (${ACTUAL:0:16}…)"

# --- install -------------------------------------------------------------------
if command -v install >/dev/null 2>&1; then
  install -m 0755 "$TMP/aether.bin" "$DEST"
else
  cp "$TMP/aether.bin" "$DEST"   # fallback for minimal MSYS/Windows shells
  chmod 0755 "$DEST" 2>/dev/null || true
fi

log "installed to ${DEST}"
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) warn "${INSTALL_DIR} is not on your PATH; add it with:"
     warn "  export PATH=\"${INSTALL_DIR}:\$PATH\"" ;;
esac

"$DEST" --version
log "done — try: aether ask \"hello\" (no API key needed)"
log "config + sessions in ~/.config/aether/ are untouched"
