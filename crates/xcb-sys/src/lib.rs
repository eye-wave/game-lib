#![no_std]
#![cfg(unix)]

use core::ffi::{c_char, c_int, c_void};

pub type Conn = c_void;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Cookie {
    pub sequence: u32,
}
#[repr(C)]
pub struct Screen {
    pub root: u32,
}
#[repr(C)]
pub struct ScreenIter {
    pub data: *const Screen,
    pub rem: c_int,
    pub index: c_int,
}
#[repr(C)]
pub struct AtomReply {
    pub response_type: u8,
    pub pad0: u8,
    pub sequence: u16,
    pub length: u32,
    pub atom: u32,
}
#[repr(C)]
pub struct Event {
    pub response_type: u8,
    pub detail: u8,
    pub sequence: u16,
}
#[repr(C)]
pub struct MotionEvent {
    pub response_type: u8,
    pub detail: u8,
    pub sequence: u16,
    pub time: u32,
    pub root: u32,
    pub event: u32,
    pub child: u32,
    pub root_x: i16,
    pub root_y: i16,
    pub event_x: i16, // relative to the window
    pub event_y: i16,
}
#[repr(C)]
pub struct ClientMsg {
    pub response_type: u8,
    pub format: u8,
    pub sequence: u16,
    pub window: u32,
    pub type_: u32,
    pub data: [u32; 5],
}

#[link(name = "libxcb.so.1", kind = "dylib", modifiers = "+verbatim")]
unsafe extern "C" {
    pub fn xcb_connect(display: *const c_char, screen: *mut c_int) -> *mut Conn;
    pub fn xcb_connection_has_error(c: *mut Conn) -> c_int;
    pub fn xcb_get_setup(c: *mut Conn) -> *const c_void;
    pub fn xcb_setup_roots_iterator(setup: *const c_void) -> ScreenIter;
    pub fn xcb_generate_id(c: *mut Conn) -> u32;
    pub fn xcb_create_window(
        c: *mut Conn,
        depth: u8,
        wid: u32,
        parent: u32,
        x: i16,
        y: i16,
        width: u16,
        height: u16,
        border_width: u16,
        class: u16,
        visual: u32,
        value_mask: u32,
        value_list: *const c_void,
    ) -> Cookie;
    pub fn xcb_change_property(
        c: *mut Conn,
        mode: u8,
        window: u32,
        property: u32,
        type_: u32,
        format: u8,
        data_len: u32,
        data: *const c_void,
    ) -> Cookie;
    pub fn xcb_intern_atom(
        c: *mut Conn,
        only_if_exists: u8,
        len: u16,
        name: *const c_char,
    ) -> Cookie;
    pub fn xcb_intern_atom_reply(
        c: *mut Conn,
        cookie: Cookie,
        err: *mut *mut c_void,
    ) -> *mut AtomReply;
    pub fn xcb_map_window(c: *mut Conn, window: u32) -> Cookie;
    pub fn xcb_warp_pointer(
        c: *mut Conn,
        src_window: u32,
        dst_window: u32,
        src_x: i16,
        src_y: i16,
        src_w: u16,
        src_h: u16,
        dst_x: i16,
        dst_y: i16,
    ) -> Cookie;
    pub fn xcb_flush(c: *mut Conn) -> c_int;
    pub fn xcb_poll_for_event(c: *mut Conn) -> *mut Event;
}

pub const XCB_CW_EVENT_MASK: u32 = 0x800;
pub const XCB_EVENT_MASK_KEY_PRESS: u32 = 1;
pub const XCB_EVENT_MASK_KEY_RELEASE: u32 = 2;
pub const XCB_EVENT_MASK_POINTER_MOTION: u32 = 1 << 6;
pub const XCB_EVENT_MASK_STRUCTURE_NOTIFY: u32 = 1 << 17;
pub const XCB_KEY_PRESS: u8 = 2;
pub const XCB_KEY_RELEASE: u8 = 3;
pub const XCB_MOTION_NOTIFY: u8 = 6;
pub const XCB_CONFIGURE_NOTIFY: u8 = 22;
pub const XCB_CLIENT_MESSAGE: u8 = 33;
pub const XCB_ATOM_ATOM: u32 = 4;
pub const XCB_ATOM_STRING: u32 = 31;
pub const XCB_ATOM_WM_NAME: u32 = 39;
pub const KEYCODE_ESCAPE: u8 = 9;
