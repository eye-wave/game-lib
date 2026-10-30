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

/// Key ids are below this.
pub const MAX_KEYS: usize = 128;
const _: () = assert!(NATIVE.len() <= MAX_KEYS); // `Bits` holds 128 bits

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

/// A set of key ids.
#[derive(Clone, Copy, Default)]
pub struct Bits([u64; 2]);

impl Bits {
    pub fn set(&mut self, key: u8, on: bool) {
        let (w, b) = (key as usize / 64, key % 64);
        if on {
            self.0[w] |= 1 << b;
        } else {
            self.0[w] &= !(1 << b);
        }
    }

    pub fn get(&self, key: u8) -> bool {
        self.0[key as usize / 64] >> (key % 64) & 1 != 0
    }
}

/// Raw keyboard state, kept up to date by `poll`: which keys are held, and how many times each key changed state
/// since `take_changes`. The counts are what make a tap shorter than a frame visible.
#[derive(Clone, Copy)]
pub struct Keys {
    held: Bits,
    changes: [u8; MAX_KEYS],
}

impl Default for Keys {
    fn default() -> Self {
        Keys {
            held: Bits::default(),
            changes: [0; MAX_KEYS],
        }
    }
}

impl Keys {
    pub fn set(&mut self, key: u8, down: bool) {
        if self.held.get(key) != down {
            self.held.set(key, down);
            let c = &mut self.changes[key as usize];
            *c = c.saturating_add(1);
        }
    }

    pub fn get(&self, key: u8) -> bool {
        self.held.get(key)
    }

    pub fn held(&self) -> Bits {
        self.held
    }

    /// How many times each key went up or down since the last call (a tap inside one frame is 2); resets the counts.
    pub fn take_changes(&mut self) -> [u8; MAX_KEYS] {
        core::mem::replace(&mut self.changes, [0; MAX_KEYS])
    }
}

/// What one reader sees: the held keys, and which keys went down / up since that reader last looked.
/// A key can be both pressed and released (a tap, or a release and press again).
#[derive(Clone, Copy, Default)]
pub struct View {
    held: Bits,
    pressed: Bits,
    released: Bits,
}

impl View {
    /// A reader that looks once per frame: `held` is the state now, `changes` is `Keys::take_changes` for the frame.
    pub fn frame(held: Bits, changes: &[u8; MAX_KEYS]) -> View {
        let mut v = View {
            held,
            ..View::default()
        };
        for k in 0..MAX_KEYS as u8 {
            let c = changes[k as usize];
            if c != 0 {
                let before = held.get(k) ^ (c & 1 == 1); // an odd count means the state flipped over the frame
                v.pressed.set(k, !before || c >= 2);
                v.released.set(k, before || c >= 2);
            }
        }
        v
    }

    pub fn is_holding(&self, key: u8) -> bool {
        self.held.get(key)
    }
    pub fn just_pressed(&self, key: u8) -> bool {
        self.pressed.get(key)
    }
    pub fn just_released(&self, key: u8) -> bool {
        self.released.get(key)
    }
    /// +1 while any of `plus` is held, -1 while any of `minus` is, 0 for both or neither.
    pub fn axis(&self, plus: &[u8], minus: &[u8]) -> f32 {
        let any = |ks: &[u8]| ks.iter().any(|&k| self.is_holding(k));
        any(plus) as i8 as f32 - any(minus) as i8 as f32
    }
}

/// Key changes waiting for a reader that looks at its own pace (fixed steps). Each `step` takes at most one change
/// per key, so nothing is lost when several happen between steps and nothing is seen twice when steps outnumber frames.
#[derive(Clone, Copy)]
pub struct Pending([u8; MAX_KEYS]);

impl Default for Pending {
    fn default() -> Self {
        Pending([0; MAX_KEYS])
    }
}

impl Pending {
    pub fn add(&mut self, changes: &[u8; MAX_KEYS]) {
        for (p, c) in self.0.iter_mut().zip(changes) {
            *p = p.saturating_add(*c);
        }
    }

    /// Move `view` forward one step.
    pub fn step(&mut self, view: &mut View) {
        (view.pressed, view.released) = (Bits::default(), Bits::default());
        for k in 0..MAX_KEYS as u8 {
            let p = &mut self.0[k as usize];
            if *p != 0 {
                *p -= 1;
                let down = !view.held.get(k);
                view.held.set(k, down);
                if down {
                    view.pressed.set(k, true);
                } else {
                    view.released.set(k, true);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tap(keys: &mut Keys, k: u8) {
        keys.set(k, true);
        keys.set(k, false);
    }

    #[test]
    fn frame_view_sees_taps_shorter_than_a_frame() {
        let mut keys = Keys::default();
        tap(&mut keys, KeyA);
        let v = View::frame(keys.held(), &keys.take_changes());
        assert!(v.just_pressed(KeyA) && v.just_released(KeyA) && !v.is_holding(KeyA));
        let v = View::frame(keys.held(), &keys.take_changes()); // next frame: nothing new
        assert!(!v.just_pressed(KeyA) && !v.just_released(KeyA));
    }

    #[test]
    fn frame_view_press_hold_release() {
        let mut keys = Keys::default();
        keys.set(KeyA, true);
        let v = View::frame(keys.held(), &keys.take_changes());
        assert!(v.just_pressed(KeyA) && v.is_holding(KeyA) && !v.just_released(KeyA));
        let v = View::frame(keys.held(), &keys.take_changes()); // still held, no change
        assert!(v.is_holding(KeyA) && !v.just_pressed(KeyA));
        keys.set(KeyA, false);
        let v = View::frame(keys.held(), &keys.take_changes());
        assert!(v.just_released(KeyA) && !v.is_holding(KeyA) && !v.just_pressed(KeyA));
    }

    #[test]
    fn fixed_steps_see_each_change_once() {
        let (mut keys, mut pend, mut view) = (Keys::default(), Pending::default(), View::default());
        keys.set(KeyA, true);
        pend.add(&keys.take_changes());
        pend.step(&mut view); // first step after the press
        assert!(view.just_pressed(KeyA) && view.is_holding(KeyA));
        pend.step(&mut view); // a second step in the same frame: no double press
        assert!(!view.just_pressed(KeyA) && view.is_holding(KeyA));
        pend.add(&keys.take_changes()); // a frame with no fixed step in between loses nothing
        keys.set(KeyA, false);
        pend.add(&keys.take_changes());
        pend.step(&mut view);
        assert!(view.just_released(KeyA) && !view.is_holding(KeyA));
    }

    #[test]
    fn fixed_steps_spread_a_tap_over_two_steps() {
        let (mut keys, mut pend, mut view) = (Keys::default(), Pending::default(), View::default());
        tap(&mut keys, KeyA);
        pend.add(&keys.take_changes());
        pend.step(&mut view);
        assert!(view.just_pressed(KeyA) && view.is_holding(KeyA)); // the tap is visible for one step
        pend.step(&mut view);
        assert!(view.just_released(KeyA) && !view.is_holding(KeyA));
        pend.step(&mut view);
        assert!(!view.just_released(KeyA));
    }
}
