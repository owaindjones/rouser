#!/usr/bin/env bash
set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info() { echo -e "${GREEN}[INFO]${NC} $*"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $*"; }
error() { echo -e "${RED}[ERROR]${NC} $*" >&2; }

# Detect CPU architecture
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

# Fetch latest release from GitHub (requires gh CLI or curl with auth)
GITHUB_REPO="${ROUSER_GH_REPO:-yourusername/rouser}"
TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT

info "Fetching latest rouser release..."

# Try to get the latest artifact from GitHub Actions artifacts
# First, try using gh CLI for authenticated access (needed for artifacts)
if command -v gh &>/dev/null; then
    DOWNLOAD_URL="$(gh api repos/"$GITHUB_REPO"/actions/artifacts/latest/rouser-"${ARCH}"-linux --jq '.archive_download_url' 2>/dev/null)" || true
fi

# Fallback: try public releases page (for tagged releases)
if [ -z "$DOWNLOAD_URL" ] && command -v curl &>/dev/null; then
    LATEST_RELEASE=$(curl -sL "https://api.github.com/repos/${GITHUB_REPO}/releases/latest" 2>/dev/null | grep '"tag_name"' | head -1 | sed 's/.*"v//;s/".*//' || true)
fi

# If we have a release tag, construct the download URL
if [ -n "${LATEST_RELEASE:-}" ]; then
    DOWNLOAD_URL="https://github.com/${GITHUB_REPO}/releases/download/v${LATEST_RELEASE}/rouser-v${LATEST_RELEASE}-linux-${ARCH}.tar.gz"
fi

# If we got a direct artifact download URL, use it
if [ -n "${DOWNLOAD_URL:-}" ]; then
    info "Downloading from: ${DOWNLOAD_URL}"
    curl -fSL --max-time 120 -o "$TEMP_DIR/rouser.tar.gz" "$DOWNLOAD_URL"
else
    error "Could not find latest release. Ensure the repository is set (ROUSER_GH_REPO env var) and releases/artifacts exist."
    exit 1
fi

# Extract archive
info "Extracting..."
mkdir -p "$TEMP_DIR/extracted"
tar xzf "$TEMP_DIR/rouser.tar.gz" -C "$TEMP_DIR/extracted/"

# Copy binary to ~/.local/bin
BIN_TARGET="$HOME/.local/bin/rouser"
info "Installing rouser binary to ${BIN_TARGET}..."
mkdir -p "$(dirname "$BIN_TARGET")"
cp "$TEMP_DIR/extracted/rouser" "$BIN_TARGET"
chmod +x "$BIN_TARGET"

# Copy default config
CONFIG_DEST="$HOME/.config/rouser/config.toml"
info "Installing default config to ${CONFIG_DEST}..."
mkdir -p "$(dirname "$CONFIG_DEST")"
if [ -f "$TEMP_DIR/extracted/config.toml" ]; then
    cp "$TEMP_DIR/extracted/config.toml" "$CONFIG_DEST"
elif [ -f "$TEMP_DIR/rouser/config.toml" ]; then
    cp "$TEMP_DIR/rouser/config.toml" "$CONFIG_DEST"
else
    warn "No config file found in archive, creating minimal default..."
    cat > "$CONFIG_DEST" << 'EOF'
name = "rouser"
update_interval = "5s"
log_level = "info"

[metrics.cpu]
threshold = 80.0
ema_alpha = 0.3

[metrics.gpu]
threshold = 90.0
ema_alpha = 0.3

[metrics.network]
threshold = 100.0
ema_alpha = 0.2
exclude_interfaces = ["lo"]
include_interfaces = []

[metrics.disk]
threshold = 50.0
ema_alpha = 0.2
exclude_device_prefixes = ["loop", "fd", "sr", "cdrom"]

[timing]
duration_threshold = "30s"
cooldown_duration = "60s"

[inhibitor]
what = "sleep"
mode = "block"
EOF
fi

# Install systemd service file
SERVICE_DEST="$HOME/.config/systemd/user/rouser.service"
info "Installing systemd service to ${SERVICE_DEST}..."
mkdir -p "$(dirname "$SERVICE_DEST")"
if [ -f "$TEMP_DIR/extracted/rouser.service" ]; then
    cp "$TEMP_DIR/extracted/rouser.service" "$SERVICE_DEST"
elif [ -f "$TEMP_DIR/systemd/rouser.service" ]; then
    cp "$TEMP_DIR/systemd/rouser.service" "$SERVICE_DEST"
fi

# Enable and start the service
info "Enabling rouser systemd user service..."
systemctl --user daemon-reload
systemctl --user enable --now rouser.service || warn "Failed to enable/start service (is logind lingering enabled?)"

echo ""
info "rouser installed successfully!"
echo ""
echo "Next steps:"
echo "  1. Review your config: ${CONFIG_DEST}"
echo "  2. Test with dry-run: rouser --config ${CONFIG_DEST} --dry-run"
echo "  3. Check status: systemctl --user status rouser"
echo ""

# Guide user on logind lingering if service failed to start
if ! loginctl show-session "$(loginctl | grep "$(whoami)" | awk '{print $1}' | head -1)" -p IdleActionSec &>/dev/null 2>&1; then
    warn "Logind may not have lingering enabled for user '${USER}'."
    echo ""
    echo "If rouser doesn't run when you're logged out, enable lingering:"
    echo "  sudo loginctl enable-linger ${USER}"
    echo ""
fi

# Also check if systemd-user@.service is active
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
