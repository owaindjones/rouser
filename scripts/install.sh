#!/usr/bin/env bash
set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info() { echo -e "${GREEN}[INFO]${NC} $*"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $*"; }
error() { echo -e "${RED}[ERROR]${NC} $*" >&2; }

stop_service() { systemctl --user stop rouser.service 2>/dev/null || true; }

FROM_REPO=false
REBUILD=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --help)
            echo "Usage: install.sh [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --from-repo   Use local repo build (cargo build --release from current dir)"
            echo "  --rebuild     Force rebuild even if binary already exists"
            exit 0
            ;;
        --from-repo) FROM_REPO=true; shift ;;
        --rebuild)   REBUILD=true; shift ;;
        *)           error "Unknown option: $1"; exit 1 ;;
    esac
done

get_arch() {
    case "$(uname -m)" in
        x86_64)   echo "x86_64" ;;
        aarch64)  echo "aarch64" ;;
        arm64)    echo "aarch64" ;;
        *)        error "Unsupported architecture: $(uname -m). rouser only supports x86_64 and aarch64."; exit 1 ;;
    esac
}

ARCH="$(get_arch)"
info "Detected architecture: ${ARCH}"

stop_service

BIN_TARGET="${XDG_BIN_HOME:-$HOME/.local/bin}/rouser"
mkdir -p "$(dirname "$BIN_TARGET")"

if [ "$FROM_REPO" = true ]; then
    info "Using local repository build from $(pwd)..."
    BIN_SOURCE="$PWD/target/release/rouser"
    if [ "$REBUILD" = true ] || [ ! -f "$BIN_SOURCE" ]; then
        info "Building rouser release binary..."
        cargo build --release 2>&1 | tail -n 1 || { error "Build failed."; exit 1; }

        if [ ! -f "$BIN_SOURCE" ]; then
            error "Build did not produce $BIN_SOURCE. Aborting."
            exit 1
        fi
    else
        info "Binary already exists at ${BIN_SOURCE}, skipping build (use --rebuild to force rebuild)."
    fi

    cp "$BIN_SOURCE" "$BIN_TARGET"
    chmod +x "$BIN_TARGET"

    SERVICE_DEST="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/rouser.service"
    info "Installing systemd service to ${SERVICE_DEST}..."
    mkdir -p "$(dirname "$SERVICE_DEST")"
    REPO_SERVICE="$PWD/systemd/rouser.service"

    if [ -f "$REPO_SERVICE" ]; then
        cp "$REPO_SERVICE" "$SERVICE_DEST"
    else
        warn "No service file found at ${REPO_SERVICE}, skipping."
    fi
else
    GITHUB_REPO="${ROUSER_GH_REPO:-owaindjones/rouser}"
    TEMP_DIR=$(mktemp -d)
    trap 'rm -rf "$TEMP_DIR"' EXIT

    info "Fetching latest rouser release..."

    LATEST_RELEASE=$(curl -sL "https://api.github.com/repos/${GITHUB_REPO}/releases/latest" 2>/dev/null | grep '"tag_name"' | head -1 | sed 's/.*"v//;s/".*//' || true)

    if [ -z "${LATEST_RELEASE:-}" ]; then
        error "Could not find latest release. Ensure the repository is set (ROUSER_GH_REPO env var)."
        exit 1
    fi

    DOWNLOAD_URL="https://github.com/${GITHUB_REPO}/releases/download/v${LATEST_RELEASE}/rouser-v${LATEST_RELEASE}-linux-${ARCH}.tar.gz"

    if [ -n "${DOWNLOAD_URL:-}" ]; then
        info "Downloading from: ${DOWNLOAD_URL}"
        curl -fSL --max-time 120 -o "$TEMP_DIR/rouser.tar.gz" "$DOWNLOAD_URL"
    else
        error "Could not find latest release. Ensure the repository is set (ROUSER_GH_REPO env var) and releases/artifacts exist."
        exit 1
    fi

    info "Extracting..."
    mkdir -p "$TEMP_DIR/extracted"
    tar xzf "$TEMP_DIR/rouser.tar.gz" -C "$TEMP_DIR/extracted/"

    cp "$TEMP_DIR/extracted/rouser" "$BIN_TARGET"
    chmod +x "$BIN_TARGET"

    SERVICE_DEST="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/rouser.service"
    mkdir -p "$(dirname "$SERVICE_DEST")"
    if [ -f "$TEMP_DIR/extracted/systemd/rouser.service" ]; then
        info "Installing systemd service to ${SERVICE_DEST}..."
        cp "$TEMP_DIR/extracted/systemd/rouser.service" "$SERVICE_DEST"
    else
        warn "No systemd service file found in release archive (expected: extracted/systemd/rouser.service), skipping."
    fi
fi

info "Enabling rouser systemd user service..."
systemctl --user daemon-reload
systemctl --user enable --now rouser.service || warn "Failed to enable/start service (is logind lingering enabled?)"
systemctl --user restart rouser.service || warn "Failed to enable/start service (is logind lingering enabled?)"

echo ""
info "rouser installed successfully!"
echo ""
echo "Next steps:"
echo "  1. Config created at startup: ${XDG_CONFIG_HOME:-$HOME/.config}/rouser/config.toml"
echo "  2. Test with dry-run: rouser --dry-run"
echo "  3. Check status: systemctl --user status rouser"
echo ""

if ! loginctl show-session "$(loginctl | grep "$(whoami)" | awk '{print $1}' | head -1)" -p IdleActionSec &>/dev/null 2>&1; then
    warn "Logind may not have lingering enabled for user '${USER}'."
    echo ""
    echo "If rouser doesn't run when you're logged out, enable lingering:"
    echo "  sudo loginctl enable-linger ${USER}"
    echo ""
fi

if systemctl --user is-active rouser.service &>/dev/null; then
    info "rouser service is running!"
else
    warn "rouser service did not start automatically."
    echo ""
    echo "This usually means logind lingering is not enabled for your user."
    echo "To fix, run: sudo loginctl enable-linger ${USER}"
    echo "Then restart the service: systemctl --user restart rouser"
fi

info "Done!"
