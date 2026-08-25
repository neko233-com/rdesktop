# rdesktop

[English](README.md) | **简体中文**

[![Crates.io](https://img.shields.io/crates/v/rdesktop-cli.svg)](https://crates.io/crates/rdesktop-cli)
[![docs.rs](https://docs.rs/rdesktop-core/badge.svg)](https://docs.rs/rdesktop-core)
[![Rust](https://img.shields.io/badge/rust-2021-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/neko233-com/rdesktop)

> 面向 Agent 的 Rust 桌面应用框架，支持可切换的 WebView 与 Chromium 渲染后端。

rdesktop 面向这样的桌面应用：界面使用 Web 技术构建，但开发、测试、原生运行时都需要保持可观察、可脚本化。项目提供统一的 Rust API，用于窗口、渲染、IPC、输入、全局快捷键，以及开发阶段的视觉验证。

## 项目状态

rdesktop 当前处于活跃开发的 `0.1.x` 阶段。浏览器开发服务器、Agent API、渲染器抽象和视觉调试工作流是当前版本线的重点；原生构建与安装包集成仍在持续完善，正式分发前请针对目标平台单独验证产物。

项目强调明确行为和可组合的 workspace，而不是将所有能力隐藏在单一运行时中。在 `1.0.0` 之前，公开 API 仍可能发生变化。

## 为什么选择 rdesktop？

大多数桌面 Web 框架主要优化两个目标之一：更小的包体，或最大的浏览器兼容性。rdesktop 关注第三个目标：让 Agent 驱动的开发过程始终可检查、可回放、可验证。

- **开发阶段直接使用浏览器。** Agent 可以通过 HTTP 检查 DOM、查询元素、执行结构化操作并读取应用状态。
- **保留原生渲染能力。** 同一个应用模型可以选择系统 WebView，或基于 Chromium 的渲染路径。
- **验证用户真正看到的结果。** 原生截图和 Windows 桌面 MP4 录屏可以让人和 Agent 复核视觉回归与完整交互流程。
- **显式表达平台行为。** 窗口状态、IPC、全局输入、快捷键、悬浮层、壁纸窗口和穿透点击都通过 Rust API 与配置表达。

rdesktop 并不是所有桌面框架的替代品，可以按下面的原则选择：

| 需求 | 可考虑的方案 |
| --- | --- |
| 轻量内部工具、原生集成较少 | Tauri 或系统 WebView 封装 |
| 已有 Electron 应用、需要最大 Web 兼容性 | Electron |
| 需要 Agent 可观测开发、渲染器选择和原生桌面控制 | rdesktop |

## 核心能力

### 可切换渲染器

`rdesktop-core` 定义 workspace 使用的渲染器和窗口抽象：

- `rdesktop-webview` 通过 `wry` 和 `tao` 使用平台 WebView；
- `rdesktop-cef` 通过 Chrome DevTools Protocol（CDP）驱动 Chrome、Chromium 或 Edge，提供一致的 Chromium 渲染路径；
- 应用层 IPC 和窗口操作尽量与具体渲染器解耦。

```toml
[renderer]
kind = "webview" # 轻量系统 WebView
# kind = "chrome" # Chromium/CDP 渲染路径
```

Chromium 路径要求主机上存在兼容的 Chrome、Chromium 或 Edge 可执行文件；它不是内置 CEF 发行包。

### 面向 Agent 的开发服务器

`rdesktop dev` 会在普通浏览器中提供前端页面，并注入 rdesktop bridge。Agent 可以把它当作标准浏览器目标，同时仍然使用应用自己的 IPC 和交互模型。

开发服务器支持：

- DOM 快照和 CSS 选择器元素查询；
- click、type、fill、scroll、hover、focus、select、press、drag 等结构化操作；
- 应用状态快照和 IPC 请求；
- 前端文件变化后的热重载；
- 原生截图发布，以及可选的等待画面更新语义；
- 可被 Playwright、Puppeteer、MCP 工具或普通脚本调用的本地 HTTP API。

### 视觉验证与录屏

每个开发服务器只拥有一个录制会话和一个固定输出文件：

- Windows 下捕获整个虚拟桌面，通过 GDI 与 Media Foundation 编码为真正的 H.264 MP4；
- Windows 路径不需要安装 FFmpeg，也不需要在实际打包中携带 FFmpeg；
- 重复调用 `start` 会复用当前会话，重复调用 `stop` 是安全的；
- 默认最长录制 5 分钟，硬上限为 1 小时；
- 正常 finalize、启动失败和优雅退出时都会清理临时文件；
- 非 Windows 平台使用浏览器 bridge 的 `MediaRecorder` 降级路径，可能需要用户授权，也可能输出 WebM 而不是 MP4。

这种有边界且幂等的设计是有意为之：Agent 即使重试请求，也不会持续产生无上限的调试视频垃圾。

### 原生桌面能力

核心 workspace 包含以下抽象和平台实现：

- 普通窗口、悬浮层、壁纸层、置顶、透明和点击穿透窗口；
- 自定义标题栏拖动与窗口调整大小；
- 原生窗口图标；
- 全局快捷键，以及默认关闭、显式开启的全局输入钩子；
- Rust 与前端之间的结构化 IPC。

## 快速开始

### 安装 CLI

```bash
cargo install rdesktop-cli --version 0.1.7
```

### 创建项目

```bash
rdesktop init hello-rdesktop
cd hello-rdesktop
```

初始化器会生成最小的 `rdesktop.toml`、Rust 入口文件和 `frontend/index.html`。

### 启动开发服务器

```bash
rdesktop dev
```

服务器默认监听 `http://localhost:1420` 并自动打开浏览器。要为 Agent 或 CI 运行无界面流程，可以关闭自动打开浏览器：

```bash
rdesktop dev --no-open
```

常用 CLI 命令：

```text
rdesktop init <name>                 创建项目骨架
rdesktop dev [--path <dir>]         启动浏览器开发服务器
rdesktop build [--chrome]           构建原生应用路径
rdesktop bundle --target <target>   生成平台 bundle
rdesktop info                       查看 rdesktop.toml
rdesktop icons --input <png>        从一张 PNG 生成多尺寸 ICO/PNG 图标
```

### 从 PNG 生成 Windows 图标

开发者只需要准备一张带透明背景的 PNG，即可生成 Windows 快捷方式需要的真实 `.ico`，以及
16/24/32/48/64/128/256 像素的 PNG 变体：

```bash
rdesktop icons \
  --input resources/icons/source.png \
  --output-dir resources/icons \
  --name app
```

输出目录会包含 `app.ico` 和 `app-16.png` 等文件。非正方形源图会被完整缩放到透明正方形画布，
不会被静默裁剪；输入边长超过 8192 像素会被拒绝。GitX 的 Windows 打包流程使用同样的
`.ico` 作为开始菜单和桌面快捷方式图标，并会在打包阶段 fail-closed 校验它存在。

构建和安装包命令仍在持续开发中。正式分发前，请检查命令输出，并在目标平台验证生成的产物。

## 配置

生成的项目使用 `rdesktop.toml`。下面是一个最小开发配置：

```toml
[app]
identifier = "com.example.hello"
name = "hello-rdesktop"
version = "0.1.0"

[renderer]
kind = "webview"
webgpu = true

[dev]
host = "localhost"
port = 1420
open_browser = true
hot_reload = true
agent_mode = true
devtools = true

[window]
title = "Hello rdesktop"
width = 1280
height = 720
resizable = true

[global_input]
enabled = false
keyboard = true
mouse = true
mouse_move = false
```

除非确实需要远程访问，否则请保持 `host = "localhost"`。设置为 `0.0.0.0` 会把开发与 Agent 接口暴露到网络，只应在可信且隔离的网络中使用。

## Agent 工作流

开发服务器默认地址是 `http://localhost:1420`，结构化接口位于 `/__rdesktop__/agent/` 下。

| 方法 | 接口 | 用途 |
| --- | --- | --- |
| `GET` | `/dom` | 读取最新 DOM 快照 |
| `GET` | `/elements?selector=...` | 按选择器、文本或角色查询元素 |
| `POST` | `/action` | 加入结构化 UI 操作队列 |
| `GET` | `/state` | 读取最新应用状态快照 |
| `POST` | `/ipc` | 向前端/后端 bridge 发送 IPC 消息 |
| `GET` | `/screenshot` | 获取截图；`wait=true&after=<generation>` 可等待新的原生画面 |
| `GET` | `/recording` | 获取唯一录制会话状态 |
| `POST` | `/recording/start` | 开始或复用唯一录制 |
| `POST` | `/recording/stop` | 停止并 finalize 录制 |
| `GET` | `/recording/file` | 下载已完成录制 |

示例交互：

```bash
# 查询按钮
curl "http://localhost:1420/__rdesktop__/agent/elements?selector=button"

# 执行操作；原生渲染时等待新的画面
curl -X POST "http://localhost:1420/__rdesktop__/agent/action?wait=true" \
  -H "content-type: application/json" \
  -d '{"action":"click","selector":"button#submit"}'

# 启动一个有上限的录制
curl -X POST "http://localhost:1420/__rdesktop__/agent/recording/start" \
  -H "content-type: application/json" \
  -d '{"fps":30,"max_duration_seconds":300}'

# 执行完整流程后 finalize 同一个录制
curl -X POST "http://localhost:1420/__rdesktop__/agent/recording/stop" \
  -H "content-type: application/json" \
  -d '{}'

# 下载结果
curl -L "http://localhost:1420/__rdesktop__/agent/recording/file" -o recording.mp4
```

稳健的视觉断言建议优先使用结构化 DOM/状态接口，执行操作后等待对应的原生画面，最后再把 MP4 作为可长期查看的审计产物。这样可以让日常 Agent 运行保持快速，同时在需要时保留完整的人类可读轨迹。

## Workspace 结构

```text
rdesktop/
├── crates/
│   ├── rdesktop-core/       共享类型、渲染器 trait、IPC、输入和窗口 API
│   ├── rdesktop-webview/    WebView2/WebKit/WebKitGTK 后端
│   ├── rdesktop-cef/        Chromium/CDP 后端
│   ├── rdesktop-dev/        浏览器开发服务器和 Agent API
│   ├── rdesktop-bundler/    平台 bundle 抽象与生成器
│   ├── rdesktop-assets/     PNG 到 ICO/桌面图标资源生成器
│   └── rdesktop-cli/        `rdesktop` 命令行工具
├── examples/                示例应用
├── test-app/                本地视觉与交互测试夹具
├── ARCHITECTURE.md          设计说明与子系统边界
└── README-CN.md             中文文档
```

## 平台说明

| 平台 | 系统 WebView | Chromium 路径 | 原生 MP4 录制 |
| --- | --- | --- | --- |
| Windows | WebView2 | Chrome/Chromium/Edge | 通过 GDI + Media Foundation 支持 |
| macOS | WKWebView | Chrome/Chromium/Edge | 浏览器降级路径 |
| Linux | WebKitGTK | Chrome/Chromium/Edge | 浏览器降级路径 |

具体运行时要求取决于所选后端。WebView 后端使用操作系统提供的 WebView；Chromium 后端要求主机上存在兼容浏览器。移动端不在当前范围内。

## 从源码构建

```bash
git clone https://github.com/neko233-com/rdesktop.git
cd rdesktop
cargo check --workspace
cargo test --workspace
cargo run -p rdesktop-cli -- --help
```

Windows 原生录屏路径需要在 Windows 上构建和运行，才能编译并实际验证 Media Foundation 实现。

## 参与贡献

欢迎贡献代码。在提交 Pull Request 前：

1. 阅读 [ARCHITECTURE.md](ARCHITECTURE.md)，确认受影响的 crate。
2. 保持公开 API 改动聚焦，并记录 Agent 或用户可以观察到的行为。
3. 运行 `cargo fmt --all -- --check`。
4. 运行 `cargo check --workspace` 和 `cargo test --workspace`。
5. 如果涉及 Agent API 或渲染行为，请提供可复现的请求序列，并说明视觉验证方式。
6. 不要把录屏、构建产物、凭据和本地测试文件提交进仓库。

文档修复同样非常有价值。请使用清晰的提交信息，并在 Pull Request 中说明平台相关假设。

## 安全与隐私

Agent API 是开发接口，不是面向互联网的服务。它可以检查和操控运行中的应用，并且在支持的平台上捕获桌面。默认绑定 localhost，不要在共享网络上暴露；分享录屏前请检查其中是否包含敏感内容。

请不要在 Issue 或 Pull Request 中提交凭据、私有截图或含有敏感信息的录屏。

## 许可证

rdesktop 采用以下任一许可证：

- [Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0)
- [MIT License](https://opensource.org/license/mit/)

由你选择。

## 致谢

rdesktop 构建于 Rust 生态之上，使用了 `wry`、`tao`、`tokio`、`axum`、`chromiumoxide`、`serde` 和 `windows` 等项目。感谢这些项目的维护者与贡献者。
