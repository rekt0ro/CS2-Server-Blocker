#!/usr/bin/env bash
set -euo pipefail

echo "Removing CS2 Server Blocker..."

sudo rm -f /usr/local/bin/cs2-server-blocker
sudo rm -f /usr/share/applications/cs2-server-blocker.desktop
sudo rm -f /usr/share/icons/hicolor/scalable/apps/cs2-server-blocker.svg

if command -v update-desktop-database >/dev/null 2>&1; then
    sudo update-desktop-database /usr/share/applications >/dev/null 2>&1 || true
fi

echo "CS2 Server Blocker has been uninstalled."
echo "Your firewall rules and saved state were not removed."
