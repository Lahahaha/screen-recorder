# Screen Recorder

[English](./README.en.md) | 中文

轻量级屏幕记录工具，定时保存截图，并将截图生成延时视频。使用 Rust 构建，主要通过系统托盘运行，适合个人日常记录工作状态、学习过程或长时间任务进度。

## 功能特性

- 🖥️ **系统托盘运行**：在 macOS 菜单栏或 Windows 系统托盘中静默运行。
- ⏱️ **定时截屏**：支持 10s、30s、60s、120s 间隔。
- 📷 **手动截屏**：随时通过托盘菜单立即截一张。
- 🎬 **今日视频生成**：一键将当天截图生成延时视频。
- 🗂️ **历史视频窗口**：列出历史截图文件夹，也可以手动导入外部文件夹生成视频。
- 🧩 **多屏视频合成基础**：视频生成器支持按命名规则识别多屏批次并合成为同一帧；当前截屏采集仍为单屏。
- 🌐 **中英文切换**：托盘菜单、tooltip、通知和历史窗口支持中文/英文手动切换。
- 🔔 **用户反馈**：截图、视频生成、失败场景会通过系统通知或状态文本反馈。
- 📁 **自动整理**：截图按日期保存，视频集中保存到 `videos` 目录。
- 🖥️ **跨平台目标**：支持 macOS 和 Windows。Windows 已通过交叉编译检查，仍建议真机验证。

## 下载

从 [Releases](https://github.com/Lahahaha/screen-recorder/releases) 下载最新版本：

| 平台 | 文件 | 说明 |
|------|------|------|
| macOS Apple Silicon | `ScreenRecorder-macos-arm64.zip` | 包含 app bundle 和 ffmpeg |
| Windows x64 | `ScreenRecorder-windows-x64.zip` | 包含 exe 和 ffmpeg.exe |

## 使用方法

### macOS

1. 下载并解压 `ScreenRecorder-macos-arm64.zip`。
2. 双击 `ScreenRecorder.app` 运行。
3. 首次截屏时授权屏幕录制权限：系统设置 -> 隐私与安全性 -> 屏幕录制。
4. 在菜单栏找到应用图标。

### Windows

1. 下载并解压 `ScreenRecorder-windows-x64.zip`。
2. 双击 `screen-recorder.exe` 运行。
3. 在系统托盘找到应用图标。
4. 如果视频生成失败，确认 `ffmpeg.exe` 与程序在同一目录，或已经加入 `PATH`。

### 托盘菜单

| 菜单项 | 说明 |
|--------|------|
| 📷 截一张 | 立即保存一张截图 |
| ▶ 开始 / ⏸ 暂停 | 开始或暂停定时截屏 |
| ⏱ 间隔 | 设置截屏间隔 |
| 🎬 生成今日视频 | 将今天的截图生成视频 |
| 历史视频 | 打开历史视频窗口 |
| 📁 打开保存目录 | 打开截图、视频和配置所在目录 |
| 🌐 语言 / Language | 在中文和英文之间切换 |
| ❌ 退出 | 保存配置并退出程序 |

### 历史视频窗口

历史视频窗口用于从历史截图或外部文件夹重新生成视频。

- 自动列出 `screenshots` 下的日期文件夹。
- 支持添加外部文件夹。
- 支持多选并逐个生成视频。
- 生成过程中显示进度，可以取消当前批量任务。
- 空目录、无图片目录会显示为不可生成。
- 单项生成失败不会阻止后续任务继续执行。

也可以从命令行直接打开：

```bash
screen-recorder --history
```

## 存储位置

默认会优先使用系统视频目录：

| 平台 | 默认路径 |
|------|----------|
| macOS | `~/Movies/ScreenRecorder/` |
| Windows | `C:\Users\<用户名>\Videos\ScreenRecorder\` |

如果系统视频目录不可用，程序会依次尝试文档目录和用户主目录。

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

## 配置

配置文件位于 `config.json`。缺失字段会使用默认值。

```json
{
  "interval": 30,
  "fps": 10,
  "image_format": "png",
  "scale": 1.0,
  "dedup": false,
  "auto_start": false,
  "video_codec": "h264",
  "language": "zh-CN"
}
```

| 字段 | 默认值 | 说明 |
|------|--------|------|
| `interval` | `30` | 截屏间隔，支持 `10`、`30`、`60`、`120` 秒 |
| `fps` | `10` | 生成视频的帧率 |
| `image_format` | `"png"` | 截图格式，支持 `"png"`、`"jpg"` |
| `scale` | `1.0` | 截图缩放比例 |
| `dedup` | `false` | 是否跳过连续重复截图 |
| `auto_start` | `false` | 启动后是否自动开始定时截屏 |
| `video_codec` | `"h264"` | 视频编码，支持 `"h264"`、`"h265"` |
| `language` | `"zh-CN"` | UI 语言，支持 `"zh-CN"`、`"en"` |

## 视频生成规则

- 当前内置截屏采集保存单屏文件，命名为 `HH-MM-SS.mmm-000123.png` 或 `.jpg`。
- 视频生成会忽略非图片文件。
- 外部非标准命名图片也可以作为普通单图帧参与生成。
- 损坏图片会被跳过；如果没有任何可读图片，则生成失败并显示错误。
- 单屏视频保持原图尺寸，必要时将宽高调整为偶数以兼容视频编码。
- 多屏批次文件会按 `HH-MM-SS.mmm-screen-01-000123.png` 这类命名分组，并优先使用 `.screens.json` 中的几何信息合成。
- 多屏合成画布最大限制为 `7680x4320`。

## 从源码构建

### 前置要求

- [Rust](https://rustup.rs/) 最新稳定版
- [FFmpeg](https://ffmpeg.org/) 用于视频生成

### macOS

```bash
git clone https://github.com/Lahahaha/screen-recorder.git
cd screen-recorder

# 构建 release 二进制
cargo build --release

# 或创建 app bundle，并下载/签名 ffmpeg
./build.sh

# 运行 app bundle
open ScreenRecorder.app

# 直接打开历史视频窗口
target/release/screen-recorder --history
```

### Windows

```powershell
git clone https://github.com/Lahahaha/screen-recorder.git
cd screen-recorder

cargo build --release

mkdir ScreenRecorder
copy target\release\screen-recorder.exe ScreenRecorder\

# 将 ffmpeg.exe 放到 ScreenRecorder 目录，或将 ffmpeg 加入 PATH
.\ScreenRecorder\screen-recorder.exe
```

## 开发检查

```bash
cargo fmt --check
cargo test
cargo clippy -- -D warnings
cargo build --release

# 可选：Windows 条件编译检查
rustup target add x86_64-pc-windows-gnu
cargo clippy --target x86_64-pc-windows-gnu -- -D warnings
```

调试视频生成耗时：

```bash
target/release/screen-recorder --profile-video-dir <截图目录> <输出视频.mp4>
```

模拟多屏截图生成：

```bash
cargo run -- --simulate-multiscreen-video
```

## 自动发布

推送 tag 后触发 GitHub Actions 自动构建发布：

```bash
git tag v0.1.0
git push origin v0.1.0
```

## 开源协议

[MIT License](LICENSE)

## 贡献

欢迎提交 issue 或 Pull Request。
