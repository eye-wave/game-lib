//! Platform-independent key ids, one `u8` constant per key: `platform::input::KeyW`, `ArrowLeft`, `Escape`, ...
//! The table below is the only place native codes live: (X11 keycode, Windows virtual-key; 0 = none).
#![allow(non_upper_case_globals)]

macro_rules! keys {
    (@consts $i:expr; $name:ident $($rest:ident)*) => {
        pub const $name: u8 = $i;
        keys!(@consts $i + 1; $($rest)*);
    };
    (@consts $i:expr;) => {};
    ($($name:ident $linux:literal $win:literal,)*) => {
        keys!(@consts 0u8; $($name)*);
        /// Indexed by key id.
        const NATIVE: &[(u8, u8)] = &[$(($linux, $win)),*];
    };
}

keys! {
    KeyA 38 0x41, KeyB 56 0x42, KeyC 54 0x43, KeyD 40 0x44, KeyE 26 0x45, KeyF 41 0x46,
    KeyG 42 0x47, KeyH 43 0x48, KeyI 31 0x49, KeyJ 44 0x4A, KeyK 45 0x4B, KeyL 46 0x4C,
    KeyM 58 0x4D, KeyN 57 0x4E, KeyO 32 0x4F, KeyP 33 0x50, KeyQ 24 0x51, KeyR 27 0x52,
    KeyS 39 0x53, KeyT 28 0x54, KeyU 30 0x55, KeyV 55 0x56, KeyW 25 0x57, KeyX 53 0x58,
    KeyY 29 0x59, KeyZ 52 0x5A,
    Digit0 19 0x30, Digit1 10 0x31, Digit2 11 0x32, Digit3 12 0x33, Digit4 13 0x34,
    Digit5 14 0x35, Digit6 15 0x36, Digit7 16 0x37, Digit8 17 0x38, Digit9 18 0x39,
    F1 67 0x70, F2 68 0x71, F3 69 0x72, F4 70 0x73, F5 71 0x74, F6 72 0x75,
    F7 73 0x76, F8 74 0x77, F9 75 0x78, F10 76 0x79, F11 95 0x7A, F12 96 0x7B,
    ArrowUp 111 0x26, ArrowDown 116 0x28, ArrowLeft 113 0x25, ArrowRight 114 0x27,
    Escape 9 0x1B, Space 65 0x20, Enter 36 0x0D, Tab 23 0x09, Backspace 22 0x08,
    Delete 119 0x2E, Insert 118 0x2D, Home 110 0x24, End 115 0x23, PageUp 112 0x21, PageDown 117 0x22,
    ShiftLeft 50 0xA0, ShiftRight 62 0xA1, ControlLeft 37 0xA2, ControlRight 105 0xA3,
    AltLeft 64 0xA4, AltRight 108 0xA5, CapsLock 66 0x14,
    Minus 20 0xBD, Equal 21 0xBB, BracketLeft 34 0xDB, BracketRight 35 0xDD, Backslash 51 0xDC,
    Semicolon 47 0xBA, Quote 48 0xDE, Comma 59 0xBC, Period 60 0xBE, Slash 61 0xBF, Backquote 49 0xC0,
    Numpad0 90 0x60, Numpad1 87 0x61, Numpad2 88 0x62, Numpad3 89 0x63, Numpad4 83 0x64,
    Numpad5 84 0x65, Numpad6 85 0x66, Numpad7 79 0x67, Numpad8 80 0x68, Numpad9 81 0x69,
    NumpadMultiply 63 0x6A, NumpadAdd 86 0x6B, NumpadSubtract 82 0x6D, NumpadDecimal 91 0x6E,
    NumpadDivide 106 0x6F, NumpadEnter 104 0,
}

const _: () = assert!(NATIVE.len() <= 128); // `Keys` holds 128 bits

fn find(n: u8, col: impl Fn(&(u8, u8)) -> u8) -> Option<u8> {
    NATIVE
        .iter()
        .position(|k| col(k) == n && n != 0)
        .map(|i| i as u8)
}

/// X11 keycode -> key id.
pub fn from_linux(code: u8) -> Option<u8> {
    find(code, |k| k.0)
}

/// Windows virtual-key -> key id (the platform layer resolves left/right and numpad variants first).
pub fn from_windows(vk: u8) -> Option<u8> {
    find(vk, |k| k.1)
}

/// Which keys are held. `poll` keeps one up to date.
#[derive(Clone, Copy, Default)]
pub struct Keys([u64; 2]);

impl Keys {
    pub fn set(&mut self, key: u8, down: bool) {
        let (w, b) = (key as usize / 64, key % 64);
        if down {
            self.0[w] |= 1 << b;
        } else {
            self.0[w] &= !(1 << b);
        }
    }

    pub fn get(&self, key: u8) -> bool {
        self.0[key as usize / 64] >> (key % 64) & 1 != 0
    }
}

/// Everything `poll` keeps up to date: held keys and the cursor in window pixels (origin top-left, y down).
#[derive(Clone, Copy, Default)]
pub struct State {
    pub keys: Keys,
    pub mouse: [i32; 2],
}
