# Screen Recorder

[English](./README.en.md) | 中文

轻量级屏幕录制工具，自动定时截屏并生成延时视频。使用 Rust 构建，资源占用极低。

## 功能特性

- 🖥️ **系统托盘** - 安静运行在菜单栏/托盘区
- ⏱️ **定时截屏** - 自动截屏（10s/30s/60s/120s 间隔）
- 🎬 **视频生成** - 生成 H.265 延时视频
- 🔔 **系统通知** - macOS 系统通知（成功/失败）
- 📁 **自动整理** - 截图按日期自动分类
- 🖥️ **跨平台** - 支持 macOS 和 Windows

## 下载

从 [Releases](https://github.com/Lahahaha/screen-recorder/releases) 下载最新版本：

| 平台 | 文件 | 大小 |
|------|------|------|
| macOS (Apple Silicon) | `ScreenRecorder-macos-arm64.zip` | ~50MB |
| Windows (x64) | `ScreenRecorder-windows-x64.zip` | ~120MB |

## 使用方法

### macOS

1. 下载并解压 `ScreenRecorder-macos-arm64.zip`
2. 双击 `ScreenRecorder.app` 运行
3. 首次运行需授权屏幕录制权限（系统设置 → 隐私与安全性 → 屏幕录制）
4. 在菜单栏找到应用图标

### Windows

1. 下载并解压 `ScreenRecorder-windows-x64.zip`
2. 双击 `screen-recorder.exe` 运行
3. 在系统托盘找到应用图标

### 菜单选项

| 选项 | 说明 |
|------|------|
| 📷 截一张 | 立即截取一张屏幕 |
| ▶ 开始 | 启动定时自动截屏 |
| ⏸ 暂停 | 暂停自动截屏 |
| ⏱ 间隔 | 设置截屏间隔（10s/30s/60s/120s） |
| 🎬 生成今日视频 | 生成今天的延时视频 |
| ❌ 退出 | 退出程序 |

### 存储位置

截图和视频保存在：

| 平台 | 路径 |
|------|------|
| macOS | `~/Movies/ScreenRecorder/` |
| Windows | `C:\Users\<用户名>\Videos\ScreenRecorder\` |

```
ScreenRecorder/
├── config.json           # 配置文件
├── screenshots/
│   └── 2026-05-29/
│       ├── 09-00-00.png
│       └── 09-00-30.png
└── videos/
    └── 2026-05-29.mp4
```

### 配置文件

编辑 `config.json` 自定义设置：

```json
{
  "interval": 30,
  "fps": 10,
  "image_format": "png"
}
```

| 设置项 | 默认值 | 说明 |
|--------|--------|------|
| `interval` | 30 | 截屏间隔（秒） |
| `fps` | 10 | 视频帧率 |
| `image_format` | "png" | 截图格式（png/jpg） |

## 从源码构建

### 前置要求

- [Rust](https://rustup.rs/)（最新稳定版）
- [FFmpeg](https://ffmpeg.org/)（用于视频生成）

### macOS

```bash
# 克隆仓库
git clone https://github.com/Lahahaha/screen-recorder.git
cd screen-recorder

# 构建
cargo build --release

# 创建 app bundle
mkdir -p ScreenRecorder.app/Contents/MacOS
mkdir -p ScreenRecorder.app/Contents/Resources
cp target/release/screen-recorder ScreenRecorder.app/Contents/MacOS/
cp Info.plist ScreenRecorder.app/Contents/

# 下载 ffmpeg
curl -L "https://github.com/eugeneware/ffmpeg-static/releases/download/b6.0/ffmpeg-darwin-arm64" -o ScreenRecorder.app/Contents/Resources/ffmpeg
chmod +x ScreenRecorder.app/Contents/Resources/ffmpeg

# 运行
open ScreenRecorder.app
```

### Windows

```powershell
# 克隆仓库
git clone https://github.com/Lahahaha/screen-recorder.git
cd screen-recorder

# 构建
cargo build --release

# 下载 ffmpeg
Invoke-WebRequest -Uri "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip" -OutFile ffmpeg.zip
Expand-Archive -Path ffmpeg.zip -DestinationPath ffmpeg-extract
mkdir ScreenRecorder
cp target/release/screen-recorder.exe ScreenRecorder/
cp ffmpeg-extract/*/bin/ffmpeg.exe ScreenRecorder/

# 运行
.\ScreenRecorder\screen-recorder.exe
```

## 自动发布

推送 tag 自动构建并发布：

```bash
git tag v0.1.0
git push origin v0.1.0
```

## 开源协议

[MIT License](LICENSE)

## 贡献

欢迎提交 Pull Request！
