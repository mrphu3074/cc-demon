#!/bin/bash
# CC-Demon installer script
# Downloads and installs the pre-built demon binary

set -euo pipefail

VERSION="${1:-latest}"
INSTALL_DIR="${HOME}/.local/bin"
REPO="phunguyen/cc-demon"

# Detect platform
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$ARCH" in
    x86_64|amd64) ARCH="x86_64" ;;
    aarch64|arm64) ARCH="aarch64" ;;
    *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
esac

case "$OS" in
    linux)  TARGET="${ARCH}-unknown-linux-gnu" ;;
    darwin) TARGET="${ARCH}-apple-darwin" ;;
    *)      echo "Unsupported OS: $OS"; exit 1 ;;
esac

BINARY_NAME="demon-${TARGET}"

echo "CC-Demon Installer"
echo "==================="
echo "OS:      $OS"
echo "Arch:    $ARCH"
echo "Target:  $TARGET"
echo "Install: $INSTALL_DIR"
echo ""

# Create install directory
mkdir -p "$INSTALL_DIR"

if [ "$VERSION" = "latest" ]; then
    DOWNLOAD_URL="https://github.com/${REPO}/releases/latest/download/${BINARY_NAME}"
else
    DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${VERSION}/${BINARY_NAME}"
fi

echo "Downloading from: $DOWNLOAD_URL"

if command -v curl &> /dev/null; then
    curl -fsSL "$DOWNLOAD_URL" -o "${INSTALL_DIR}/demon"
elif command -v wget &> /dev/null; then
    wget -q "$DOWNLOAD_URL" -O "${INSTALL_DIR}/demon"
else
    echo "Error: curl or wget required"
    exit 1
fi

chmod +x "${INSTALL_DIR}/demon"

echo ""
echo "Installed demon to: ${INSTALL_DIR}/demon"
echo ""

# Check if install dir is in PATH
if ! echo "$PATH" | grep -q "$INSTALL_DIR"; then
    echo "WARNING: $INSTALL_DIR is not in your PATH"
    echo "Add this to your shell profile:"
    echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
    echo ""
fi

# Initialize config directory
mkdir -p "${HOME}/.demon"

echo "Run 'demon status' to verify installation"
echo "Run '/demon:config' in Claude Code to configure"
