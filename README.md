# rdesktop

> A dual-engine Rust desktop framework. WebView by default, Chrome Embedded for pixel-perfect cross-platform consistency.

> 双引擎 Rust 桌面框架。默认使用系统 WebView，可选 Chrome Embedded 实现跨平台像素级一致。

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange)](https://www.rust-lang.org/)

---

## Why rdesktop? / 为什么选择 rdesktop?

### The Problem / 问题

Tauri is great, but has real pain points:

Tauri 很好用，但存在一些实际痛点：

1. **WebView inconsistency / WebView 渲染不一致**: WebView2 (Windows), WebKit (macOS), WebKitGTK (Linux) render differently
2. **Windows bundling complexity / Windows 打包复杂**: NSIS/WiX setup is manual and error-prone
3. **No direct EXE output / 不支持直接输出 EXE**: Always needs an installer or complex configuration

Some teams (like [opencode](https://github.com/nicedoc/opencode)) switched from Tauri v2 back to Electron because users reported visual differences across platforms.

一些团队（如 opencode）从 Tauri v2 切回 Electron，因为用户反馈跨平台渲染不一致。

### The Solution / 解决方案

**Make the renderer a choice, not a constraint:**

**让渲染器成为可选项，而非约束：**

| Mode / 模式 | Size / 大小 | Use Case / 适用场景 |
|---|---|---|
| **WebView** (default) | ~5MB | Internal tools, apps where pixel-perfect isn't critical / 内部工具，像素级一致不重要的应用 |
| **Chrome** (optional) | ~150MB | Cross-platform pixel-perfect rendering / 跨平台像素级一致渲染 |

### Comparison / 对比

| | Tauri | Electron | **rdesktop** |
|---|---|---|---|
| Renderer / 渲染器 | System WebView only | Chromium only | **Both** (switchable) |
| Bundle size / 包体积 | ~5MB | ~150MB | **5MB** or **150MB** |
| Cross-platform pixels / 跨平台像素 | Inconsistent | Consistent | **Consistent** (chrome mode) |
| Windows EXE / 直接 EXE | Complex | Simple | **Simple** |
| Agent-first dev / Agent 优先开发 | No | No | **Yes** |

---

## Agent-First Development / Agent 优先开发

rdesktop is designed for the AI agent era. During development, the app runs in a browser, so AI agents can use mature browser automation tools (Playwright, Puppeteer MCP) to inspect and interact with your app.

rdesktop 为 AI Agent 时代而设计。开发阶段，应用运行在浏览器中，AI Agent 可以使用成熟的浏览器自动化工具（Playwright、Puppeteer MCP）来检查和交互。

### Why Browser Mode? / 为什么用浏览器模式？

AI agents have excellent browser automation via MCP tools, but very limited native desktop control. By serving the app in a browser during development:

AI Agent 通过 MCP 工具拥有出色的浏览器自动化能力，但原生桌面控制能力有限。开发阶段将应用运行在浏览器中：

- **Inspect DOM directly / 直接检查 DOM**: No screenshots needed, query elements by CSS selector / 无需截图，直接用 CSS 选择器查询元素
- **Execute actions / 执行操作**: Click, type, scroll with precise targeting / 精确点击、输入、滚动
- **State snapshots / 状态快照**: Full DOM + application state in JSON / 完整 DOM + 应用状态的 JSON
- **Standard HTTP API / 标准 HTTP API**: Can be scripted and tested / 可以脚本化和测试

### Agent API Endpoints / Agent API 端点

```
GET  /__rdesktop__/agent/dom          # Full DOM snapshot / 完整 DOM 快照
GET  /__rdesktop__/agent/elements     # Query elements / 查询元素
POST /__rdesktop__/agent/action       # Execute UI action / 执行 UI 操作
GET  /__rdesktop__/agent/state        # App state snapshot / 应用状态快照
POST /__rdesktop__/agent/ipc          # Send IPC to backend / 发送 IPC 到后端
GET  /__rdesktop__/agent/screenshot   # Capture view / 捕获视图
```

### Agent Workflow / Agent 工作流

```
1. Agent runs: rdesktop dev
2. App starts at http://localhost:1420
3. Agent uses Playwright MCP to:
   - Navigate to the app
   - Query DOM elements
   - Execute actions (click, type)
   - Verify results via state snapshots
4. No native window needed!
```

---

## Quick Start / 快速开始

```bash
# Create a new project / 创建新项目
rdesktop init my-app

# Or with Chrome renderer / 或使用 Chrome 渲染器
rdesktop init my-app --chrome

# Run in development mode (browser) / 开发模式（浏览器）
cd my-app
rdesktop dev

# Build for release / 构建发布版
rdesktop build

# Bundle into installer / 打包为安装程序
rdesktop bundle
```

---

## Architecture / 架构

```
rdesktop/
├── crates/
│   ├── rdesktop-core/      # Core abstractions / 核心抽象
│   ├── rdesktop-webview/   # WebView backend / WebView 后端
│   ├── rdesktop-cef/       # Chrome Embedded backend / Chrome Embedded 后端
│   ├── rdesktop-dev/       # Dev server (browser mode) / 开发服务器（浏览器模式）
│   ├── rdesktop-bundler/   # Cross-platform bundler / 跨平台打包器
│   └── rdesktop-cli/       # CLI tool / CLI 工具
└── examples/
    └── hello_world/        # Example app / 示例应用
```

### Renderer Trait / 渲染器 Trait

Both backends implement the same trait:

两个后端实现相同的 trait：

```rust
pub trait Renderer {
    fn init(&mut self) -> Result<()>;
    fn create_window(&mut self, config: &WindowConfig) -> Result<WindowHandle>;
    fn load_url(&self, window: WindowHandle, url: &str) -> Result<()>;
    fn load_html(&self, window: WindowHandle, html: &str) -> Result<()>;
    fn eval_script(&self, window: WindowHandle, script: &str) -> Result<()>;
    fn set_ipc_handler(&mut self, handler: Box<dyn IpcHandler>);
    fn run(self: Box<Self>) -> Result<()>;
    // ... more methods
}
```

### IPC / 进程间通信

Frontend (JavaScript) <-> Backend (Rust) communication:

前端（JavaScript）与后端（Rust）通信：

```javascript
// Frontend / 前端
const result = await invoke('greet', { name: 'World' });
console.log(result.message); // "Hello, World!"
```

```rust
// Backend / 后端
let handler = FnIpcHandler::new(|msg: IpcMessage| {
    IpcResponse {
        id: msg.id,
        success: true,
        data: json!({ "message": format!("Hello, {}!", msg.payload["name"]) }),
    }
});
```

---

## Bundler / 打包器

rdesktop includes a built-in bundler for all platforms:

rdesktop 内置跨平台打包器：

### Windows
- **NSIS installer** (.exe) - Traditional Windows installer / 传统 Windows 安装程序
- **WiX MSI** (.msi) - Microsoft Installer format / 微软安装程序格式
- **Portable EXE** - Single executable, no install / 单文件可执行，免安装

### macOS
- **.app bundle** - Standard macOS application / 标准 macOS 应用
- **DMG** - Disk image with drag-to-install / 拖拽安装的磁盘映像

### Linux
- **AppImage** - Universal Linux package / 通用 Linux 包
- **.deb** - Debian/Ubuntu package / Debian/Ubuntu 包
- **.rpm** - Fedora/RHEL package / Fedora/RHEL 包

```bash
# Bundle for current platform / 为当前平台打包
rdesktop bundle

# Bundle specific format / 指定格式打包
rdesktop bundle --target nsis
rdesktop bundle --target portable
```

---

## Configuration / 配置

`rdesktop.toml`:

```toml
[app]
identifier = "com.example.myapp"
name = "My App"
version = "1.0.0"

[renderer]
kind = "webview"  # or "chrome" / 或 "chrome"

[window]
title = "My App"
width = 1280
height = 720

[dev]
port = 1420           # Dev server port / 开发服务器端口
agent_mode = true     # Enable Agent API / 启用 Agent API
hot_reload = true     # Enable hot reload / 启用热重载

[bundle]
windows_installer = "nsis"
linux_packages = ["appimage", "deb"]
```

---

## Platform Support / 平台支持

| Platform / 平台 | WebView | Chrome | Status / 状态 |
|---|---|---|---|
| Windows 10/11 | WebView2 (Edge) | CEF | Supported / 支持 |
| macOS 10.15+ | WKWebView | CEF | Supported / 支持 |
| Linux (X11/Wayland) | WebKitGTK | CEF | Supported / 支持 |

**Note**: Mobile (iOS/Android) is not in scope. rdesktop focuses on desktop platforms.

**注意**：移动端（iOS/Android）不在范围内。rdesktop 专注于桌面平台。

---

## License / 许可证

MIT OR Apache-2.0
