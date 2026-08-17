//! rdesktop-dev: Agent-first development server.
//!
//! This crate provides a browser-based development mode for rdesktop applications.
//! Instead of opening a native window, it serves the frontend as a web page that
//! AI agents can interact with using mature browser automation tools.
//!
//! ## Why Browser Mode for Development?
//!
//! AI agents (Claude, GPT, etc.) have excellent browser automation capabilities
//! via Playwright/Puppeteer MCP tools, but very limited native desktop control.
//! By serving the app in a browser during development, agents can:
//!
//! - Inspect the DOM directly (no screenshots needed)
//! - Query elements by CSS selector or text content
//! - Execute actions (click, type, scroll) with precise targeting
//! - Take snapshots of the full application state
//! - Run automated tests against the UI
//!
//! ## Agent API Endpoints
//!
//! When `agent_mode` is enabled, these endpoints are available:
//!
//! - `GET /__rdesktop__/agent/dom` - Full DOM snapshot as JSON
//! - `GET /__rdesktop__/agent/elements?selector=...` - Query elements
//! - `POST /__rdesktop__/agent/action` - Execute UI actions
//! - `GET /__rdesktop__/agent/state` - Application state snapshot
//! - `POST /__rdesktop__/agent/ipc` - Send IPC message from agent
//! - `GET /__rdesktop__/agent/screenshot` - Capture current view

pub mod server;
pub mod agent_api;

pub use server::DevServer;
