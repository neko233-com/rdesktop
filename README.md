# rdesktop

> Dual-engine Rust desktop framework. WebView by default, Chrome Embedded for pixel-perfect consistency. Built for the AI agent era.

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange)](https://www.rust-lang.org/)

---

## 为什么又写一个桌面框架？

### Tauri v2 没有解决的问题

Tauri 是一个优秀的框架，但在实际使用中存在几个无法回避的问题：

1. **跨平台渲染不一致**：Windows 用 WebView2 (Edge)，macOS 用 WKWebView (WebKit)，Linux 用 WebKitGTK。三个不同的渲染引擎，同一个 CSS 在三个平台上表现不同。对于需要像素级一致的产品，这是致命的。

2. **Windows 打包体验差**：NSIS/WiX 需要手动安装配置，没有内置的 EXE 直接输出。Tauri 的 bundler 经常因为环境问题失败。

3. **WebView2 运行时依赖**：Windows 上需要用户预装 WebView2 运行时（虽然 Win10+ 大多已预装，但企业环境不一定有）。

### opencode 为什么从 Tauri v2 切回 Electron

[opencode](https://github.com/nicedoc/opencode) 的开发者在使用 Tauri v2 后切回了 Electron，核心原因是：

- 用户反馈不同平台上 UI 渲染有差异
- 某些 CSS/JS 特性在不同 WebView 中行为不同
- 调试跨平台渲染问题耗费大量开发时间
- 最终结论：**对于终端用户产品，渲染一致性比包体积更重要**

### rdesktop 的定位

**rdesktop 不是 Tauri 的替代品，而是补充。**

| 场景 | 推荐方案 |
|---|---|
| 内部工具、原型、对像素不敏感 | **Tauri**（最轻量） |
| 需要跨平台像素一致、终端用户产品 | **rdesktop**（双引擎可选） |
| 已有 Electron 生态、需要最大兼容性 | **Electron**（最成熟） |

rdesktop 的核心价值：**让你在需要时可以切换到 Chrome 渲染，而不需要重写整个应用。**

---

## 核心特性

### 1. 双引擎渲染器

```toml
# rdesktop.toml
[renderer]
kind = "webview"   # 默认：轻量，~5MB
# kind = "chrome"  # 可选：像素一致，~150MB
```

同一个 `Renderer` trait，两种实现。应用代码完全不变：

```rust
// 无论用哪个引擎，代码完全一样
renderer.load_url("https://my-app.com")?;
renderer.eval_script("document.title")?;
renderer.send_to_frontend("Hello from Rust")?;
```

### 2. Agent 优先开发

rdesktop 的独特设计：**开发阶段用浏览器，生产阶段用原生窗口。**

为什么？因为 AI Agent（Claude、GPT 等）有成熟的浏览器自动化能力（Playwright/Puppeteer MCP），但几乎没有原生桌面控制能力。

```
开发阶段：rdesktop dev → 浏览器打开 → Agent 用 Playwright 调试
生产阶段：rdesktop build → 原生窗口 → 用户使用
```

Agent 可以：
- 直接查询 DOM（不需要截图 + 视觉模型）
- 精确执行操作（CSS 选择器，不是像素坐标）
- 获取结构化状态（JSON，不是模糊的截图）
- 通过 HTTP API 自动化测试

### 3. 内置打包器

Tauri 需要手动安装 NSIS/WiX，rdesktop 内置：

| 平台 | 输出格式 | 命令 |
|---|---|---|
| Windows | .exe (NSIS), .msi (WiX), 免安装 EXE | `rdesktop bundle` |
| macOS | .app, .dmg | `rdesktop bundle` |
| Linux | AppImage, .deb, .rpm | `rdesktop bundle` |

### 4. 直接 EXE 输出

```bash
# 不需要安装 NSIS/WiX，直接输出可执行文件
rdesktop bundle --target portable
# → target/release/bundle/windows/MyApp.exe
```

---

## Quick Start

```bash
# 安装 CLI
cargo install rdesktop-cli

# 创建项目
rdesktop init my-app
cd my-app

# 开发模式（浏览器）
rdesktop dev

# 构建原生版本
rdesktop build

# 打包为安装程序
rdesktop bundle
```

---

## Agent 开发工作流

这是 rdesktop 的核心差异化能力：

### 1. 启动开发服务器

```bash
rdesktop dev
# → http://localhost:1420
```

### 2. Agent 通过 MCP 工具交互

```python
# Agent 使用 Playwright MCP
page.goto("http://localhost:1420")
page.click("button#submit")
page.fill("input#name", "World")

# 通过 Agent API 获取结构化数据
import requests
dom = requests.get("http://localhost:1420/__rdesktop__/agent/dom").json()
state = requests.get("http://localhost:1420/__rdesktop__/agent/state").json()
```

### 3. Agent API 端点

```
GET  /__rdesktop__/agent/dom          # 完整 DOM 快照（JSON）
GET  /__rdesktop__/agent/elements     # 按 CSS 选择器查询元素
POST /__rdesktop__/agent/action       # 执行 UI 操作（click/type/scroll）
GET  /__rdesktop__/agent/state        # 应用状态快照
POST /__rdesktop__/agent/ipc          # 向 Rust 后端发送 IPC 消息
GET  /__rdesktop__/agent/screenshot   # 截图（委托给 Playwright）
```

### 4. 从浏览器模式无缝切换到原生

```bash
# 开发时用浏览器
rdesktop dev

# 测试原生版本
rdesktop build
./target/release/my-app  # 同样的代码，原生窗口
```

---

## 架构

```
rdesktop/
├── crates/
│   ├── rdesktop-core/      # 核心抽象（Renderer trait、IPC、Config）
│   ├── rdesktop-webview/   # WebView 后端（WebView2/WebKit/WebKitGTK）
│   ├── rdesktop-cef/       # Chrome Embedded 后端（CEF）
│   ├── rdesktop-dev/       # 开发服务器（浏览器模式 + Agent API）
│   ├── rdesktop-bundler/   # 跨平台打包器
│   └── rdesktop-cli/       # CLI 工具
└── examples/
    └── hello_world/
```

### Renderer Trait

```rust
pub trait Renderer {
    fn init(&mut self) -> Result<()>;
    fn create_window(&mut self, config: &WindowConfig) -> Result<WindowHandle>;
    fn load_url(&self, window: WindowHandle, url: &str) -> Result<()>;
    fn load_html(&self, window: WindowHandle, html: &str) -> Result<()>;
    fn eval_script(&self, window: WindowHandle, script: &str) -> Result<()>;
    fn set_ipc_handler(&mut self, handler: Box<dyn IpcHandler>);
    fn send_to_frontend(&self, window: WindowHandle, message: &str) -> Result<()>;
    fn run(self: Box<Self>) -> Result<()>;
}
```

### IPC 通信

```javascript
// 前端 → 后端
const result = await window.__RDESKTOP_INVOKE__('greet', { name: 'World' });
```

```rust
// 后端处理
let handler = FnIpcHandler::new(|msg: IpcMessage| {
    match msg.cmd.as_str() {
        "greet" => IpcResponse {
            id: msg.id,
            success: true,
            data: json!({ "message": format!("Hello, {}!", msg.payload["name"]) }),
        },
        _ => IpcResponse::error(msg.id, "Unknown command"),
    }
});
```

---

## 平台支持

| 平台 | WebView | Chrome | 状态 |
|---|---|---|---|
| Windows 10/11 | WebView2 (Edge) | CEF | ✅ |
| macOS 10.15+ | WKWebView | CEF | ✅ |
| Linux (X11/Wayland) | WebKitGTK | CEF | ✅ |

> 移动端（iOS/Android）不在范围内，rdesktop 专注于桌面平台。

---

## 与 Tauri 的关系

rdesktop 大量借鉴了 Tauri 的设计（wry、tao、IPC 模式），在此基础上增加了：

1. Chrome Embedded 作为可选渲染器
2. 浏览器模式开发（Agent 优先）
3. 内置打包器（不需要外部工具）
4. 直接 EXE 输出

如果你的项目用 Tauri 很满意，不需要切换。rdesktop 适合那些**需要跨平台渲染一致性**或**需要 AI Agent 参与开发**的场景。

---

## License

MIT OR Apache-2.0
