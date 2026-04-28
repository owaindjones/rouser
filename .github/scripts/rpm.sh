#!/usr/bin/env bash
# Build an .rpm package for rouser from extracted files using rpmbuild.
shopt -s pipefail 2>/dev/null || true
set -euo pipefail

ACTION="${1:-help}"
SOURCE_DIR="${2:-pkg-build}"
VERSION="${3:-0.1.0}"
ARCH="${4:-x86_64}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
RPM_VERSION=$(echo "$VERSION" | sed 's/^v//')
SOURCE_NAME="rouser-${RPM_VERSION}"

RPMTOP="${HOME}/rpmbuild"
mkdir -p "${RPMTOP}/BUILD" "${RPMTOP}/RPMS/${ARCH}" \
         "${RPMTOP}/SOURCES" "${RPMTOP}/SPECS" \
         "${RPMTOP}/SRPMs" "${RPMTOP}/BUILDROOT"

# Assemble all source files under SOURCES/<name>/ for tarball creation
mkdir -p "${RPMTOP}/SOURCES/${SOURCE_NAME}"
cp "${SOURCE_DIR}/rouser"       "${RPMTOP}/SOURCES/${SOURCE_NAME}/rouser" 2>/dev/null || true
if [ -d "$SOURCE_DIR/config" ] && [ -f "$SOURCE_DIR/config/rouser.toml" ]; then
    mkdir -p "${RPMTOP}/SOURCES/${SOURCE_NAME}"
    cp "${SOURCE_DIR}/config/rouser.toml"   "${RPMTOP}/SOURCES/${SOURCE_NAME}/rouser.toml"
fi
if ls "$SOURCE_DIR/systemd/"*.service 1>/dev/null 2>&1; then
    mkdir -p "${RPMTOP}/SOURCES/${SOURCE_NAME}/systemd"
    cp "${SOURCE_DIR}/systemd/"*.service   "${RPMTOP}/SOURCES/${SOURCE_NAME}/systemd/"
elif ls "$PROJECT_ROOT/systemd/"*.service 1>/dev/null 2>&1; then
    mkdir -p "${RPMTOP}/SOURCES/${SOURCE_NAME}/systemd"
    cp "$PROJECT_ROOT/systemd/"*.service    "${RPMTOP}/SOURCES/${SOURCE_NAME}/systemd/"
fi

cd "${RPMTOP}/SOURCES" && tar czf rouser-source.tar.gz "${SOURCE_NAME}" && cd -

cat > "${RPMTOP}/SPECS/.rpm-spec" << SPECEOF
%global debug_package %{nil}
Name:           rouser
Version:        ${RPM_VERSION}
Release:        1%{?dist}
Summary:        System metrics daemon with sleep inhibition
License:        MIT
URL:            https://github.com/owaindjones/rouser
Source0:        %{name}-source.tar.gz


%description
A Linux daemon that monitors CPU, GPU, network, and disk
activity to prevent unwanted system suspend or hibernation
when activity thresholds are exceeded.



%prep
%setup -q

%install
mkdir -p %{buildroot}/usr/bin
cp rouser       %{buildroot}/usr/bin/
mkdir -p        %{buildroot}/etc/rouser
[ -f rouser.toml ] && cp rouser.toml   %{buildroot}/etc/rouser/config.toml || true
if ls systemd/*.service 1>/dev/null 2>&1; then
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
* $(date '+%a %b %d %Y') Release <release@example.com> - ${RPM_VERSION}
- Build for release
SPECEOF

rpmbuild \
    --target "$ARCH" \
    -bb "${RPMTOP}/SPECS/.rpm-spec" 2>&1 || { cat "${RPMTOP}/BUILD/*.log" 2>/dev/null; exit 1; }

for rpm_file in "${RPMTOP}/RPMS/${ARCH}/"*.rpm; do
    [ -f "$rpm_file" ] && cp "$rpm_file" .
done

rm -rf "${SOURCE_NAME}"

for f in rouser-${RPM_VERSION}-*.rpm; do
    if [ -f "$f" ]; then
        new_name=$(echo "$f" | sed "s/rouser-${RPM_VERSION}-[^-]*-\(.*\)\.rpm/rouser-${RPM_VERSION}-\1.rpm/")
        mv -- "$f" "$new_name" 2>/dev/null || true
    fi
done

echo "Built RPM for $ARCH (version $RPM_VERSION)"
