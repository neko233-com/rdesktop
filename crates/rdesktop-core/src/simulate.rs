//! System-wide input simulation — programmatically inject keyboard and mouse
//! events into the OS input stream, as if typed or clicked by a physical user.
//!
//! This is the "output" counterpart to [`crate::input`] (capture). Together
//! they enable device-remapping / macro / automation scenarios — the
//! "keyboard driver / mouse driver" side of a Logitech G-Hub-style tool:
//! observe raw input via [`crate::input::GlobalInput`], then re-emit (or
//! transform) it via [`InputSimulator`].
//!
//! - **Windows**: `SendInput` with `KEYBDINPUT` / `MOUSEINPUT` structures.
//! - **macOS**: a `CGEvent`-based implementation (pending real macOS build;
//!   the current target returns [`crate::error::RdesktopError::UnsupportedPlatform`]).

use crate::error::Result;
#[cfg(not(any(windows, target_os = "macos")))]
use crate::error::RdesktopError;
use crate::hotkeys::Key;
use crate::input::MouseButton;

/// Platform-independent input simulator.
///
/// Construct with [`InputSimulator::new`]; on unsupported platforms it returns
/// [`crate::error::RdesktopError::UnsupportedPlatform`]. The simulator holds no
/// threads and is cheap to keep around — create one and reuse it.
pub struct InputSimulator {
    inner: PlatformSim,
}

enum PlatformSim {
    #[cfg(windows)]
    Windows(crate::simulate_win::WinSim),
    #[cfg(target_os = "macos")]
    Macos(crate::simulate_mac::MacSim),
    #[cfg(not(any(windows, target_os = "macos")))]
    Unsupported,
}

impl InputSimulator {
    /// Create a platform simulator. Errors on unsupported platforms.
    pub fn new() -> Result<Self> {
        Ok(Self {
            inner: PlatformSim::new()?,
        })
    }

    /// Hold a key down (no auto release).
    pub fn press_key(&self, key: Key) -> Result<()> {
        self.inner.press_key(key)
    }

    /// Release a held key.
    pub fn release_key(&self, key: Key) -> Result<()> {
        self.inner.release_key(key)
    }

    /// Press then immediately release a key.
    pub fn tap_key(&self, key: Key) -> Result<()> {
        self.press_key(key)?;
        self.release_key(key)
    }

    /// Press every key in `keys`, then release them in reverse order. Use this
    /// for chords such as `Ctrl+C`: pass `[Key::Letter('c')]` after holding Ctrl
    /// via [`InputSimulator::press_key`], or build the whole chord from raw
    /// keys and let this method do press/release ordering.
    pub fn tap_combo(&self, keys: &[Key]) -> Result<()> {
        self.inner.tap_combo(keys)
    }

    /// Type arbitrary text as if entered on the keyboard. Uses Unicode input
    /// events, so it works for any character (including non-ASCII).
    pub fn type_text(&self, text: &str) -> Result<()> {
        self.inner.type_text(text)
    }

    /// Move the mouse. When `absolute` is true, `(x, y)` are screen coordinates;
    /// otherwise they are relative mickeys (pixels).
    pub fn move_mouse(&self, x: i32, y: i32, absolute: bool) -> Result<()> {
        self.inner.move_mouse(x, y, absolute)
    }

    /// Hold a mouse button down.
    pub fn press_mouse(&self, button: MouseButton) -> Result<()> {
        self.inner.press_mouse(button)
    }

    /// Release a held mouse button.
    pub fn release_mouse(&self, button: MouseButton) -> Result<()> {
        self.inner.release_mouse(button)
    }

    /// Click (press + release) a mouse button at the current position.
    pub fn click(&self, button: MouseButton) -> Result<()> {
        self.press_mouse(button)?;
        self.release_mouse(button)
    }

    /// Scroll the wheel by `delta` units (positive = away from the user).
    pub fn scroll(&self, delta: i32) -> Result<()> {
        self.inner.scroll(delta)
    }
}

impl PlatformSim {
    fn new() -> Result<Self> {
        #[cfg(windows)]
        {
            Ok(PlatformSim::Windows(crate::simulate_win::WinSim::new()?))
        }
        #[cfg(target_os = "macos")]
        {
            Ok(PlatformSim::Macos(crate::simulate_mac::MacSim::new()?))
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            Err(RdesktopError::UnsupportedPlatform(
                "system-wide input simulation is only supported on Windows and macOS".into(),
            ))
        }
    }

    fn press_key(&self, key: Key) -> Result<()> {
        match self {
            #[cfg(windows)]
            PlatformSim::Windows(w) => w.press_key(key),
            #[cfg(target_os = "macos")]
            PlatformSim::Macos(m) => m.press_key(key),
            #[cfg(not(any(windows, target_os = "macos")))]
            PlatformSim::Unsupported => Err(RdesktopError::UnsupportedPlatform(
                "system-wide input simulation is only supported on Windows and macOS".into(),
            )),
        }
    }

    fn release_key(&self, key: Key) -> Result<()> {
        match self {
            #[cfg(windows)]
            PlatformSim::Windows(w) => w.release_key(key),
            #[cfg(target_os = "macos")]
            PlatformSim::Macos(m) => m.release_key(key),
            #[cfg(not(any(windows, target_os = "macos")))]
            PlatformSim::Unsupported => Err(RdesktopError::UnsupportedPlatform(
                "system-wide input simulation is only supported on Windows and macOS".into(),
            )),
        }
    }

    fn tap_combo(&self, keys: &[Key]) -> Result<()> {
        match self {
            #[cfg(windows)]
            PlatformSim::Windows(w) => w.tap_combo(keys),
            #[cfg(target_os = "macos")]
            PlatformSim::Macos(m) => m.tap_combo(keys),
            #[cfg(not(any(windows, target_os = "macos")))]
            PlatformSim::Unsupported => Err(RdesktopError::UnsupportedPlatform(
                "system-wide input simulation is only supported on Windows and macOS".into(),
            )),
        }
    }

    fn type_text(&self, text: &str) -> Result<()> {
        match self {
            #[cfg(windows)]
            PlatformSim::Windows(w) => w.type_text(text),
            #[cfg(target_os = "macos")]
            PlatformSim::Macos(m) => m.type_text(text),
            #[cfg(not(any(windows, target_os = "macos")))]
            PlatformSim::Unsupported => Err(RdesktopError::UnsupportedPlatform(
                "system-wide input simulation is only supported on Windows and macOS".into(),
            )),
        }
    }

    fn move_mouse(&self, x: i32, y: i32, absolute: bool) -> Result<()> {
        match self {
            #[cfg(windows)]
            PlatformSim::Windows(w) => w.move_mouse(x, y, absolute),
            #[cfg(target_os = "macos")]
            PlatformSim::Macos(m) => m.move_mouse(x, y, absolute),
            #[cfg(not(any(windows, target_os = "macos")))]
            PlatformSim::Unsupported => Err(RdesktopError::UnsupportedPlatform(
                "system-wide input simulation is only supported on Windows and macOS".into(),
            )),
        }
    }

    fn press_mouse(&self, button: MouseButton) -> Result<()> {
        match self {
            #[cfg(windows)]
            PlatformSim::Windows(w) => w.press_mouse(button),
            #[cfg(target_os = "macos")]
            PlatformSim::Macos(m) => m.press_mouse(button),
            #[cfg(not(any(windows, target_os = "macos")))]
            PlatformSim::Unsupported => Err(RdesktopError::UnsupportedPlatform(
                "system-wide input simulation is only supported on Windows and macOS".into(),
            )),
        }
    }

    fn release_mouse(&self, button: MouseButton) -> Result<()> {
        match self {
            #[cfg(windows)]
            PlatformSim::Windows(w) => w.release_mouse(button),
            #[cfg(target_os = "macos")]
            PlatformSim::Macos(m) => m.release_mouse(button),
            #[cfg(not(any(windows, target_os = "macos")))]
            PlatformSim::Unsupported => Err(RdesktopError::UnsupportedPlatform(
                "system-wide input simulation is only supported on Windows and macOS".into(),
            )),
        }
    }

    fn scroll(&self, delta: i32) -> Result<()> {
        match self {
            #[cfg(windows)]
            PlatformSim::Windows(w) => w.scroll(delta),
            #[cfg(target_os = "macos")]
            PlatformSim::Macos(m) => m.scroll(delta),
            #[cfg(not(any(windows, target_os = "macos")))]
            PlatformSim::Unsupported => Err(RdesktopError::UnsupportedPlatform(
                "system-wide input simulation is only supported on Windows and macOS".into(),
            )),
        }
    }
}
