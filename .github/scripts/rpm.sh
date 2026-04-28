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
    RPMBUILD_DIR=$(mktemp -d)
    trap 'rm -rf "$RPMBUILD_DIR"' EXIT
    
    # Set up standard rpmbuild directory structure
    mkdir -p "${RPMBUILD_DIR}/BUILD"              "${RPMBUILD_DIR}/RPMS/$ARCH"              "${RPMBUILD_DIR}/SOURCES"              "${RPMBUILD_DIR}/SPECS"              "${RPMBUILD_DIR}/SRPMs"

    # Build source tarball content (all under RPMBUILD_DIR)  
    SOURCE_NAME="rouser-${RPM_VERSION}"
    mkdir -p "${RPMBUILD_DIR}/SOURCES/${SOURCE_NAME}/config"              "${RPMBUILD_DIR}/SOURCES/${SOURCE_NAME}/systemd"
    
    cp "$SOURCE_DIR/rouser"       "${RPMBUILD_DIR}/SOURCES/${SOURCE_NAME}/rouser" 2>/dev/null || true
    if [ -d "$SOURCE_DIR/config" ] && [ -f "$SOURCE_DIR/config/rouser.toml" ]; then
      cp "$SOURCE_DIR/config/rouser.toml" "${RPMBUILD_DIR}/SOURCES/${SOURCE_NAME}/config/"
    elif [ -f "$PROJECT_ROOT/etc/rouser/config.toml.example" ]; then
      cp "$PROJECT_ROOT/etc/rouser/config.toml.example" "${RPMBUILD_DIR}/SOURCES/${SOURCE_NAME}/config/rouser.toml"
    fi
    if ls "$SOURCE_DIR/systemd/"*.service 1>/dev/null 2>&1; then
      cp "$SOURCE_DIR/systemd/"*.service "${RPMBUILD_DIR}/SOURCES/${SOURCE_NAME}/systemd/" 2>/dev/null || true
    elif ls "$PROJECT_ROOT/systemd/"*.service 1>/dev/null 2>&1; then
      cp "$PROJECT_ROOT/systemd/"*.service "${RPMBUILD_DIR}/SOURCES/${SOURCE_NAME}/systemd/"
    fi
    
    tar czf "${RPMBUILD_DIR}/SOURCES/rouser-source.tar.gz" "${SOURCE_NAME}"

    # SPEC file (written to SPECS dir)  
    cat > "${RPMBUILD_DIR}/SPECS/.rpm-spec" <<'SPECEOF'
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
     
     sed -i "s|@VERSION@|${RPM_VERSION}|g" "${RPMBUILD_DIR}/SPECS/.rpm-spec"
     
     # Run rpmbuild with proper topdir and spec path  
     $RPMBUILD \
       --define "_topdir ${RPMBUILD_DIR}" \
       --target "$ARCH" \
       -bb "${RPMBUILD_DIR}/SPECS/.rpm-spec" 2>&1

     # Copy built RPMs to workspace  
     if ls "${RPMBUILD_DIR}/RPMS/$ARCH/"*.rpm 1>/dev/null 2>&1; then
       cp "${RPMBUILD_DIR}/RPMS/$ARCH/"*.rpm . 2>/dev/null || true
     fi
     
     rm -rf "$SOURCE_NAME"
     echo "Built RPM for $ARCH (version $RPM_VERSION)"

    ;;





  *)
    echo "Usage: $0 build <source-dir> <version> <target-arch>" >&2
    exit 1
    ;;
esac
