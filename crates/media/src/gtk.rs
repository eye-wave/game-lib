//! Linux backend: a hidden WebKitGTK view. Raw FFI, no crates. Needs a display (xvfb is fine), libgtk-3 and
//! libwebkit2gtk-4.1 at build and run time. Create and pump it on the thread that owns GTK (the main thread).
use crate::web::{self, Inbox};
use alloc::{boxed::Box, vec::Vec};
use core::ffi::{CStr, c_char, c_int, c_ulong, c_void};
use core::ptr::{null, null_mut};

type P = *mut c_void;

#[link(name = "glib-2.0")]
unsafe extern "C" {
    fn g_free(p: P);
}
#[link(name = "gobject-2.0")]
unsafe extern "C" {
    fn g_signal_connect_data(
        i: P,
        sig: *const c_char,
        cb: *const c_void,
        data: P,
        destroy: *const c_void,
        flags: c_int,
    ) -> c_ulong;
    fn g_object_unref(o: P);
}
#[link(name = "gtk-3")]
unsafe extern "C" {
    fn gtk_init_check(argc: *mut c_int, argv: *mut *mut *mut c_char) -> c_int;
    fn gtk_offscreen_window_new() -> P;
    fn gtk_container_add(container: P, widget: P);
    fn gtk_widget_show_all(w: P);
    fn gtk_widget_destroy(w: P);
    fn gtk_events_pending() -> c_int;
    fn gtk_main_iteration_do(blocking: c_int) -> c_int;
}
#[link(name = "webkit2gtk-4.1")]
unsafe extern "C" {
    fn webkit_user_content_manager_new() -> P;
    fn webkit_user_content_manager_register_script_message_handler(
        m: P,
        name: *const c_char,
    ) -> c_int;
    fn webkit_web_view_new_with_user_content_manager(m: P) -> P;
    fn webkit_web_view_load_html(v: P, html: *const c_char, base: *const c_char);
    fn webkit_web_view_evaluate_javascript(
        v: P,
        script: *const c_char,
        len: isize,
        world: *const c_char,
        uri: *const c_char,
        cancel: P,
        done: *const c_void,
        data: P,
    );
    fn webkit_javascript_result_get_js_value(r: P) -> P;
}
#[link(name = "javascriptcoregtk-4.1")]
unsafe extern "C" {
    fn jsc_value_to_string(v: P) -> *mut c_char;
}
unsafe extern "C" {
    fn setenv(k: *const c_char, v: *const c_char, overwrite: c_int) -> c_int;
}

pub struct View {
    pub inbox: Box<Inbox>, // boxed: the signal handler keeps a pointer to it
    win: P,
    view: P,
}

impl View {
    /// `None` = no display.
    pub fn new() -> Option<Self> {
        unsafe {
            // We never draw: don't make GDK create a GL context.
            setenv(
                c"WEBKIT_DISABLE_COMPOSITING_MODE".as_ptr(),
                c"1".as_ptr(),
                1,
            );
            setenv(c"WEBKIT_DISABLE_DMABUF_RENDERER".as_ptr(), c"1".as_ptr(), 1);
            if gtk_init_check(null_mut(), null_mut()) == 0 {
                return None;
            }
            let mut inbox = Box::new(Inbox::new());
            let ucm = webkit_user_content_manager_new();
            webkit_user_content_manager_register_script_message_handler(ucm, c"ipc".as_ptr());
            g_signal_connect_data(
                ucm,
                c"script-message-received::ipc".as_ptr(),
                on_message as *const () as *const c_void,
                &mut *inbox as *mut Inbox as P,
                null(),
                0,
            );
            let view = webkit_web_view_new_with_user_content_manager(ucm);
            g_object_unref(ucm);
            let win = gtk_offscreen_window_new();
            gtk_container_add(win, view);
            gtk_widget_show_all(win);
            let mut html = Vec::new();
            for s in [web::POST, &web::page(), web::END] {
                html.extend_from_slice(s.as_bytes());
            }
            html.push(0);
            webkit_web_view_load_html(view, html.as_ptr().cast(), null());
            Some(Self { inbox, win, view })
        }
    }

    /// Run whatever is ready (replies land in `inbox`); never blocks.
    pub fn pump(&mut self) {
        unsafe {
            while gtk_events_pending() != 0 {
                gtk_main_iteration_do(0);
            }
        }
    }

    /// Start decoding `payload` (yEnc byte values); the answer comes back through `inbox`.
    pub fn run(&mut self, id: usize, kind: char, payload: &[u8]) {
        let mut js = Vec::with_capacity(payload.len() * 3 / 2 + 32);
        js.extend_from_slice(alloc::format!("go({id},\"{kind}\",\"").as_bytes());
        for &c in payload {
            push_utf8(c, &mut js);
        }
        js.extend_from_slice(b"\")");
        unsafe {
            webkit_web_view_evaluate_javascript(
                self.view,
                js.as_ptr().cast(),
                js.len() as isize,
                null(),
                null(),
                null_mut(),
                null(),
                null_mut(),
            )
        }
    }
}

impl Drop for View {
    fn drop(&mut self) {
        unsafe { gtk_widget_destroy(self.win) }
    }
}

unsafe extern "C" fn on_message(_ucm: P, result: P, inbox: P) {
    unsafe {
        let s = jsc_value_to_string(webkit_javascript_result_get_js_value(result));
        if !s.is_null() {
            (*(inbox as *mut Inbox)).push(&from_utf8(CStr::from_ptr(s).to_bytes()));
            g_free(s as P);
        }
    }
}

/// The page's byte values travel as UTF-8 text (code points 0..=255): one byte below 0x80, two above.
fn push_utf8(c: u8, out: &mut Vec<u8>) {
    if c < 0x80 {
        out.push(c);
    } else {
        out.push(0xC0 | c >> 6);
        out.push(0x80 | c & 0x3F);
    }
}

fn from_utf8(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let mut c = s[i];
        if c >= 0xC0 {
            i += 1;
            c = c << 6 | s.get(i).copied().unwrap_or(0) & 0x3F;
        }
        i += 1;
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_roundtrip() {
        let all: Vec<u8> = (0..=255).collect();
        let mut t = Vec::new();
        all.iter().for_each(|&c| push_utf8(c, &mut t));
        assert_eq!(from_utf8(&t), all);
    }
}
