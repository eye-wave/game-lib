//! `Input`: keyboard (always) and mouse (feature `mouse`, see `mouse.rs`) for `Update` systems.
use core::ops::Deref;
use platform::input::View as PlatformView;

/// Resource: keyboard and mouse for `Update` systems, refreshed once per frame. Keys are `platform::input::*` ids
/// (`KeyW`, `ArrowLeft`, `Space`, ...): `is_holding`, `just_pressed`, `just_released`, `axis`. A tap that starts and
/// ends inside one frame is not lost: it is both `just_pressed` and `just_released`.
/// With `fixed_input`, fixed systems should read `FixedInput` instead: this one only changes once per frame, so a
/// fixed step could see the same press several times, or none.
/// The pointer methods (`mouse_delta`, `cursor`, `cursor_world`, `mouse_down`, `clicked`) need the `mouse` feature.
/// Without `MouseCapture` the pointer is free: `cursor`, `cursor_world`, `mouse_down` and `clicked` work, and
/// `mouse_delta` is 0. With it the mouse is captured (hidden, confined, recentered) while the window has focus,
/// so it only reports motion (`mouse_delta`), like an FPS, and the pointer methods report nothing.
/// Only `Input` has the pointer methods, not `FixedInput`.
#[derive(Clone, Copy, Default)]
pub struct Input {
    pub(crate) view: PlatformView,
    #[cfg(feature = "mouse")]
    pub(crate) mouse: crate::mouse::Mouse,
}
impl Deref for Input {
    type Target = PlatformView;
    fn deref(&self) -> &PlatformView {
        &self.view
    }
}
