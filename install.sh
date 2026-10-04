#!/usr/bin/env bash
set -euo pipefail

REPO="rekt0ro/CS2-Server-Blocker"
ARCH="$(uname -m)"

case "$ARCH" in
    x86_64)
        ASSET="cs2-server-blocker-x86_64-linux.tar.gz"
        ;;
    *)
        echo "Unsupported architecture: $ARCH"
        echo "Currently supported: x86_64"
        exit 1
        ;;
esac

for command in curl tar; do
    if ! command -v "$command" >/dev/null 2>&1; then
        echo "Missing required command: $command"
        exit 1
    fi
done

BASE_URL="https://github.com/${REPO}/releases/latest/download"
DOWNLOAD_URL="${BASE_URL}/${ASSET}"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "Downloading the latest CS2 Server Blocker release..."
curl -fL --retry 3 --retry-delay 1 -o "$TMP_DIR/$ASSET" "$DOWNLOAD_URL"

echo "Extracting package..."
tar -xzf "$TMP_DIR/$ASSET" -C "$TMP_DIR"

test -f "$TMP_DIR/cs2-server-blocker"
test -f "$TMP_DIR/cs2-server-blocker.desktop"
test -f "$TMP_DIR/cs2-server-blocker.svg"

echo "Installing CS2 Server Blocker..."

sudo install -Dm755     "$TMP_DIR/cs2-server-blocker"     /usr/local/bin/cs2-server-blocker

sudo install -Dm644     "$TMP_DIR/cs2-server-blocker.desktop"     /usr/share/applications/cs2-server-blocker.desktop

sudo install -Dm644     "$TMP_DIR/cs2-server-blocker.svg"     /usr/share/icons/hicolor/scalable/apps/cs2-server-blocker.svg

if command -v update-desktop-database >/dev/null 2>&1; then
    sudo update-desktop-database /usr/share/applications >/dev/null 2>&1 || true
fi

echo
echo "CS2 Server Blocker installed successfully."
echo "Launch it from your application menu or run:"
echo "  cs2-server-blocker"

if ! command -v pkexec >/dev/null 2>&1; then
    echo
    echo "Note: pkexec is not installed."
    echo "Firewall actions require root privileges and use pkexec when not run as root."
fi
