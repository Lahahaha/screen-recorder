# Screen Recorder

English | [中文](./README.md)

A lightweight screen activity recorder that saves timed screenshots and turns them into time-lapse videos. It is built with Rust, runs primarily from the system tray, and is intended for personal daily recording of work sessions, study sessions, or long-running task progress.

## Features

- 🖥️ **System tray app**: Runs quietly in the macOS menu bar or Windows system tray.
- ⏱️ **Timed screenshots**: Supports 10s, 30s, 60s, and 120s intervals.
- 📷 **Manual capture**: Take an immediate screenshot from the tray menu.
- 🎬 **Today's video**: Generate a time-lapse video from today's screenshots.
- 🗂️ **History video window**: List historical screenshot folders or import external folders to generate videos.
- 🧩 **Multi-screen capture and composition**: Auto-detects screen count, saves multi-screen batches, and generates composed videos.
- 🌐 **Chinese/English UI**: Tray menu, tooltip, notifications, and the history window support manual language switching.
- 🔔 **User feedback**: Capture, video generation, and failure cases are surfaced through notifications or status text.
- 📁 **Auto organization**: Screenshots are grouped by date, and videos are saved under `videos`.
- 🖥️ **Cross-platform target**: macOS and Windows are supported targets. Windows passes cross-compilation checks, but real-device validation is still recommended.

## Download

Download the latest version from [Releases](https://github.com/Lahahaha/screen-recorder/releases):

| Platform | File | Notes |
|----------|------|-------|
| macOS Apple Silicon | `ScreenRecorder-macos-arm64.zip` | Includes the app bundle and ffmpeg |
| Windows x64 | `ScreenRecorder-windows-x64.zip` | Includes the executable and ffmpeg.exe |

## Usage

### macOS

1. Download and unzip `ScreenRecorder-macos-arm64.zip`.
2. Double-click `ScreenRecorder.app`.
3. Grant screen recording permission on first capture: System Settings -> Privacy & Security -> Screen Recording.
4. Find the app icon in the menu bar.

### Windows

1. Download and unzip `ScreenRecorder-windows-x64.zip`.
2. Double-click `screen-recorder.exe`.
3. Find the app icon in the system tray.
4. If video generation fails, make sure `ffmpeg.exe` is next to the app or available in `PATH`.

### Tray Menu

| Menu item | Description |
|-----------|-------------|
| 📷 Capture Now | Save one screenshot immediately |
| ▶ Start / ⏸ Pause | Start or pause timed capture |
| ⏱ Interval | Change the capture interval |
| 🖥️ Capture Source | Use automatic multi-screen capture, or pin capture to one screen |
| 🎬 Generate Today's Video | Generate a video from today's screenshots |
| History Videos | Open the history video window |
| 📁 Open Save Folder | Open the folder containing screenshots, videos, and config |
| 🌐 Language | Switch between Chinese and English |
| ❌ Quit | Save config and quit the app |

### History Video Window

The history video window is used to regenerate videos from historical screenshots or external folders.

- Automatically lists date folders under `screenshots`.
- Supports adding external folders.
- Supports selecting multiple folders and generating videos one by one.
- Generation mode can use multi-screen composition or only one screen such as `screen-01` or `screen-02`.
- Shows progress during generation and lets you cancel the current batch.
- Empty folders and folders without images are shown as unavailable.
- A failed item does not stop the remaining selected items.

You can also open it from the command line:

```bash
screen-recorder --history
```

## Storage Location

The app prefers the system video directory by default:

| Platform | Default path |
|----------|--------------|
| macOS | `~/Movies/ScreenRecorder/` |
| Windows | `C:\Users\<username>\Videos\ScreenRecorder\` |

If the system video directory is unavailable, the app falls back to the documents directory and then the home directory.

```text
ScreenRecorder/
├── app.log
├── app.log.1
├── config.json
├── screenshots/
│   └── 2026-05-30/
│       ├── 09-00-00.123-000000.png
│       └── 09-00-30.456-000001.png
└── videos/
    └── 2026-05-30.mp4
```

## Configuration

Settings are stored in `config.json`. Missing fields use defaults.

```json
{
  "interval": 30,
  "fps": 10,
  "image_format": "png",
  "scale": 1.0,
  "dedup": false,
  "auto_start": false,
  "video_codec": "h264",
  "language": "zh-CN",
  "capture_mode": "auto"
}
```

| Field | Default | Description |
|-------|---------|-------------|
| `interval` | `30` | Capture interval. Supported values: `10`, `30`, `60`, `120` seconds |
| `fps` | `10` | Generated video frame rate |
| `image_format` | `"png"` | Screenshot format: `"png"` or `"jpg"` |
| `scale` | `1.0` | Screenshot scale factor |
| `dedup` | `false` | Skip consecutive duplicate screenshots |
| `auto_start` | `false` | Start timed capture when the app launches |
| `video_codec` | `"h264"` | Video codec: `"h264"` or `"h265"` |
| `language` | `"zh-CN"` | UI language: `"zh-CN"` or `"en"` |
| `capture_mode` | `"auto"` | Capture source: `"auto"`, `"screen-01"`, `"screen-02"`, etc. |

## Video Generation Rules

- In auto mode, single-screen machines keep the legacy file name: `HH-MM-SS.mmm-000123.png` or `.jpg`.
- In auto mode, multi-screen machines save one image per successful screen in the same batch: `HH-MM-SS.mmm-screen-01-000123.png`, `screen-02`, etc., plus `.screens.json`.
- Users can also pin capture to one specific screen from the tray menu. That mode keeps the legacy single-screen file name.
- If one screen fails during a multi-screen batch, successful screens are still saved; metadata records only the successful screens.
- Non-image files are ignored.
- External images with non-standard names can still be used as normal single-image frames.
- Broken images are skipped. If no readable image remains, generation fails with a clear error.
- Single-screen videos keep the source image size, with width/height adjusted to even values when needed for video encoding.
- Multi-screen batch files are grouped by timestamp and sequence, and `.screens.json` geometry metadata is preferred when present.
- The history video window can switch to single-screen generation; single-screen outputs add a `-screen-XX` suffix.
- Multi-screen composition is capped at `7680x4320`.

## Build from Source

### Prerequisites

- [Rust](https://rustup.rs/) latest stable
- [FFmpeg](https://ffmpeg.org/) for video generation

### macOS

```bash
git clone https://github.com/Lahahaha/screen-recorder.git
cd screen-recorder

# Build the release binary
cargo build --release

# Or create an app bundle and download/sign ffmpeg
./build.sh

# Run the app bundle
open ScreenRecorder.app

# Open the history video window directly
target/release/screen-recorder --history
```

### Windows

```powershell
git clone https://github.com/Lahahaha/screen-recorder.git
cd screen-recorder

cargo build --release

mkdir ScreenRecorder
copy target\release\screen-recorder.exe ScreenRecorder\

# Put ffmpeg.exe in the ScreenRecorder folder, or add ffmpeg to PATH
.\ScreenRecorder\screen-recorder.exe
```

## Development Checks

```bash
cargo fmt --check
cargo test
cargo clippy -- -D warnings
cargo build --release

# Optional: Windows conditional compilation check
rustup target add x86_64-pc-windows-gnu
cargo clippy --target x86_64-pc-windows-gnu -- -D warnings
```

Profile video generation:

```bash
target/release/screen-recorder --profile-video-dir <screenshot-dir> <output-video.mp4>
```

Simulate multi-screen screenshots:

```bash
cargo run -- --simulate-multiscreen-video
```

## Auto Release

Pushing a tag triggers GitHub Actions to build and publish a release:

```bash
git tag v0.1.0
git push origin v0.1.0
```

## License

[MIT License](LICENSE)

## Contributing

Issues and pull requests are welcome.
