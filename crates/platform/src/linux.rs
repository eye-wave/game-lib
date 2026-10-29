use super::*;
use core::{
    alloc::{GlobalAlloc, Layout},
    ffi::{c_char, c_void},
};
use xcb_sys::*;

pub const SURFACE_EXT: &CStr = khr::xcb_surface::NAME;

#[link(name = "c")]
unsafe extern "C" {}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { libc::abort() }
}

struct Malloc;

unsafe impl GlobalAlloc for Malloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if l.align() <= 16 {
            libc::malloc(l.size()) as *mut u8
        } else {
            let mut p: *mut c_void = ptr::null_mut();
            libc::posix_memalign(&mut p, l.align(), l.size());
            p as *mut u8
        }
    }
    unsafe fn dealloc(&self, p: *mut u8, _: Layout) {
        libc::free(p as *mut c_void)
    }
}

#[global_allocator]
static ALLOC: Malloc = Malloc;

pub fn die(msg: &CStr) -> ! {
    unsafe {
        libc::puts(msg.as_ptr());
        libc::exit(1)
    }
}

pub struct Window {
    conn: *mut Conn,
    win: u32,
    wm_delete: u32,
}

unsafe fn intern(c: *mut Conn, name: &str) -> u32 {
    let ck = xcb_intern_atom(c, 0, name.len() as u16, name.as_ptr() as *const c_char);
    let r = xcb_intern_atom_reply(c, ck, ptr::null_mut());
    if r.is_null() {
        die(XCB_INTERN_ATOM_FAILED);
    }
    let atom = (*r).atom;
    libc::free(r as *mut c_void);
    atom
}

pub unsafe fn open(width: u16, height: u16, title: &CStr) -> Window {
    let conn = xcb_connect(ptr::null(), ptr::null_mut());
    if conn.is_null() || xcb_connection_has_error(conn) != 0 {
        die(CANNOT_CONNECT_TO_X_SERVER_DISPLAY_NOT_SET);
    }
    let root = (*xcb_setup_roots_iterator(xcb_get_setup(conn)).data).root;

    let win = xcb_generate_id(conn);
    let event_mask = [XCB_EVENT_MASK_KEY_PRESS
        | XCB_EVENT_MASK_KEY_RELEASE
        | XCB_EVENT_MASK_POINTER_MOTION
        | XCB_EVENT_MASK_STRUCTURE_NOTIFY];
    xcb_create_window(
        conn,
        0,
        win,
        root,
        0,
        0,
        width,
        height,
        0,
        1,
        0,
        XCB_CW_EVENT_MASK,
        event_mask.as_ptr() as *const c_void,
    );
    let title = title.to_bytes();
    xcb_change_property(
        conn,
        0,
        win,
        XCB_ATOM_WM_NAME,
        XCB_ATOM_STRING,
        8,
        title.len() as u32,
        title.as_ptr() as *const c_void,
    );
    let wm_protocols = intern(conn, "WM_PROTOCOLS");
    let wm_delete = intern(conn, "WM_DELETE_WINDOW");
    xcb_change_property(
        conn,
        0,
        win,
        wm_protocols,
        XCB_ATOM_ATOM,
        32,
        1,
        &wm_delete as *const u32 as *const c_void,
    );
    Window {
        conn,
        win,
        wm_delete,
    }
}

/// Windows start unmapped (an unpainted window during Vulkan init looks like a hang); call once the first frame is queued.
pub unsafe fn show(w: &Window) {
    xcb_map_window(w.conn, w.win);
    xcb_flush(w.conn);
}

pub unsafe fn surface(instance: vk::Instance, w: &Window) -> vk::SurfaceKHR {
    let mut s = vk::SurfaceKHR::null();
    let r = vkCreateXcbSurfaceKHR(
        instance,
        &vk::XcbSurfaceCreateInfoKHR::default()
            .connection(w.conn as *mut _)
            .window(w.win),
        NA,
        &mut s,
    );
    surface_ck(r, s)
}

/// Seconds on a monotonic clock.
pub fn now() -> f64 {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut t) };
    t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9
}

/// Drain pending events; false means the user asked to quit. `state` is kept up to date with the held keys and the cursor.
pub unsafe fn poll(w: &Window, resized: &mut bool, state: &mut input::State) -> bool {
    let mut running = true;
    loop {
        let e = xcb_poll_for_event(w.conn);
        if e.is_null() {
            break;
        }
        match (*e).response_type & 0x7f {
            XCB_CLIENT_MESSAGE => {
                if (*(e as *const ClientMsg)).data[0] == w.wm_delete {
                    running = false;
                }
            }
            XCB_CONFIGURE_NOTIFY => *resized = true,
            XCB_KEY_PRESS if (*e).detail == KEYCODE_ESCAPE => running = false,
            t @ (XCB_KEY_PRESS | XCB_KEY_RELEASE) => {
                if let Some(k) = input::from_linux((*e).detail) {
                    state.keys.set(k, t == XCB_KEY_PRESS);
                }
            }
            XCB_MOTION_NOTIFY => {
                let m = &*(e as *const MotionEvent);
                state.mouse = [m.event_x as i32, m.event_y as i32];
            }
            _ => {}
        }
        libc::free(e as *mut c_void);
    }
    running
}

/// `platform::entry!(run)`: makes `unsafe fn run()` the process entry point.
#[macro_export]
macro_rules! entry {
    ($f:path) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn main(_: i32, _: *const *const u8) -> i32 {
            unsafe { $f() };
            0
        }
    };
}
