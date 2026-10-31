// No CRT: kernel32/user32 are imported directly (raw-dylib); linker flags are in .cargo/config.toml.
use super::*;
use core::{
    alloc::{GlobalAlloc, Layout},
    ffi::c_void,
    sync::atomic::{AtomicBool, Ordering::Relaxed},
};
use win32_sys::*;

pub const SURFACE_EXT: &CStr = khr::win32_surface::NAME;

#[cfg(all(feature = "rt", not(test)))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { ExitProcess(3) }
}

struct Heap;

// HeapAlloc is 16-byte aligned, nothing here asks for more
unsafe impl GlobalAlloc for Heap {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        rt_check();
        HeapAlloc(GetProcessHeap(), 0, l.size()) as *mut u8
    }
    unsafe fn dealloc(&self, p: *mut u8, _: Layout) {
        rt_check();
        HeapFree(GetProcessHeap(), 0, p as *mut c_void);
    }
}

#[global_allocator]
static ALLOC: Heap = Heap;

// MSVC references this symbol when floats are used and the CRT is absent.
#[unsafe(no_mangle)]
static _fltused: i32 = 0;

// The CRT is dropped (/NODEFAULTLIB), so a >4KB stack frame (inlined under LTO) needs our own probe.
// x86_64 only; add an aarch64 variant if you ever target it
#[cfg(all(feature = "rt", not(test), target_arch = "x86_64"))]
core::arch::global_asm!(
    ".globl __chkstk",
    "__chkstk:",
    "push rcx",
    "push rax",
    "cmp rax, 0x1000",
    "lea rcx, [rsp + 24]",
    "jb 3f",
    "2:",
    "sub rcx, 0x1000",
    "test [rcx], rcx",
    "sub rax, 0x1000",
    "cmp rax, 0x1000",
    "ja 2b",
    "3:",
    "sub rcx, rax",
    "test [rcx], rcx",
    "pop rax",
    "pop rcx",
    "ret",
);

/// Cheap id of the calling thread.
pub fn thread_id() -> usize {
    unsafe { GetCurrentThreadId() as usize }
}

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
static CLOSED: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn wndproc(h: isize, m: u32, w: usize, l: isize) -> isize {
    match m {
        WM_SIZE => RESIZED.store(true, Relaxed),
        // WM_CLOSE is sent, not posted, so poll never sees it; don't let DefWindowProc destroy the window
        WM_CLOSE => {
            CLOSED.store(true, Relaxed);
            return 0;
        }
        _ => {}
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
        die(REGISTER_CLASS_FAILED);
    }
    let hwnd = CreateWindowExA(
        0,
        c"w".as_ptr(),
        title.as_ptr(),
        WS_OVERLAPPEDWINDOW,
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
        die(CREATE_WINDOW_FAILED);
    }
    Window { hinstance, hwnd }
}

/// Windows start hidden (an unpainted window during Vulkan init looks like a white hang); call once the first frame is queued.
pub unsafe fn show(w: &Window) {
    ShowWindow(w.hwnd, SW_SHOW);
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

/// Key id for a key message. WM_KEYDOWN reports generic Shift/Ctrl/Alt and shares VK_RETURN with the numpad,
/// so left/right and numpad Enter are told apart from the scancode / extended bit in lparam.
fn key_of(m: &Msg) -> Option<u8> {
    let ext = (m.lparam >> 24) & 1 != 0;
    let vk = match m.wparam {
        0x10 if (m.lparam >> 16) & 0xFF == 0x36 => 0xA1, // right shift
        0x10 => 0xA0,
        0x11 => 0xA2 + ext as usize,
        0x12 => 0xA4 + ext as usize,
        0x0D if ext => return Some(input::NumpadEnter),
        v => v,
    };
    input::from_windows(u8::try_from(vk).ok()?)
}

/// Seconds on the performance counter.
pub fn now() -> f64 {
    let (mut t, mut f) = (0i64, 1i64);
    unsafe {
        QueryPerformanceCounter(&mut t);
        QueryPerformanceFrequency(&mut f);
    }
    t as f64 / f as f64
}

/// Full path of the running executable (empty if unknown or too long).
#[cfg(not(debug_assertions))]
pub(crate) fn exe_path() -> Vec<u8> {
    let mut b = vec![0u8; 1024];
    let n = unsafe { GetModuleFileNameA(0, b.as_mut_ptr().cast(), b.len() as u32) } as usize;
    b.truncate(if n == b.len() { 0 } else { n }); // a full buffer means the path was cut off
    b
}

/// Whole file into memory, or `None` if it can't be opened or read (files over 4 GB are not supported).
pub fn read_file(path: &CStr) -> Option<Vec<u8>> {
    unsafe {
        // GENERIC_READ, FILE_SHARE_READ, OPEN_EXISTING
        let h = CreateFileA(path.as_ptr(), 0x8000_0000, 1, ptr::null(), 3, 0, 0);
        if h == -1 {
            return None;
        }
        let n = GetFileSize(h, ptr::null_mut());
        if n == u32::MAX {
            CloseHandle(h);
            return None;
        }
        let mut v = vec![0u8; n as usize];
        let mut got = 0u32;
        let ok = ReadFile(h, v.as_mut_ptr().cast(), n, &mut got, ptr::null_mut());
        CloseHandle(h);
        (ok != 0 && got == n).then_some(v)
    }
}

static CAPTURED: AtomicBool = AtomicBool::new(false);

/// FPS-style mouse: while the window is in the foreground the cursor is hidden and confined to it, and every call
/// returns how far it moved (pixels, x right, y down) since the last call, then puts it back on (`cx`, `cy`) so it
/// never runs out of room. Returns 0 while the window is in the background.
pub unsafe fn mouse_delta(w: &Window, cx: i32, cy: i32) -> [f32; 2] {
    let was = CAPTURED.load(Relaxed);
    if GetForegroundWindow() != w.hwnd {
        if was {
            ClipCursor(ptr::null());
            ShowCursor(1);
            CAPTURED.store(false, Relaxed);
        }
        return [0.0; 2];
    }
    // the window may have moved or resized: refresh the clip rectangle and the center every call
    let (mut r, mut c) = ([0i32; 4], [cx, cy]);
    GetClientRect(w.hwnd, &mut r);
    let (mut tl, mut br) = ([r[0], r[1]], [r[2], r[3]]);
    ClientToScreen(w.hwnd, &mut tl);
    ClientToScreen(w.hwnd, &mut br);
    ClientToScreen(w.hwnd, &mut c);
    ClipCursor(&[tl[0], tl[1], br[0], br[1]]);
    if !was {
        ShowCursor(0);
        SetCursorPos(c[0], c[1]); // so the first real delta starts from the center
        CAPTURED.store(true, Relaxed);
        return [0.0; 2];
    }
    let mut p = [0i32; 2];
    GetCursorPos(&mut p);
    let d = [p[0] - c[0], p[1] - c[1]];
    if d != [0, 0] {
        SetCursorPos(c[0], c[1]);
    }
    [d[0] as f32, d[1] as f32]
}

/// The cursor in window pixels (x right, y down, may lie outside the window) and whether the left button is down.
/// `None` while the window is in the background. Polled, so a click shorter than a frame is missed.
pub unsafe fn pointer(w: &Window) -> Option<([f32; 2], bool)> {
    if GetForegroundWindow() != w.hwnd {
        return None;
    }
    let mut p = [0i32; 2];
    GetCursorPos(&mut p);
    ScreenToClient(w.hwnd, &mut p);
    Some(([p[0] as f32, p[1] as f32], GetAsyncKeyState(0x01) < 0)) // VK_LBUTTON, high bit = down
}

/// Drain pending messages (sleeping while minimized); false means the user asked to quit. `keys` is kept up to date with the held keys.
pub unsafe fn poll(w: &Window, resized: &mut bool, keys: &mut input::Keys) -> bool {
    loop {
        let mut m: Msg = core::mem::zeroed();
        while PeekMessageA(&mut m, 0, 0, 0, PM_REMOVE) != 0 {
            if m.message == WM_KEYDOWN && m.wparam == VK_ESCAPE {
                return false;
            }
            if let Some(k) = key_of(&m) {
                match m.message {
                    WM_KEYDOWN | WM_SYSKEYDOWN => keys.set(k, true),
                    WM_KEYUP | WM_SYSKEYUP => keys.set(k, false),
                    _ => {}
                }
            }
            DispatchMessageA(&m);
        }
        if CLOSED.load(Relaxed) {
            return false;
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

/// A running child process whose stdout is a pipe we only read when data is waiting. Dropping it kills the process
/// if it is still running.
pub struct Child {
    pipe: isize,
    process: isize, // 0 once waited for
}

/// Starts `args[0]` (looked up on PATH) with its stdout piped to the returned `Child`; stderr is inherited, no
/// console window opens. `None` if it can't start (not installed). Arguments are quoted, with `"` escaped as `\"`.
pub fn spawn(args: &[&CStr]) -> Option<Child> {
    unsafe {
        let mut cmd = Vec::new();
        for a in args {
            cmd.push(b'"');
            for &c in a.to_bytes() {
                if c == b'"' {
                    cmd.push(b'\\');
                }
                cmd.push(c);
            }
            cmd.extend_from_slice(b"\" ");
        }
        cmd.push(0);
        let sa = SecurityAttributes {
            len: size_of::<SecurityAttributes>() as u32,
            descriptor: ptr::null(),
            inherit: 1,
        };
        let (mut r, mut w) = (0, 0);
        if CreatePipe(&mut r, &mut w, &sa, 0) == 0 {
            return None;
        }
        SetHandleInformation(r, HANDLE_FLAG_INHERIT, 0); // the child must not hold our end
        let si = StartupInfo {
            cb: size_of::<StartupInfo>() as u32,
            reserved: ptr::null(),
            desktop: ptr::null(),
            title: ptr::null(),
            geometry: [0; 7],
            flags: STARTF_USESTDHANDLES,
            show: 0,
            reserved2_len: 0,
            reserved2: ptr::null(),
            std_in: 0,
            std_out: w,
            std_err: GetStdHandle(STD_ERROR_HANDLE),
        };
        let mut pi = ProcessInfo {
            process: 0,
            thread: 0,
            pid: 0,
            tid: 0,
        };
        let ok = CreateProcessA(
            ptr::null(),
            cmd.as_mut_ptr().cast(),
            ptr::null(),
            ptr::null(),
            1,
            CREATE_NO_WINDOW,
            ptr::null(),
            ptr::null(),
            &si,
            &mut pi,
        );
        CloseHandle(w);
        if ok == 0 {
            CloseHandle(r);
            return None;
        }
        CloseHandle(pi.thread);
        Some(Child {
            pipe: r,
            process: pi.process,
        })
    }
}

impl Child {
    /// Never blocks.
    pub fn read(&mut self, buf: &mut [u8]) -> Read {
        unsafe {
            let mut avail = 0u32;
            // fails once the child has closed its end and the pipe is drained
            if PeekNamedPipe(
                self.pipe,
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                &mut avail,
                ptr::null_mut(),
            ) == 0
            {
                return Read::Eof;
            }
            if avail == 0 {
                return Read::Empty;
            }
            let mut got = 0u32;
            let n = avail.min(buf.len() as u32);
            if ReadFile(
                self.pipe,
                buf.as_mut_ptr().cast(),
                n,
                &mut got,
                ptr::null_mut(),
            ) == 0
            {
                return Read::Eof;
            }
            Read::Data(got as usize)
        }
    }

    /// Call after `Read::Eof`: waits for the process and says whether it exited with code 0.
    pub fn wait(&mut self) -> bool {
        unsafe {
            WaitForSingleObject(self.process, INFINITE);
            let mut code = 1u32;
            let ok = GetExitCodeProcess(self.process, &mut code) != 0 && code == 0;
            CloseHandle(self.process);
            self.process = 0;
            ok
        }
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.pipe);
            if self.process != 0 {
                TerminateProcess(self.process, 1);
                CloseHandle(self.process);
            }
        }
    }
}

pub fn sleep_ms(ms: u32) {
    unsafe { Sleep(ms) }
}
