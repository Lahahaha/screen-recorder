# Screen Recorder — MVP 方案

## 项目概述

macOS 定时截屏工具，后台运行，记录每天电脑使用情况，可生成视频回顾。基于 Rust 开发，系统托盘交互，体积小、资源占用低。

## 技术栈

- 语言: Rust
- 系统托盘: tray-icon + muda
- 截屏: screenshots crate
- 图片处理: image crate
- 异步运行时: tokio
- 视频合成: 外部调用 ffmpeg
- 序列化: serde + serde_json
- 时间处理: chrono
- 目录: dirs crate

## 目录结构

```
~/Movies/ScreenRecorder/
├── config.json                    # 配置文件
├── screenshots/
│   └── 2026-05-29/
│       ├── 09-00-00.png
│       └── 09-00-30.png
└── videos/
    └── 2026-05-29.mp4
```

## 配置文件 (config.json)

```json
{
  "interval": 30,
  "fps": 10,
  "image_format": "png",
  "scale": 1.0,
  "dedup": false,
  "auto_start": false
}
```

MVP 实现: interval, fps, image_format
预留字段: scale, dedup, auto_start (读取但忽略)

## 系统托盘菜单

```
📷 截一张                  → 立即截一张屏
──────────────────────────
▶ 开始                     → 启动定时截屏
⏸ 暂停                     → 暂停定时截屏
──────────────────────────
⏱ 间隔                     → 当前：30s
   ├─ 10s
   ├─ 30s
   ├─ 60s
   └─ 120s
──────────────────────────
🎬 生成今日视频             → 调用 ffmpeg 合成
──────────────────────────
❌ 退出
```

## 核心架构

主线程: 系统托盘 + 事件循环 (tray-icon + muda)
后台线程: 定时截屏循环
共享状态: Arc<AtomicBool> running + Arc<Mutex<Config>> config

后台线程逻辑:
```
loop {
    if running && 到达截屏时间点:
        screenshots::capture()
        保存到 screenshots/YYYY-MM-DD/HH-MM-SS.png
    sleep(1s)
}
```

## 截图文件命名

HH-MM-SS.png，如 09-00-00.png, 14-30-15.png
文件名天然排序即为时间顺序。

## 视频生成流程

1. 获取今天日期 YYYY-MM-DD
2. 扫描 screenshots/YYYY-MM-DD/ 下所有 png
3. 按文件名排序
4. 生成临时文件列表 filelist.txt (ffmpeg concat 格式)
5. 调用 ffmpeg:
   ffmpeg -y -framerate {fps} -f concat -safe 0 -i filelist.txt -c:v libx264 -pix_fmt yuv420p videos/YYYY-MM-DD.mp4
6. 清理临时文件

## 关键实现要求

1. 截屏使用 screenshots crate 的 Screen::capture() 或 Screen::capture_area()
2. 保存截图时按配置的 image_format 选择 png 或 jpg
3. 配置文件首次运行时自动创建，默认值如上
4. 托盘菜单的间隔选项需要实时更新勾选状态
5. "开始"和"暂停"菜单项文字需要根据状态切换
6. 生成视频时需要处理今天还没有截图的情况
7. macOS 需要屏幕录制权限才能截屏（用户手动授权）
8. 程序退出时保存当前配置到 config.json

## 依赖

```toml
[dependencies]
screenshots = "0.8"
tray-icon = "0.14"
muda = "0.12"
image = "0.25"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = "0.4"
dirs = "5"
```

## MVP 功能清单

- [x] 系统托盘 + 菜单
- [x] 开始/暂停定时截屏
- [x] 手动截一张
- [x] 设置截屏间隔 (10s/30s/60s/120s)
- [x] 截图按日期存储
- [x] 生成今日视频 (ffmpeg)
- [x] 配置文件读写
