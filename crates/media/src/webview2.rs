//! Windows backend: a hidden WebView2 (Edge) view over raw COM, no crates. Needs the WebView2 Runtime (Windows 11 has
//! it) and `WebView2Loader.dll` next to the exe (linked with raw-dylib, so there is no import library to build
//! with). Create and pump it on one thread that has a message loop; `platform`'s event poll is one.
use crate::web::{self, Inbox, State};
use alloc::{boxed::Box, vec::Vec};
use core::ffi::c_void;
use core::ptr::{null, null_mut};
use win32_sys::{CreateWindowExA, DispatchMessageA, Msg, PM_REMOVE, PeekMessageA};

type P = *mut c_void;
type Hr = i32;

/// Only posted messages (which is how COM and WebView2 call back), so keys and mouse stay for the game's own poll.
const PM_QS_POSTMESSAGE: u32 = 0x0098_0000;

#[link(name = "WebView2Loader", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateCoreWebView2EnvironmentWithOptions(
        folder: *const u16,
        user_data: *const u16,
        options: P,
        done: P,
    ) -> Hr;
}
#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: P, flags: u32) -> Hr;
    fn CoTaskMemFree(p: P);
}
#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DestroyWindow(h: isize) -> i32;
}

/// Method `i` of a COM object's vtable (0..=2 are IUnknown). Slots used below, from WebView2.idl:
/// Environment 3 CreateCoreWebView2Controller; Controller 24 Close, 25 get_CoreWebView2;
/// WebView 6 NavigateToString, 33 PostWebMessageAsString, 34/35 add/remove_WebMessageReceived;
/// message args 5 TryGetWebMessageAsString.
unsafe fn vt<F>(obj: P, i: usize) -> F {
    unsafe { core::mem::transmute_copy(&*(*(obj as *const *const usize)).add(i)) }
}

unsafe fn add_ref(obj: P) {
    unsafe { vt::<unsafe extern "system" fn(P) -> u32>(obj, 1)(obj) };
}

unsafe fn release(obj: P) {
    unsafe { vt::<unsafe extern "system" fn(P) -> u32>(obj, 2)(obj) };
}

// One hand-made COM callback object serves all three handler interfaces: each has `Invoke(a, b)` in slot 3, and
// `kind` says which. They live inside `Sys`, so the reference counting is a no-op.
#[repr(C)]
struct Vtbl {
    qi: unsafe extern "system" fn(P, P, *mut P) -> Hr,
    add_ref: unsafe extern "system" fn(P) -> u32,
    release: unsafe extern "system" fn(P) -> u32,
    invoke: unsafe extern "system" fn(P, usize, P) -> Hr,
}

static VT: Vtbl = Vtbl {
    qi,
    add_ref: one,
    release: one,
    invoke,
};

unsafe extern "system" fn qi(this: P, _iid: P, out: *mut P) -> Hr {
    unsafe { *out = this }; // every interface asked for is the one it is
    0
}

unsafe extern "system" fn one(_: P) -> u32 {
    1
}

#[repr(C)]
struct Handler {
    vtbl: &'static Vtbl,
    sys: *mut Sys,
    kind: u8, // 0 environment created, 1 controller created, 2 message received
}

struct Sys {
    inbox: *mut Inbox,
    hwnd: isize,
    env: P,
    ctrl: P,
    wv: P,
    token: i64,
    h: [Handler; 3],
}

pub struct View {
    pub inbox: Box<Inbox>, // boxed: the callbacks keep a pointer to it
    s: Box<Sys>,
}

impl View {
    /// `None` = no window or no WebView2 loader/runtime. The view is usable once `inbox.state` is `Ready`.
    pub fn new() -> Option<Self> {
        unsafe {
            CoInitializeEx(null_mut(), 2); // apartment threaded; "already initialised" is fine
            let hwnd = CreateWindowExA(
                0,
                c"STATIC".as_ptr(),
                null(),
                0x8000_0000,
                0,
                0,
                1,
                1,
                0,
                0,
                0,
                null(),
            );
            if hwnd == 0 {
                return None;
            }
            let mut inbox = Box::new(Inbox::new());
            let h = |kind| Handler {
                vtbl: &VT,
                sys: null_mut(),
                kind,
            };
            let mut s = Box::new(Sys {
                inbox: &mut *inbox,
                hwnd,
                env: null_mut(),
                ctrl: null_mut(),
                wv: null_mut(),
                token: 0,
                h: [h(0), h(1), h(2)],
            });
            let sp: *mut Sys = &mut *s;
            s.h.iter_mut().for_each(|h| h.sys = sp);
            let hr = CreateCoreWebView2EnvironmentWithOptions(
                null(),
                null(),
                null_mut(),
                (&raw mut s.h[0]).cast(),
            );
            if hr < 0 {
                DestroyWindow(hwnd);
                return None;
            }
            Some(Self { inbox, s })
        }
    }

    /// Run the callbacks that are due (replies land in `inbox`); never blocks.
    pub fn pump(&mut self) {
        unsafe {
            let mut m: Msg = core::mem::zeroed();
            while PeekMessageA(&mut m, 0, 0, 0, PM_REMOVE | PM_QS_POSTMESSAGE) != 0 {
                DispatchMessageA(&m);
            }
        }
    }

    /// Start decoding `payload` (yEnc byte values); the answer comes back through `inbox`. Only when `Ready`.
    pub fn run(&mut self, id: usize, sound: bool, payload: &[u8]) {
        let mut m: Vec<u16> = alloc::format!("{id}:{}:", if sound { 'a' } else { 'i' })
            .bytes()
            .map(u16::from)
            .collect();
        m.extend(payload.iter().map(|&c| c as u16)); // yEnc never produces NUL
        m.push(0);
        unsafe {
            let post: unsafe extern "system" fn(P, *const u16) -> Hr = vt(self.s.wv, 33);
            post(self.s.wv, m.as_ptr());
        }
    }
}

impl Drop for View {
    fn drop(&mut self) {
        unsafe {
            let s = &mut *self.s;
            if !s.wv.is_null() {
                vt::<unsafe extern "system" fn(P, i64) -> Hr>(s.wv, 35)(s.wv, s.token);
                release(s.wv);
            }
            if !s.ctrl.is_null() {
                vt::<unsafe extern "system" fn(P) -> Hr>(s.ctrl, 24)(s.ctrl);
                release(s.ctrl);
            }
            if !s.env.is_null() {
                release(s.env);
            }
            DestroyWindow(s.hwnd);
        }
    }
}

unsafe extern "system" fn invoke(this: P, a: usize, b: P) -> Hr {
    unsafe {
        let h = &*(this as *const Handler);
        let s = &mut *h.sys;
        let inbox = &mut *s.inbox;
        match h.kind {
            0 | 1 if (a as i32) < 0 || b.is_null() => inbox.state = State::Dead,
            0 => {
                // `a` = HRESULT, `b` = environment
                add_ref(b);
                s.env = b;
                let create: unsafe extern "system" fn(P, isize, P) -> Hr = vt(b, 3);
                if create(b, s.hwnd, (&raw mut s.h[1]).cast()) < 0 {
                    inbox.state = State::Dead;
                }
            }
            1 => {
                // `a` = HRESULT, `b` = controller
                add_ref(b);
                s.ctrl = b;
                vt::<unsafe extern "system" fn(P, *mut P) -> Hr>(b, 25)(b, &mut s.wv); // we own the reference it returns
                vt::<unsafe extern "system" fn(P, P, *mut i64) -> Hr>(s.wv, 34)(
                    s.wv,
                    (&raw mut s.h[2]).cast(),
                    &mut s.token,
                );
                let mut html: Vec<u16> = [web::POST, web::PAGE, web::END]
                    .iter()
                    .flat_map(|p| p.bytes())
                    .map(u16::from)
                    .collect();
                html.push(0);
                vt::<unsafe extern "system" fn(P, *const u16) -> Hr>(s.wv, 6)(s.wv, html.as_ptr());
            }
            _ => {
                // `a` = sender, `b` = event args
                let mut w: *mut u16 = null_mut();
                if vt::<unsafe extern "system" fn(P, *mut *mut u16) -> Hr>(b, 5)(b, &mut w) >= 0
                    && !w.is_null()
                {
                    let mut v = Vec::new();
                    let mut p = w;
                    while *p != 0 {
                        v.push(*p as u8); // the page only sends code points 0..=255
                        p = p.add(1);
                    }
                    CoTaskMemFree(w.cast());
                    inbox.push(&v);
                }
            }
        }
        0
    }
}
