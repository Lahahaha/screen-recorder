#!/usr/bin/env bash
set -euo pipefail

APP_NAME="${APP_NAME:-ScreenRecorder}"
APP_BUNDLE="${APP_BUNDLE:-$APP_NAME.app}"
BINARY_NAME="${BINARY_NAME:-screen-recorder}"
APP_VERSION="${APP_VERSION:-0.12.2}"
BINARY_PATH="target/release/$BINARY_NAME"
CONTENTS_DIR="$APP_BUNDLE/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"
PLIST_PATH="$CONTENTS_DIR/Info.plist"
APP_BINARY_PATH="$MACOS_DIR/$BINARY_NAME"
FFMPEG_PATH="$RESOURCES_DIR/ffmpeg"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This build script currently supports macOS only." >&2
  exit 1
fi

ARCH="$(uname -m)"
case "$ARCH" in
  arm64)
    FFMPEG_URL="https://github.com/eugeneware/ffmpeg-static/releases/download/b6.0/ffmpeg-darwin-arm64"
    ;;
  x86_64)
    FFMPEG_URL="https://github.com/eugeneware/ffmpeg-static/releases/download/b6.0/ffmpeg-darwin-x64"
    ;;
  *)
    echo "Unsupported macOS architecture: $ARCH" >&2
    exit 1
    ;;
esac

echo "Building release binary..."
cargo build --release

if [[ ! -x "$BINARY_PATH" ]]; then
  echo "Error: expected release binary at $BINARY_PATH" >&2
  exit 1
fi

echo "Creating app bundle at $APP_BUNDLE..."
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"
cp "$BINARY_PATH" "$APP_BINARY_PATH"
chmod +x "$APP_BINARY_PATH"

if [[ -f "Info.plist" ]]; then
  cp "Info.plist" "$PLIST_PATH"
else
  cat >"$PLIST_PATH" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>$APP_NAME</string>
    <key>CFBundleDisplayName</key>
    <string>$APP_NAME</string>
    <key>CFBundleIdentifier</key>
    <string>com.screen-recorder.app</string>
    <key>CFBundleVersion</key>
    <string>$APP_VERSION</string>
    <key>CFBundleShortVersionString</key>
    <string>$APP_VERSION</string>
    <key>CFBundleExecutable</key>
    <string>$BINARY_NAME</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>LSMinimumSystemVersion</key>
    <string>12.0</string>
    <key>LSUIElement</key>
    <true/>
</dict>
</plist>
PLIST
fi

if [[ -x "$FFMPEG_PATH" ]]; then
  echo "Using existing ffmpeg at $FFMPEG_PATH"
else
  TMP_DIR="$(mktemp -d)"
  trap 'rm -rf "$TMP_DIR"' EXIT

  FFMPEG_DOWNLOAD="$TMP_DIR/ffmpeg"
  echo "Downloading ffmpeg for macOS $ARCH..."
  if ! curl -fL --connect-timeout 30 --max-time 300 "$FFMPEG_URL" -o "$FFMPEG_DOWNLOAD"; then
    echo "Error: Failed to download ffmpeg" >&2
    exit 1
  fi

  cp "$FFMPEG_DOWNLOAD" "$FFMPEG_PATH"
  chmod +x "$FFMPEG_PATH"
fi

echo "Bundled ffmpeg at $FFMPEG_PATH"
echo "ffmpeg size: $(du -h "$FFMPEG_PATH" | cut -f1)"

SIGN_IDENTITY="$(security find-identity -v -p codesigning 2>/dev/null | awk -F '"' '/Apple Development/ { print $2; exit }' || true)"
if [[ -n "$SIGN_IDENTITY" ]]; then
  SIGN_LABEL="$SIGN_IDENTITY"
else
  SIGN_IDENTITY="-"
  SIGN_LABEL="ad-hoc (-)"
fi

echo "Signing app with: $SIGN_LABEL"
codesign --force --sign "$SIGN_IDENTITY" --timestamp=none "$FFMPEG_PATH"
codesign --force --sign "$SIGN_IDENTITY" --timestamp=none "$APP_BINARY_PATH"
codesign --force --deep --sign "$SIGN_IDENTITY" --timestamp=none "$APP_BUNDLE"

echo "Build complete: $APP_BUNDLE"
