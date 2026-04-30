#!/usr/bin/env bash
# Generate a PKGBUILD for rouser from release assets.
# Usage: ./pkgbuild.sh generate <output-pkgbuild> [version]
set -euo pipefail

ACTION="${1:-help}"
OUTPUT_FILE="${2:-PKGBUILD}"
VERSION="${3:-0.1.0}"
OWNER="owaindjones"
REPO="rouser"

# Strip leading 'v' if present
RPM_VERSION=$(echo "$VERSION" | sed 's/^v//')

case "$ACTION" in
  generate)
    TMPDIR_PKG=$(mktemp -d)
    trap 'rm -rf "$TMPDIR_PKG"' EXIT

    # Download release tarballs and compute SHA256 checksums for integrity verification.
    # This prevents supply chain attacks where GitHub releases are tampered with.
    x86_64_url="https://github.com/${OWNER}/${REPO}/releases/download/v${RPM_VERSION}/rouser-v${RPM_VERSION}-linux-x86_64.tar.gz"
    aarch64_url="https://github.com/${OWNER}/${REPO}/releases/download/v${RPM_VERSION}/rouser-v${RPM_VERSION}-linux-aarch64.tar.gz"

    x86_64_hash=$(curl -fsSL "$x86_64_url" | sha256sum | cut -d' ' -f1)
    aarch64_hash=$(curl -fsSL "$aarch64_url" | sha256sum | cut -d' ' -f1)

    cat > "$OUTPUT_FILE" <<EOF
# Maintainer: Owain Jones <contact@odj.me>
pkgname=rouser
_pkgver='${RPM_VERSION}'
pkgver=\${_pkgver}
pkgrel=1
arch=('x86_64' 'aarch64')
url="https://github.com/${OWNER}/${REPO}"
license=('MIT')
depends=('systemd' 'dbus-libs')
makedepends=()

source_x86_64=(
  "rouser-\${pkgver}-linux-x86_64.tar.gz::https://github.com/${OWNER}/${REPO}/releases/download/v\${pkgver}/rouser-v\${pkgver}-linux-x86_64.tar.gz"
)
sha256sums_x86_64=("${x86_64_hash}")

source_aarch64=(
  "rouser-\${pkgver}-linux-aarch64.tar.gz::https://github.com/${OWNER}/${REPO}/releases/download/v\${pkgver}/rouser-v\${pkgver}-linux-aarch64.tar.gz"
)
sha256sums_aarch64=("${aarch64_hash}")

pkgdesc="A Linux daemon that monitors system metrics and inhibits sleep when activity thresholds are exceeded."

prepare() {
  cd "\$srcdir"
}

package_x86_64() {
  mkdir -p "\${pkgdir}/usr/local/bin"
  install -m 0755 rouser        "\${pkgdir}/usr/local/bin/rouser"
  mkdir -p   "\${pkgdir}/etc/rouser"
  [ -f config.toml ] && cp    config.toml     "\${pkgdir}/etc/rouser/config.toml" || true
}

package_aarch64() {
  mkdir -p "\${pkgdir}/usr/local/bin"
  install -m 0755 rouser        "\${pkgdir}/usr/local/bin/rouser"
  mkdir -p   "\${pkgdir}/etc/rouser"
  [ -f config.toml ] && cp    config.toml     "\${pkgdir}/etc/rouser/config.toml" || true
}
EOF

    echo "Generated $OUTPUT_FILE (version: ${RPM_VERSION})"
    ;;

  *)
    echo "Usage: $0 generate <output-file> [version]" >&2
    exit 1
    ;;
esac
