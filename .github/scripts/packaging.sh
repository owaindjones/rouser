#!/usr/bin/env bash
# Package a rouser binary + config + service into a tarball.
# Usage: ./packaging.sh package <binary-path> <output-tarball> [--version VERSION]
set -euo pipefail

ACTION="${1:-help}"
BINARY_PATH="${2:-}"
OUTPUT_TARBALL="${3:-}"
VERSION="${4:-dev}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

case "$ACTION" in
  package)
    if [ -z "$BINARY_PATH" ] || [ -z "$OUTPUT_TARBALL" ]; then
      echo "Usage: $0 package <binary-path> <output-tarball> [--version VERSION]" >&2
      exit 1
    fi

    TMPDIR=$(mktemp -d)
    trap 'rm -rf "$TMPDIR"' EXIT

    # Copy binary
    cp "$BINARY_PATH" "$TMPDIR/rouser"
    chmod +x "$TMPDIR/rouser"

    # Copy config — prefer config/rouser.toml, fall back to example
    if [ -f "$PROJECT_ROOT/config/rouser.toml" ]; then
      mkdir -p "$TMPDIR/config"
      cp "$PROJECT_ROOT/config/rouser.toml" "$TMPDIR/config/rouser.toml"
    elif [ -f "$PROJECT_ROOT/etc/rouser/config.toml.example" ]; then
      mkdir -p "$TMPDIR/config"
      cp "$PROJECT_ROOT/etc/rouser/config.toml.example" "$TMPDIR/config/rouser.toml"
    fi

    # Copy systemd service
    if [ -d "$PROJECT_ROOT/systemd" ] && ls "$PROJECT_ROOT/systemd/"*.service 1>/dev/null 2>&1; then
      mkdir -p "$TMPDIR/systemd"
      cp "$PROJECT_ROOT/systemd/"*.service "$TMPDIR/systemd/"
    fi

    tar czf "$OUTPUT_TARBALL" -C "$TMPDIR" .
    echo "Created $OUTPUT_TARBALL ($(du -h "$OUTPUT_TARBALL" | cut -f1))"
    ;;

  extract)
    # Extract binary + config + service from a release tarball into pkg-build/
    if [ -z "$BINARY_PATH" ]; then
      echo "Usage: $0 extract <tarball> [--dest DEST_DIR]" >&2
      exit 1
    fi

    DEST="${5:-pkg-build}"
    mkdir -p "$DEST"
    tar xzf "$BINARY_PATH" -C "$DEST" --strip-components=1 || true
    echo "Extracted $BINARY_PATH -> $DEST/"
    ;;

  *)
    echo "Usage: $0 {package|extract} [args...]" >&2
    exit 1
    ;;
esac
