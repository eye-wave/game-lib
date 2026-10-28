#![no_std]
#![cfg(windows)]

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
    pub fn QueryPerformanceCounter(c: *mut i64) -> i32;
    pub fn QueryPerformanceFrequency(f: *mut i64) -> i32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
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
