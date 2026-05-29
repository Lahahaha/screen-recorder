# Screen Recorder

English | [中文](./README.md)

A lightweight screen recording tool that automatically captures screenshots and generates time-lapse videos. Built with Rust for minimal resource usage.

## Features

- 🖥️ **System Tray** - Runs quietly in the menu bar/system tray
- ⏱️ **Auto Capture** - Timed screenshots (10s/30s/60s/120s intervals)
- 🎬 **Video Generation** - Creates H.265 time-lapse videos
- 🔔 **Notifications** - macOS system notifications for video status
- 📁 **Auto Organization** - Screenshots organized by date
- 🖥️ **Cross Platform** - macOS and Windows support

## Download

Download the latest version from [Releases](https://github.com/Lahahaha/screen-recorder/releases):

| Platform | File | Size |
|----------|------|------|
| macOS (Apple Silicon) | `ScreenRecorder-macos-arm64.zip` | ~50MB |
| Windows (x64) | `ScreenRecorder-windows-x64.zip` | ~120MB |

## Usage

### macOS

1. Download and unzip `ScreenRecorder-macos-arm64.zip`
2. Double-click `ScreenRecorder.app` to run
3. Grant screen recording permission when prompted (System Settings → Privacy & Security → Screen Recording)
4. Find the app icon in the menu bar

### Windows

1. Download and unzip `ScreenRecorder-windows-x64.zip`
2. Double-click `screen-recorder.exe` to run
3. Find the app icon in the system tray

### Menu Options

| Option | Description |
|--------|-------------|
| 📷 Capture Now | Take an immediate screenshot |
| ▶ Start | Start automatic timed capture |
| ⏸ Pause | Pause automatic capture |
| ⏱ Interval | Set capture interval (10s/30s/60s/120s) |
| 🎬 Generate Video | Create today's time-lapse video |
| ❌ Exit | Quit the application |

### Storage Location

Screenshots and videos are saved to:

| Platform | Path |
|----------|------|
| macOS | `~/Movies/ScreenRecorder/` |
| Windows | `C:\Users\<username>\Videos\ScreenRecorder\` |

```
ScreenRecorder/
├── config.json           # Configuration
├── screenshots/
│   └── 2026-05-29/
│       ├── 09-00-00.png
│       └── 09-00-30.png
└── videos/
    └── 2026-05-29.mp4
```

### Configuration

Edit `config.json` to customize settings:

```json
{
  "interval": 30,
  "fps": 10,
  "image_format": "png"
}
```

| Setting | Default | Description |
|---------|---------|-------------|
| `interval` | 30 | Capture interval in seconds |
| `fps` | 10 | Video frame rate |
| `image_format` | "png" | Screenshot format (png/jpg) |

## Build from Source

### Prerequisites

- [Rust](https://rustup.rs/) (latest stable)
- [FFmpeg](https://ffmpeg.org/) (for video generation)

### macOS

```bash
# Clone repository
git clone https://github.com/Lahahaha/screen-recorder.git
cd screen-recorder

# Build
cargo build --release

# Create app bundle
mkdir -p ScreenRecorder.app/Contents/MacOS
mkdir -p ScreenRecorder.app/Contents/Resources
cp target/release/screen-recorder ScreenRecorder.app/Contents/MacOS/
cp Info.plist ScreenRecorder.app/Contents/

# Download ffmpeg
curl -L "https://github.com/eugeneware/ffmpeg-static/releases/download/b6.0/ffmpeg-darwin-arm64" -o ScreenRecorder.app/Contents/Resources/ffmpeg
chmod +x ScreenRecorder.app/Contents/Resources/ffmpeg

# Run
open ScreenRecorder.app
```

### Windows

```powershell
# Clone repository
git clone https://github.com/Lahahaha/screen-recorder.git
cd screen-recorder

# Build
cargo build --release

# Download ffmpeg
Invoke-WebRequest -Uri "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip" -OutFile ffmpeg.zip
Expand-Archive -Path ffmpeg.zip -DestinationPath ffmpeg-extract
mkdir ScreenRecorder
cp target/release/screen-recorder.exe ScreenRecorder/
cp ffmpeg-extract/*/bin/ffmpeg.exe ScreenRecorder/

# Run
.\ScreenRecorder\screen-recorder.exe
```

## Auto Release

Pushing a tag automatically builds and releases:

```bash
git tag v0.1.0
git push origin v0.1.0
```

## License

[MIT License](LICENSE)

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.
