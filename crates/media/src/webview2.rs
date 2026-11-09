//! Windows backend: a hidden WebView2 (Edge) view over raw COM, no crates. Needs the WebView2 Runtime (Windows 11 has
//! it) and `WebView2Loader.dll` next to the exe (linked with raw-dylib, so there is no import library to build
//! with). The browser profile is one folder shared by all apps, `%LOCALAPPDATA%\stupid-game\webview` (the default, a
//! folder next to the exe, is the fallback). Create and pump it on one thread that has a message loop; `platform`'s event poll is one.
//!
//! The page is `NavigateToString`'d and talks to the made-up host `web::BASE` (https://media.localhost/): a
//! `WebResourceRequested` filter catches those requests before they reach the network. `GET .../a/<name>` is answered
//! from the assets directory, `POST .../r/...` hands its raw body to the `Inbox`. See web.rs.
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
#[link(name = "shlwapi", kind = "raw-dylib")]
unsafe extern "system" {
    /// A new IStream over a copy of `cb` bytes (null `init`: empty).
    fn SHCreateMemStream(init: *const u8, cb: u32) -> P;
}
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetEnvironmentVariableW(name: *const u16, buf: *mut u16, n: u32) -> u32;
}
#[link(name = "shell32", kind = "raw-dylib")]
unsafe extern "system" {
    /// Creates the folder and any missing parents; 0 = created.
    fn SHCreateDirectoryExW(hwnd: isize, path: *const u16, sa: P) -> i32;
}
#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DestroyWindow(h: isize) -> i32;
}

/// Method `i` of a COM object's vtable (0..=2 are IUnknown). Slots used below, from WebView2.idl:
/// Environment 3 CreateCoreWebView2Controller, 4 CreateWebResourceResponse; Controller 24 Close, 25 get_CoreWebView2;
/// WebView 6 NavigateToString, 33 PostWebMessageAsString, 34/35 add/remove_WebMessageReceived,
/// 55/56 add/remove_WebResourceRequested, 57 AddWebResourceRequestedFilter;
/// message args 5 TryGetWebMessageAsString; resource args 3 get_Request, 5 put_Response;
/// request 3 get_Uri, 7 get_Content (IStream); IStream 3 Read.
unsafe fn vt<F>(obj: P, i: usize) -> F {
    unsafe { core::mem::transmute_copy(&*(*(obj as *const *const usize)).add(i)) }
}

/// NUL-terminated UTF-16 of a str (only ever called with ASCII).
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// The low bytes of a NUL-terminated UTF-16 string (`web::BASE` urls are ASCII).
unsafe fn narrow(mut p: *const u16) -> Vec<u8> {
    let mut v = Vec::new();
    while !p.is_null() && unsafe { *p } != 0 {
        v.push(unsafe { *p } as u8);
        p = unsafe { p.add(1) };
    }
    v
}

/// `%LOCALAPPDATA%\stupid-game\webview`, created: one browser profile for every app made with this engine (WebView2
/// lets several processes share a folder as long as they ask for the same environment, which is the default
/// one). `None` if anything about it fails: WebView2 then falls back to its own default, a `<exe>.WebView2` folder next
/// to the executable.
unsafe fn data_dir() -> Option<Vec<u16>> {
    unsafe {
        let mut base = [0u16; 512];
        let n =
            GetEnvironmentVariableW(wide("LOCALAPPDATA").as_ptr(), base.as_mut_ptr(), 512) as usize;
        if n == 0 || n >= 512 {
            return None; // unset, or too long for the buffer
        }
        let mut p = base[..n].to_vec();
        p.extend("\\stupid-game\\webview".encode_utf16());
        p.push(0);
        // 0 = created, 80 / 183 = already there
        matches!(
            SHCreateDirectoryExW(0, p.as_ptr(), null_mut()),
            0 | 80 | 183
        )
        .then_some(p)
    }
}

unsafe fn add_ref(obj: P) {
    unsafe { vt::<unsafe extern "system" fn(P) -> u32>(obj, 1)(obj) };
}

unsafe fn release(obj: P) {
    unsafe { vt::<unsafe extern "system" fn(P) -> u32>(obj, 2)(obj) };
}

// One hand-made COM callback object serves all four handler interfaces: each has `Invoke(a, b)` in slot 3, and
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
    kind: u8, // 0 environment created, 1 controller created, 2 message received, 3 resource requested
}

struct Sys {
    inbox: *mut Inbox,
    hwnd: isize,
    env: P,
    ctrl: P,
    wv: P,
    token: i64,
    token_res: i64,
    h: [Handler; 4],
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
                token_res: 0,
                h: [h(0), h(1), h(2), h(3)],
            });
            let sp: *mut Sys = &mut *s;
            s.h.iter_mut().for_each(|h| h.sys = sp);
            let dir = data_dir();
            let handler: P = (&raw mut s.h[0]).cast();
            let mut hr = CreateCoreWebView2EnvironmentWithOptions(
                null(),
                dir.as_ref().map_or(null(), |d| d.as_ptr()),
                null_mut(),
                handler,
            );
            if hr < 0 && dir.is_some() {
                hr = CreateCoreWebView2EnvironmentWithOptions(null(), null(), null_mut(), handler);
            }
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

    /// Start decoding asset `name` (`kind` as in `lib.rs`; `name` and `head` already `web::escape`d, so no `:` in
    /// them); the page fetches the file and the answer comes back through `inbox`. Only when `Ready`.
    pub fn run(&mut self, id: usize, kind: char, name: &str, head: &str) {
        let m = wide(&alloc::format!("{id}:{kind}:{name}:{head}"));
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
                vt::<unsafe extern "system" fn(P, i64) -> Hr>(s.wv, 56)(s.wv, s.token_res);
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
                vt::<unsafe extern "system" fn(P, P, *mut i64) -> Hr>(s.wv, 55)(
                    s.wv,
                    (&raw mut s.h[3]).cast(),
                    &mut s.token_res,
                );
                let filter = wide(&alloc::format!("{}*", web::BASE));
                vt::<unsafe extern "system" fn(P, *const u16, i32) -> Hr>(s.wv, 57)(
                    s.wv,
                    filter.as_ptr(),
                    0, // COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL
                );
                let html = wide(&[web::POST, &web::page(), web::END].concat());
                vt::<unsafe extern "system" fn(P, *const u16) -> Hr>(s.wv, 6)(s.wv, html.as_ptr());
            }
            3 => {
                // `a` = sender, `b` = event args
                let mut req: P = null_mut();
                vt::<unsafe extern "system" fn(P, *mut P) -> Hr>(b, 3)(b, &mut req);
                if req.is_null() {
                    return 0;
                }
                let mut u: *mut u16 = null_mut();
                vt::<unsafe extern "system" fn(P, *mut *mut u16) -> Hr>(req, 3)(req, &mut u);
                let uri = narrow(u); // the page only asks for ASCII urls
                CoTaskMemFree(u.cast());
                let path = uri.strip_prefix(web::BASE.as_bytes()).unwrap_or(&[]);
                let mut asset = None;
                if let Some(name) = path.strip_prefix(b"a/") {
                    asset = core::str::from_utf8(&web::unescape(name))
                        .ok()
                        .and_then(platform::read_asset);
                } else if path.starts_with(b"r/") {
                    let mut body: P = null_mut();
                    vt::<unsafe extern "system" fn(P, *mut P) -> Hr>(req, 7)(req, &mut body);
                    let mut data = Vec::new();
                    if !body.is_null() {
                        let read: unsafe extern "system" fn(P, *mut u8, u32, *mut u32) -> Hr =
                            vt(body, 3);
                        let mut buf = [0u8; 1 << 16];
                        loop {
                            let mut n = 0;
                            let hr = read(body, buf.as_mut_ptr(), buf.len() as u32, &mut n);
                            data.extend_from_slice(&buf[..n as usize]);
                            if hr < 0 || n == 0 {
                                break;
                            }
                        }
                        release(body);
                    }
                    inbox.push_reply(path, &data);
                }
                release(req);
                let missing = path.starts_with(b"a/") && asset.is_none();
                let (bytes, len) = asset
                    .as_ref()
                    .map_or((null(), 0), |d| (d.as_ptr(), d.len()));
                let mem = SHCreateMemStream(bytes, len as u32);
                // the page's origin is not ours: let it read the answer
                let headers = wide("Access-Control-Allow-Origin: *\r\n");
                let (code, reason) = if missing {
                    (404, wide("Not Found"))
                } else {
                    (200, wide("OK"))
                };
                let mut resp: P = null_mut();
                vt::<unsafe extern "system" fn(P, P, i32, *const u16, *const u16, *mut P) -> Hr>(
                    s.env, 4,
                )(
                    s.env,
                    mem,
                    code,
                    reason.as_ptr(),
                    headers.as_ptr(),
                    &mut resp,
                );
                if !resp.is_null() {
                    vt::<unsafe extern "system" fn(P, P) -> Hr>(b, 5)(b, resp); // put_Response
                    release(resp);
                }
                if !mem.is_null() {
                    release(mem);
                }
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
