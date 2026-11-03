//! Feature `mouse`: `MouseCapture` and the pointer half of `Input`.
use crate::input::Input;

/// Resource: whether the mouse is captured (default false). Insert `MouseCapture(true)` before `app.run(..)` to get
/// FPS-style `mouse_delta` (hidden, confined, recentered pointer). Changing it while running is not supported.
#[derive(Clone, Copy)]
pub struct MouseCapture(pub bool);

/// Pointer state sampled once per frame by the run loop.
#[derive(Clone, Copy, Default)]
pub(crate) struct Mouse {
    pub delta: [f32; 2],
    pub cursor: Option<[f32; 2]>, // window pixels, y down
    #[cfg(feature = "cam2d")]
    pub size: [f32; 2], // window size in pixels, for `cursor_world`
    pub down: bool,               // left button held
    pub clicked: bool,            // left button went down this frame
}

impl Input {
    /// Mouse motion since the last frame (x right, y down): raw device units on Linux with XInput2, else pixels.
    pub fn mouse_delta(&self) -> [f32; 2] {
        self.mouse.delta
    }

    /// Window size in pixels as of this frame (needs `cam2d`): for pixel-exact zoom and camera snapping.
    #[cfg(feature = "cam2d")]
    pub fn window_size(&self) -> [f32; 2] {
        self.mouse.size
    }

    /// Pointer in window pixels (x right, y down), or `None` while the window is unfocused or the mouse is captured.
    pub fn cursor(&self) -> Option<[f32; 2]> {
        self.mouse.cursor
    }

    /// The left button is held (wherever the pointer is).
    pub fn mouse_down(&self) -> bool {
        self.mouse.down
    }

    /// The left button went down this frame. Sampled once per frame, so a press and release inside one frame is missed.
    pub fn clicked(&self) -> bool {
        self.mouse.clicked
    }
}
