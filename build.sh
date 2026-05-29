#!/usr/bin/env bash
set -euo pipefail

APP_BUNDLE="${APP_BUNDLE:-ScreenRecorder.app}"
RESOURCES_DIR="$APP_BUNDLE/Contents/Resources"
FFMPEG_PATH="$RESOURCES_DIR/ffmpeg"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This build script currently supports macOS only." >&2
  exit 1
fi

ARCH="$(uname -m)"
if [[ "$ARCH" != "arm64" && "$ARCH" != "x86_64" ]]; then
  echo "Unsupported macOS architecture: $ARCH" >&2
  exit 1
fi

mkdir -p "$RESOURCES_DIR"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

# ARM64 版本
FFMPEG_URL="https://github.com/eugeneware/ffmpeg-static/releases/download/b6.0/ffmpeg-darwin-arm64"
ARCHIVE="$TMP_DIR/ffmpeg"

echo "Downloading ffmpeg for macOS ARM64..."
if ! curl -fL --connect-timeout 30 --max-time 300 "$FFMPEG_URL" -o "$ARCHIVE"; then
  echo "Error: Failed to download ffmpeg" >&2
  exit 1
fi

cp "$ARCHIVE" "$FFMPEG_PATH"
chmod +x "$FFMPEG_PATH"

echo "Bundled ffmpeg at $FFMPEG_PATH"
echo "Size: $(du -h "$FFMPEG_PATH" | cut -f1)"
