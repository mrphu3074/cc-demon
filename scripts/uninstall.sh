#!/bin/bash
# CC-Demon uninstaller

set -euo pipefail

INSTALL_DIR="${HOME}/.local/bin"
DEMON_DIR="${HOME}/.demon"

echo "CC-Demon Uninstaller"
echo "====================="

# Stop daemon if running
if command -v demon &> /dev/null; then
    if demon status 2>/dev/null | grep -q "running"; then
        echo "Stopping daemon..."
        demon stop 2>/dev/null || true
    fi

    # Uninstall service
    echo "Removing system service..."
    demon uninstall 2>/dev/null || true
fi

# Remove binary
if [ -f "${INSTALL_DIR}/demon" ]; then
    rm "${INSTALL_DIR}/demon"
    echo "Removed: ${INSTALL_DIR}/demon"
fi

# Ask about data
echo ""
read -p "Remove all demon data (~/.demon)? [y/N] " -n 1 -r
echo ""
if [[ $REPLY =~ ^[Yy]$ ]]; then
    rm -rf "$DEMON_DIR"
    echo "Removed: $DEMON_DIR"
else
    echo "Kept: $DEMON_DIR"
fi

echo ""
echo "CC-Demon uninstalled"
