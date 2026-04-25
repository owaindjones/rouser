#!/usr/bin/env bash
# Build an .rpm package for rouser from extracted files using rpmbuild.
# Usage: ./rpm.sh build <source-dir> <version> <target-arch>
set -euo pipefail

ACTION="${1:-help}"
SOURCE_DIR="${2:-pkg-build}"
VERSION="${3:-0.1.0}"
ARCH="${4:-x86_64}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
RPMBUILD="rpmbuild"

# Strip leading 'v' if present for RPM version field
RPM_VERSION=$(echo "$VERSION" | sed 's/^v//')

case "$ACTION" in
  build)
    TMPDIR=$(mktemp -d)
    trap 'rm -rf "$TMPDIR"' EXIT

    mkdir -p "$TMPDIR/BUILD" "$TMPDIR/SOURCES" "$TMPDIR/RPMS/$ARCH" "$TMPDIR/SRPMS"
    RPMTOP="$TMPDIR"

    # Prepare source tarball (contains just the binary for no-build RPM)
    mkdir -p "rouser-${RPM_VERSION}/config"
    cp "$SOURCE_DIR/rouser"       "rouser-${RPM_VERSION}/rouser" 2>/dev/null || true
    if [ -d "$SOURCE_DIR/config" ] && [ -f "$SOURCE_DIR/config/rouser.toml" ]; then
      cp "$SOURCE_DIR/config/rouser.toml" "rouser-${RPM_VERSION}/config/"
    elif [ -f "$PROJECT_ROOT/etc/rouser/config.toml.example" ]; then
      cp "$PROJECT_ROOT/etc/rouser/config.toml.example" "rouser-${RPM_VERSION}/config/rouser.toml"
    fi
    if [ -d "$SOURCE_DIR/systemd" ]; then
      mkdir -p "rouser-${RPM_VERSION}/systemd"
      cp "$SOURCE_DIR/systemd/"*.service "rouser-${RPM_VERSION}/systemd/" 2>/dev/null || true
    elif ls "$PROJECT_ROOT/systemd/"*.service 1>/dev/null 2>&1; then
      mkdir -p "rouser-${RPM_VERSION}/systemd"
      cp "$PROJECT_ROOT/systemd/"*.service "rouser-${RPM_VERSION}/systemd/"
    fi

    tar czf "SOURCES/rouser-source.tar.gz" "rouser-${RPM_VERSION}" 2>/dev/null || true
    rm -rf "rouser-${RPM_VERSION}"

    # SPEC file
    cat > "${RPMTOP}.rpm-spec" <<'SPECEOF'
%global debug_package %{nil}
Name:           rouser
Version:        @VERSION@
Release:        1%{?dist}
Summary:        System metrics daemon with sleep inhibition
License:        MIT
URL:            https://github.com/owaindjones/rouser
Source0:        %{name}-source.tar.gz

BuildRequires:  systemd-devel >= 240
BuildRequires:  dbus-devel >= 1.14

%description
A Linux daemon that monitors CPU, GPU, network, and disk
activity to prevent unwanted system suspend or hibernation
when activity thresholds are exceeded.

%install
mkdir -p %{buildroot}/usr/bin
cp rouser       %{buildroot}/usr/bin/
mkdir -p        %{buildroot}/etc/rouser
[ -f config/rouser.toml ] && cp config/rouser.toml   %{buildroot}/etc/rouser/config.toml || true
if [ -d systemd ]; then
  mkdir -p     %{buildroot}/lib/systemd/system
  cp           systemd/*.service    %{buildroot}/lib/systemd/system/ || true
fi

%files
/usr/bin/rouser
/etc/rouser/config.toml
/lib/systemd/system/rouser.service

%post
systemctl daemon-reload || true

%changelog
* $(date '+%a %b %d %Y') Release <release@example.com> - @VERSION@
- Build for release
SPECEOF

    sed -i "s|@VERSION@|${RPM_VERSION}|g" "${RPMTOP}.rpm-spec"

    $RPMBUILD \
      --define "_topdir $RPMTOP" \
      --target "$ARCH" \
      -bb "${RPMTOP}.rpm-spec" 2>&1 || true

    # Copy built RPM out of temp dir
    if ls "$RPMTOP/RPMS/$ARCH/rouser-${RPM_VERSION}"*.rpm 1>/dev/null 2>&1; then
      cp "$RPMTOP/RPMS/$ARCH/"*.rpm . 2>/dev/null || true
    fi

    rm -rf "rouser-${RPM_VERSION}" SOURCES/rouser-source.tar.gz "${RPMTOP}.rpm-spec"
    echo "Built RPM for $ARCH (version $RPM_VERSION)"
    ;;

  *)
    echo "Usage: $0 build <source-dir> <version> <target-arch>" >&2
    exit 1
    ;;
esac
