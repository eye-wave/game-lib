use super::*;
use core::{
    alloc::{GlobalAlloc, Layout},
    cell::Cell,
    ffi::{c_char, c_void},
};
use xcb_sys::*;

pub const SURFACE_EXT: &CStr = khr::xcb_surface::NAME;

#[link(name = "c")]
unsafe extern "C" {}

#[cfg(not(test))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { libc::abort() }
}

struct Malloc;

unsafe impl GlobalAlloc for Malloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        rt_check();
        if l.align() <= 16 {
            libc::malloc(l.size()) as *mut u8
        } else {
            let mut p: *mut c_void = ptr::null_mut();
            libc::posix_memalign(&mut p, l.align(), l.size());
            p as *mut u8
        }
    }
    unsafe fn dealloc(&self, p: *mut u8, _: Layout) {
        rt_check();
        libc::free(p as *mut c_void)
    }
}

#[global_allocator]
static ALLOC: Malloc = Malloc;

/// Cheap id of the calling thread.
pub fn thread_id() -> usize {
    unsafe { libc::pthread_self() as usize }
}

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
    focused: Cell<bool>,
    grabbed: Cell<bool>, // pointer is confined to the window and hidden
    blank: Cell<u32>,    // invisible cursor, created on first grab
    xi: u8,              // XInput2 opcode when raw mouse motion is available, else 0
    dx: Cell<f32>,       // raw motion accumulated since the last `mouse_delta`
    dy: Cell<f32>,
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
        | XCB_EVENT_MASK_FOCUS_CHANGE
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
        focused: Cell::new(true),
        grabbed: Cell::new(false),
        blank: Cell::new(0),
        xi: raw_mouse_init(conn, root),
        dx: Cell::new(0.0),
        dy: Cell::new(0.0),
    }
}

static mut XINPUT: Extension = Extension {
    name: c"XInputExtension".as_ptr(),
    global_id: 0,
};

/// Ask the X server for raw mouse motion (XInput2's XI_RawMotion): device movement that does not depend on where
/// the pointer is or whether warping it works, which is what mouse look needs (pointer warping is unreliable
/// under Wayland/XWayland). Returns the extension's opcode, or 0 if unavailable.
unsafe fn raw_mouse_init(conn: *mut Conn, root: u32) -> u8 {
    let ext = &raw mut XINPUT;
    let data = xcb_get_extension_data(conn, ext);
    if data.is_null() || (*data).present == 0 {
        return 0;
    }
    // libxcb uses the two entries before the request body as scratch space, so the body starts at [2]
    let mut v = [IoVec {
        base: ptr::null_mut(),
        len: 0,
    }; 4];
    // XIQueryVersion(2, 0) has to come before any other XInput2 request
    #[repr(C)]
    struct Version {
        major: u8,
        minor: u8,
        len: u16,
        client_major: u16,
        client_minor: u16,
    }
    let mut ver = Version {
        major: 0,
        minor: 47,
        len: 2,
        client_major: 2,
        client_minor: 0,
    };
    v[2] = IoVec {
        base: (&raw mut ver).cast(),
        len: size_of::<Version>(),
    };
    let seq = xcb_send_request(
        conn,
        0,
        v[2..].as_mut_ptr(),
        &ProtocolRequest {
            count: 2,
            ext,
            opcode: 47,
            isvoid: 0,
        },
    );
    let reply = xcb_wait_for_reply(conn, seq, ptr::null_mut());
    if reply.is_null() {
        return 0;
    }
    libc::free(reply);
    // XISelectEvents: XI_RawMotion (17) from all master devices, selected on the root window
    #[repr(C)]
    struct Select {
        major: u8,
        minor: u8,
        len: u16,
        window: u32,
        num_masks: u16,
        pad: u16,
        device: u16,   // 1 = XIAllMasterDevices
        mask_len: u16, // in 4-byte units
        mask: u32,
    }
    let mut sel = Select {
        major: 0,
        minor: 46,
        len: 5,
        window: root,
        num_masks: 1,
        pad: 0,
        device: 1,
        mask_len: 1,
        mask: 1 << 17,
    };
    v[2] = IoVec {
        base: (&raw mut sel).cast(),
        len: size_of::<Select>(),
    };
    xcb_send_request(
        conn,
        0,
        v[2..].as_mut_ptr(),
        &ProtocolRequest {
            count: 2,
            ext,
            opcode: 46,
            isvoid: 1,
        },
    );
    xcb_flush(conn);
    (*data).major_opcode
}

/// (dx, dy) of an XI_RawMotion event, in device units. Layout (after libxcb inserts `full_sequence` at byte 32):
/// valuator_mask[valuators_len] at 36, then the processed axis values, then the raw ones, as 32.32 fixed point.
unsafe fn raw_motion(e: *const u8) -> [f32; 2] {
    let words = e.add(22).cast::<u16>().read_unaligned() as usize;
    let mask = e.add(36).cast::<u32>();
    let mut set = 0;
    for i in 0..words {
        set += mask.add(i).read_unaligned().count_ones() as usize;
    }
    let raw = e.add(36 + words * 4 + set * 8);
    let (bits, mut d, mut k) = (mask.read_unaligned(), [0.0; 2], 0);
    for (axis, item) in d.iter_mut().enumerate() {
        if bits >> axis & 1 != 0 {
            let v = raw.add(k * 8);
            let (whole, frac) = (
                v.cast::<i32>().read_unaligned(),
                v.add(4).cast::<u32>().read_unaligned(),
            );
            *item = whole as f32 + frac as f32 / 4_294_967_296.0;
            k += 1;
        }
    }
    d
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

/// A 1x1 fully transparent cursor.
unsafe fn blank_cursor(w: &Window) -> u32 {
    let (pix, gc, cur) = (
        xcb_generate_id(w.conn),
        xcb_generate_id(w.conn),
        xcb_generate_id(w.conn),
    );
    xcb_create_pixmap(w.conn, 1, pix, w.win, 1, 1);
    xcb_create_gc(w.conn, gc, pix, 0, ptr::null());
    xcb_put_image(w.conn, 0, pix, gc, 1, 1, 0, 0, 0, 1, 4, [0u8; 4].as_ptr()); // one zeroed 32-bit scanline
    xcb_create_cursor(w.conn, cur, pix, pix, 0, 0, 0, 0, 0, 0, 0, 0); // mask 0 = nothing visible
    cur
}

/// FPS-style mouse: while the window has focus the pointer is hidden and confined to it, and every call returns
/// how far the mouse moved (x right, y down) since the last call. Returns 0 while unfocused or until the window can
/// grab the pointer (it must be mapped). `cx`, `cy` is the window center the pointer is kept near.
/// Motion is raw device units from XInput2 when available; otherwise it is measured by warping the pointer.
pub unsafe fn mouse_delta(w: &Window, cx: i32, cy: i32) -> [f32; 2] {
    let recenter = || {
        xcb_warp_pointer(w.conn, 0, w.win, 0, 0, 0, 0, cx as i16, cy as i16);
        xcb_flush(w.conn);
    };
    if !w.focused.get() {
        w.dx.set(0.0);
        w.dy.set(0.0);
        return [0.0; 2];
    }
    if !w.grabbed.get() {
        if w.blank.get() == 0 {
            w.blank.set(blank_cursor(w));
        }
        let ck = xcb_grab_pointer(w.conn, 1, w.win, 0, 1, 1, w.win, w.blank.get(), 0);
        let r = xcb_grab_pointer_reply(w.conn, ck, ptr::null_mut());
        let ok = !r.is_null() && (*r).status == 0;
        libc::free(r as *mut c_void);
        if ok {
            w.grabbed.set(true);
            w.dx.set(0.0);
            w.dy.set(0.0); // motion from before the grab does not count
            recenter(); // so the pointer starts in the middle (and Wayland compositors treat it as locked)
        }
        return [0.0; 2];
    }
    if w.xi != 0 {
        let d = [w.dx.replace(0.0), w.dy.replace(0.0)];
        if d != [0.0; 2] {
            recenter();
        }
        return d;
    }
    // fallback: ask where the pointer is, then put it back
    let r = xcb_query_pointer_reply(w.conn, xcb_query_pointer(w.conn, w.win), ptr::null_mut());
    if r.is_null() {
        return [0.0; 2];
    }
    let d = [(*r).win_x as i32 - cx, (*r).win_y as i32 - cy];
    libc::free(r as *mut c_void);
    if d != [0, 0] {
        recenter();
    }
    [d[0] as f32, d[1] as f32]
}

/// Drain pending events; false means the user asked to quit. `keys` is kept up to date with the held keys.
pub unsafe fn poll(w: &Window, resized: &mut bool, keys: &mut input::Keys) -> bool {
    let mut running = true;
    // X11 autorepeat sends KeyRelease + KeyPress (same key, same timestamp) while a key is held. A release is held
    // back until the next event shows whether it is one of those pairs, so repeats never count as taps.
    let mut release: Option<(u8, u32)> = None; // (keycode, time)
    loop {
        let e = xcb_poll_for_event(w.conn);
        if e.is_null() {
            break;
        }
        if let Some((code, time)) = release.take() {
            if (*e).response_type & 0x7f == XCB_KEY_PRESS
                && (*e).detail == code
                && (*(e as *const KeyEvent)).time == time
            {
                libc::free(e as *mut c_void);
                continue; // autorepeat: the key never went up
            }
            if let Some(k) = input::from_linux(code) {
                keys.set(k, false);
            }
        }
        match (*e).response_type & 0x7f {
            XCB_CLIENT_MESSAGE => {
                if (*(e as *const ClientMsg)).data[0] == w.wm_delete {
                    running = false;
                }
            }
            XCB_CONFIGURE_NOTIFY => *resized = true,
            XCB_KEY_PRESS if (*e).detail == KEYCODE_ESCAPE => running = false,
            XCB_KEY_PRESS => {
                if let Some(k) = input::from_linux((*e).detail) {
                    keys.set(k, true);
                }
            }
            XCB_KEY_RELEASE => release = Some(((*e).detail, (*(e as *const KeyEvent)).time)),
            XCB_GE_GENERIC => {
                let b = e as *const u8;
                if w.xi != 0 && *b.add(1) == w.xi && b.add(8).cast::<u16>().read_unaligned() == 17 {
                    let d = raw_motion(b);
                    w.dx.set(w.dx.get() + d[0]);
                    w.dy.set(w.dy.get() + d[1]);
                }
            }
            XCB_FOCUS_IN => w.focused.set(true),
            XCB_FOCUS_OUT => {
                // let go of the mouse so other windows can be used
                w.focused.set(false);
                if w.grabbed.replace(false) {
                    xcb_ungrab_pointer(w.conn, 0);
                    xcb_flush(w.conn);
                }
            }
            _ => {}
        }
        libc::free(e as *mut c_void);
    }
    if let Some((code, _)) = release
        && let Some(k) = input::from_linux(code)
    {
        keys.set(k, false);
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
