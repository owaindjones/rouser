#!/usr/bin/env bash
# Build a .deb package for rouser from extracted files.
# Usage: ./deb.sh build <source-dir> <output-deb> <version> <arch>
set -euo pipefail

ACTION="${1:-help}"
SOURCE_DIR="${2:-pkg-build}"
OUTPUT_DEB="${3:-rouser.deb}"
VERSION="${4:-0.1.0}"
ARCH="${5:-amd64}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

case "$ACTION" in
  build)
    DEB_PKG="rouser-debian"
    mkdir -p "$DEB_PKG/DEBIAN"
    mkdir -p "$DEB_PKG/usr/local/bin"
    mkdir -p "$DEB_PKG/etc/rouser"
    mkdir -p "$DEB_PKG/lib/systemd/system"

    # Binary
    if [ -f "$SOURCE_DIR/rouser" ]; then
      cp "$SOURCE_DIR/rouser" "$DEB_PKG/usr/local/bin/rouser"
    fi

    # Config
    if [ -d "$SOURCE_DIR/config" ] && [ -f "$SOURCE_DIR/config/rouser.toml" ]; then
      cp "$SOURCE_DIR/config/rouser.toml" "$DEB_PKG/etc/rouser/config.toml"
    elif [ -f "$PROJECT_ROOT/etc/rouser/config.toml.example" ]; then
      cp "$PROJECT_ROOT/etc/rouser/config.toml.example" "$DEB_PKG/etc/rouser/config.toml"
    fi

    # Systemd service
    if [ -d "$SOURCE_DIR/systemd" ]; then
      cp "$SOURCE_DIR/systemd/"*.service "$DEB_PKG/lib/systemd/system/" 2>/dev/null || true
    elif ls "$PROJECT_ROOT/systemd/"*.service 1>/dev/null 2>&1; then
      cp "$PROJECT_ROOT/systemd/"*.service "$DEB_PKG/lib/systemd/system/"
    fi

    # Control file
    cat > "$DEB_PKG/DEBIAN/control" <<CTRLEOF
Package: rouser
Version: ${VERSION}
Section: utils
Priority: optional
Architecture: ${ARCH}
Maintainer: Owain Jones <contact@odj.me>
Depends: systemd, libdbus-1-0
Description: System metrics daemon with sleep inhibition
 A Linux daemon that monitors CPU, GPU, network, and disk
 activity to prevent unwanted system suspend/hibernation.
 Uses NVML for NVIDIA GPUs; sysfs for AMD (amdgpu) and Intel (i915/xe).
CTRLEOF

    # Checksums and postinst are optional but good practice
    if command -v md5sum >/dev/null 2>&1; then
      cd "$DEB_PKG" && find usr etc lib -type f | xargs md5sum > DEBIAN/md5sums 2>/dev/null || true
      cd - >/dev/null
    fi

    dpkg-deb --build --root-owner-group "$DEB_PKG/" "$OUTPUT_DEB"
    rm -rf "$DEB_PKG"
    echo "Built $OUTPUT_DEB ($(du -h "$OUTPUT_DEB" | cut -f1))"
    ;;

  *)
    echo "Usage: $0 build <source-dir> <output-deb> <version> <arch>" >&2
    exit 1
    ;;
esac
