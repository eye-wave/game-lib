// No CRT: kernel32/user32 are imported directly (raw-dylib); linker flags are in .cargo/config.toml.
use super::*;
use core::{
    alloc::{GlobalAlloc, Layout},
    ffi::c_void,
    sync::atomic::{AtomicBool, Ordering::Relaxed},
};
use win32_sys::*;

pub const SURFACE_EXT: &CStr = khr::win32_surface::NAME;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { ExitProcess(3) }
}

struct Heap;

// HeapAlloc is 16-byte aligned, nothing here asks for more
unsafe impl GlobalAlloc for Heap {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        HeapAlloc(GetProcessHeap(), 0, l.size()) as *mut u8
    }
    unsafe fn dealloc(&self, p: *mut u8, _: Layout) {
        HeapFree(GetProcessHeap(), 0, p as *mut c_void);
    }
}

#[global_allocator]
static ALLOC: Heap = Heap;

// MSVC references this symbol when floats are used and the CRT is absent.
#[unsafe(no_mangle)]
static _fltused: i32 = 0;

pub fn die(msg: &CStr) -> ! {
    unsafe {
        MessageBoxA(0, msg.as_ptr(), c"error".as_ptr(), MB_ICONERROR);
        ExitProcess(1)
    }
}

pub fn exit(code: u32) -> ! {
    unsafe { ExitProcess(code) }
}

pub struct Window {
    hinstance: isize,
    hwnd: isize,
}

static RESIZED: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn wndproc(h: isize, m: u32, w: usize, l: isize) -> isize {
    if m == WM_SIZE {
        RESIZED.store(true, Relaxed);
    }
    DefWindowProcA(h, m, w, l)
}

pub unsafe fn open(width: u16, height: u16, title: &CStr) -> Window {
    let hinstance = GetModuleHandleA(ptr::null());
    let class = WndClass {
        style: 0,
        wnd_proc: Some(wndproc),
        cls_extra: 0,
        wnd_extra: 0,
        instance: hinstance,
        icon: 0,
        cursor: 0,
        background: 0,
        menu_name: ptr::null(),
        class_name: c"w".as_ptr(),
    };
    if RegisterClassA(&class) == 0 {
        die(c"RegisterClass failed");
    }
    let hwnd = CreateWindowExA(
        0,
        c"w".as_ptr(),
        title.as_ptr(),
        WS_OVERLAPPEDWINDOW_VISIBLE,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        width as i32,
        height as i32,
        0,
        0,
        hinstance,
        ptr::null(),
    );
    if hwnd == 0 {
        die(c"CreateWindow failed");
    }
    Window { hinstance, hwnd }
}

pub unsafe fn surface(instance: vk::Instance, w: &Window) -> vk::SurfaceKHR {
    let mut s = vk::SurfaceKHR::null();
    let r = vkCreateWin32SurfaceKHR(
        instance,
        &vk::Win32SurfaceCreateInfoKHR::default()
            .hinstance(w.hinstance as _)
            .hwnd(w.hwnd as _),
        NA,
        &mut s,
    );
    surface_ck(r, s)
}

/// Drain pending messages (sleeping while minimized); false means the user asked to quit.
pub unsafe fn poll(w: &Window, resized: &mut bool) -> bool {
    loop {
        let mut m: Msg = core::mem::zeroed();
        while PeekMessageA(&mut m, 0, 0, 0, PM_REMOVE) != 0 {
            if m.message == WM_CLOSE || (m.message == WM_KEYDOWN && m.wparam == VK_ESCAPE) {
                return false;
            }
            DispatchMessageA(&m);
        }
        if IsIconic(w.hwnd) == 0 {
            break;
        }
        WaitMessage();
    }
    if RESIZED.swap(false, Relaxed) {
        *resized = true;
    }
    true
}

/// `platform::entry!(run)`: makes `unsafe fn run()` the process entry point.
#[macro_export]
macro_rules! entry {
    ($f:path) => {
        #[unsafe(no_mangle)]
        pub extern "system" fn start() -> ! {
            unsafe { $f() };
            $crate::exit(0)
        }
    };
}
