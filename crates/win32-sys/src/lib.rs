#![cfg_attr(not(feature = "build"), no_std)]

#[cfg(windows)]
mod ffi {
    use core::ffi::{c_char, c_void};

    pub type Wndproc = unsafe extern "system" fn(isize, u32, usize, isize) -> isize;

    #[repr(C)]
    pub struct WndClass {
        pub style: u32,
        pub wnd_proc: Option<Wndproc>,
        pub cls_extra: i32,
        pub wnd_extra: i32,
        pub instance: isize,
        pub icon: isize,
        pub cursor: isize,
        pub background: isize,
        pub menu_name: *const c_char,
        pub class_name: *const c_char,
    }

    #[repr(C)]
    pub struct Msg {
        pub hwnd: isize,
        pub message: u32,
        pub wparam: usize,
        pub lparam: isize,
        pub time: u32,
        pub pt: [i32; 2],
        pub private: u32,
    }

    #[repr(C)]
    pub struct SecurityAttributes {
        pub len: u32,
        pub descriptor: *const c_void,
        pub inherit: i32,
    }

    #[repr(C)]
    pub struct StartupInfo {
        pub cb: u32,
        pub reserved: *const c_char,
        pub desktop: *const c_char,
        pub title: *const c_char,
        pub geometry: [u32; 7], // x, y, width, height, count chars x/y, fill attribute
        pub flags: u32,
        pub show: u16,
        pub reserved2_len: u16,
        pub reserved2: *const u8,
        pub std_in: isize,
        pub std_out: isize,
        pub std_err: isize,
    }

    #[repr(C)]
    pub struct ProcessInfo {
        pub process: isize,
        pub thread: isize,
        pub pid: u32,
        pub tid: u32,
    }

    pub const STARTF_USESTDHANDLES: u32 = 0x100;
    pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    pub const HANDLE_FLAG_INHERIT: u32 = 1;
    pub const STD_ERROR_HANDLE: u32 = -12i32 as u32;
    pub const INFINITE: u32 = u32::MAX;

    pub const WM_SIZE: u32 = 0x5;
    pub const WM_CLOSE: u32 = 0x10;
    pub const WM_KEYDOWN: u32 = 0x100;
    pub const WM_KEYUP: u32 = 0x101;
    pub const WM_SYSKEYDOWN: u32 = 0x104;
    pub const WM_SYSKEYUP: u32 = 0x105;
    pub const VK_ESCAPE: usize = 0x1B;
    pub const PM_REMOVE: u32 = 1;
    pub const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
    pub const SW_SHOW: i32 = 5;
    pub const CW_USEDEFAULT: i32 = i32::MIN;
    pub const MB_ICONERROR: u32 = 0x10;

    #[link(name = "kernel32", kind = "raw-dylib")]
    unsafe extern "system" {
        pub fn GetModuleHandleA(name: *const c_char) -> isize;
        pub fn GetProcessHeap() -> *mut c_void;
        pub fn HeapAlloc(heap: *mut c_void, flags: u32, size: usize) -> *mut c_void;
        pub fn HeapFree(heap: *mut c_void, flags: u32, p: *mut c_void) -> i32;
        pub fn ExitProcess(code: u32) -> !;
        pub fn GetCurrentThreadId() -> u32;
        pub fn QueryPerformanceCounter(c: *mut i64) -> i32;
        pub fn QueryPerformanceFrequency(f: *mut i64) -> i32;
        pub fn CreateFileA(
            name: *const c_char,
            access: u32,
            share: u32,
            security: *const c_void,
            disposition: u32,
            flags: u32,
            template: isize,
        ) -> isize;
        pub fn GetFileSize(h: isize, high: *mut u32) -> u32;
        pub fn GetModuleFileNameA(module: isize, buf: *mut c_char, size: u32) -> u32;
        pub fn ReadFile(
            h: isize,
            buf: *mut c_void,
            n: u32,
            read: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        pub fn CloseHandle(h: isize) -> i32;
        pub fn Sleep(ms: u32);
        pub fn GetStdHandle(which: u32) -> isize;
        pub fn CreatePipe(
            read: *mut isize,
            write: *mut isize,
            sa: *const SecurityAttributes,
            size: u32,
        ) -> i32;
        pub fn SetHandleInformation(h: isize, mask: u32, flags: u32) -> i32;
        pub fn PeekNamedPipe(
            h: isize,
            buf: *mut c_void,
            n: u32,
            read: *mut u32,
            avail: *mut u32,
            left: *mut u32,
        ) -> i32;
        pub fn CreateProcessA(
            app: *const c_char,
            cmdline: *mut c_char,
            process_sa: *const c_void,
            thread_sa: *const c_void,
            inherit: i32,
            flags: u32,
            env: *const c_void,
            dir: *const c_char,
            si: *const StartupInfo,
            pi: *mut ProcessInfo,
        ) -> i32;
        pub fn WaitForSingleObject(h: isize, ms: u32) -> u32;
        pub fn GetExitCodeProcess(h: isize, code: *mut u32) -> i32;
        pub fn TerminateProcess(h: isize, code: u32) -> i32;
    }

    #[link(name = "user32", kind = "raw-dylib")]
    unsafe extern "system" {
        pub fn ClientToScreen(h: isize, p: *mut [i32; 2]) -> i32;
        pub fn SetCursorPos(x: i32, y: i32) -> i32;
        pub fn GetCursorPos(p: *mut [i32; 2]) -> i32;
        pub fn ScreenToClient(h: isize, p: *mut [i32; 2]) -> i32;
        pub fn GetAsyncKeyState(vk: i32) -> i16;
        pub fn GetClientRect(h: isize, r: *mut [i32; 4]) -> i32;
        pub fn ClipCursor(r: *const [i32; 4]) -> i32;
        pub fn ShowCursor(show: i32) -> i32;
        pub fn GetForegroundWindow() -> isize;
        pub fn RegisterClassA(c: *const WndClass) -> u16;
        pub fn CreateWindowExA(
            ex_style: u32,
            class: *const c_char,
            title: *const c_char,
            style: u32,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            parent: isize,
            menu: isize,
            instance: isize,
            param: *const c_void,
        ) -> isize;
        pub fn ShowWindow(h: isize, cmd: i32) -> i32;
        pub fn DefWindowProcA(h: isize, m: u32, w: usize, l: isize) -> isize;
        pub fn PeekMessageA(m: *mut Msg, h: isize, min: u32, max: u32, remove: u32) -> i32;
        pub fn DispatchMessageA(m: *const Msg) -> isize;
        pub fn WaitMessage() -> i32;
        pub fn IsIconic(h: isize) -> i32;
        pub fn MessageBoxA(h: isize, text: *const c_char, caption: *const c_char, ty: u32) -> i32;
    }
}
#[cfg(windows)]
pub use ffi::*;

/// For build.rs (feature `build`): embed an app manifest and version info into the exe.
/// `None` uses `default.manifest` from this crate; `Some(path)` uses your own file (relative to the app's dir).
/// Does nothing unless the target is Windows, and warns if the host can't do it (winresource needs a Windows host).
#[cfg(feature = "build")]
pub fn embed_manifest(path: Option<&str>) {
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|o| o == "windows") {
        #[cfg(windows)]
        {
            let name = std::env::var("CARGO_PKG_NAME").unwrap();
            let mut res = winresource::WindowsResource::new();
            res.set("CompanyName", "YOUR NAME")
                .set("FileDescription", &name)
                .set("ProductName", &name)
                .set("LegalCopyright", "Copyright (c) YOUR NAME")
                .set("OriginalFilename", &(name + ".exe"));
            match path {
                Some(p) => {
                    println!("cargo:rerun-if-changed={p}");
                    res.set_manifest_file(p)
                }
                None => res.set_manifest(include_str!("../default.manifest")),
            };
            res.compile().unwrap();
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            println!(
                "cargo:warning=app manifest not embedded: building for Windows needs a Windows host"
            );
        }
    }
}
