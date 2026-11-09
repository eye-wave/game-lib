//! Linux backend: a hidden WebKitGTK view. Raw FFI, no crates. Needs a display (xvfb is fine), libgtk-3 and
//! libwebkit2gtk-4.1 (2.40 or newer: it reads POST bodies of custom schemes) at build and run time. Create and
//! pump it on the thread that owns GTK (the main thread).
//!
//! The page lives at `app://m/` (a custom scheme, registered on the view's own context, as CORS-enabled and secure
//! so its `fetch`es are allowed; that context is ephemeral, so nothing is written to disk): `GET app://m/a/<name>` is answered from the assets directory, `POST app://m/r/...` hands the raw
//! result bytes to the `Inbox`. See web.rs.
use crate::web::{self, Inbox};
use alloc::{boxed::Box, vec::Vec};
use core::ffi::{CStr, c_char, c_int, c_uint, c_ulong, c_void};
use core::ptr::{null, null_mut};
use core::sync::atomic::{AtomicPtr, Ordering::Relaxed};

type P = *mut c_void;

#[link(name = "glib-2.0")]
unsafe extern "C" {
    fn g_free(p: P);
    fn g_malloc(n: usize) -> P;
}
#[link(name = "gio-2.0")]
unsafe extern "C" {
    fn g_memory_input_stream_new() -> P;
    fn g_memory_input_stream_new_from_data(data: P, len: isize, destroy: *const c_void) -> P;
    fn g_input_stream_read_all(
        stream: P,
        buf: *mut c_void,
        count: usize,
        read: *mut usize,
        cancellable: P,
        err: *mut P,
    ) -> c_int;
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
    fn webkit_user_content_manager_register_script_message_handler(
        m: P,
        name: *const c_char,
    ) -> c_int;
    fn webkit_web_view_new_with_context(ctx: P) -> P;
    fn webkit_web_view_get_user_content_manager(v: P) -> P; // not owned
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
    /// A context that writes nothing to disk: no profile folder, per app or shared.
    fn webkit_web_context_new_ephemeral() -> P;
    fn webkit_web_context_register_uri_scheme(
        ctx: P,
        scheme: *const c_char,
        cb: *const c_void,
        data: P,
        destroy: *const c_void,
    );
    fn webkit_web_context_get_security_manager(ctx: P) -> P;
    fn webkit_security_manager_register_uri_scheme_as_cors_enabled(m: P, scheme: *const c_char);
    fn webkit_security_manager_register_uri_scheme_as_secure(m: P, scheme: *const c_char);
    fn webkit_uri_scheme_request_get_uri(req: P) -> *const c_char;
    fn webkit_uri_scheme_request_get_http_body(req: P) -> P; // transfer full, nullable
    fn webkit_uri_scheme_request_finish_with_response(req: P, resp: P);
    fn webkit_uri_scheme_response_new(stream: P, len: i64) -> P;
    fn webkit_uri_scheme_response_set_status(resp: P, code: c_uint, reason: *const c_char);
    fn webkit_uri_scheme_response_set_content_type(resp: P, ct: *const c_char);
}
#[link(name = "javascriptcoregtk-4.1")]
unsafe extern "C" {
    fn jsc_value_to_string(v: P) -> *mut c_char;
}
unsafe extern "C" {
    fn setenv(k: *const c_char, v: *const c_char, overwrite: c_int) -> c_int;
}

/// Where `on_scheme` files the page's replies: the live view's inbox. Null while no view exists, so a request that
/// is still in flight when the view goes away cannot reach a freed inbox.
static INBOX: AtomicPtr<Inbox> = AtomicPtr::new(null_mut());

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
            INBOX.store(&mut *inbox, Relaxed);
            let ctx = webkit_web_context_new_ephemeral();
            webkit_web_context_register_uri_scheme(
                ctx,
                c"app".as_ptr(),
                on_scheme as *const () as *const c_void,
                null_mut(),
                null(),
            );
            let sm = webkit_web_context_get_security_manager(ctx);
            webkit_security_manager_register_uri_scheme_as_cors_enabled(sm, c"app".as_ptr());
            webkit_security_manager_register_uri_scheme_as_secure(sm, c"app".as_ptr());
            let view = webkit_web_view_new_with_context(ctx);
            g_object_unref(ctx); // the view keeps it alive
            let ucm = webkit_web_view_get_user_content_manager(view);
            webkit_user_content_manager_register_script_message_handler(ucm, c"ipc".as_ptr());
            g_signal_connect_data(
                ucm,
                c"script-message-received::ipc".as_ptr(),
                on_message as *const () as *const c_void,
                &mut *inbox as *mut Inbox as P,
                null(),
                0,
            );
            let win = gtk_offscreen_window_new();
            gtk_container_add(win, view);
            gtk_widget_show_all(win);
            let mut html = Vec::new();
            for s in [web::POST, &web::page(), web::END] {
                html.extend_from_slice(s.as_bytes());
            }
            html.push(0);

            // base URI = the page's origin, so its fetches to `app://m/` are same-origin
            webkit_web_view_load_html(view, html.as_ptr().cast(), c"app://m/".as_ptr());
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

    /// Start decoding asset `name` (`kind` as in `lib.rs`; `name` and `head` already `web::escape`d, so plain ASCII
    /// that is safe in a JS string); the page fetches the file and the answer comes back through `inbox`.
    pub fn run(&mut self, id: usize, kind: char, name: &str, head: &str) {
        let js = alloc::format!("g({id},\"{kind}\",\"{name}\",\"{head}\")");
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
        INBOX.store(null_mut(), Relaxed);
        unsafe { gtk_widget_destroy(self.win) }
    }
}

unsafe extern "C" fn on_message(_ucm: P, result: P, inbox: P) {
    unsafe {
        let s = jsc_value_to_string(webkit_javascript_result_get_js_value(result));
        if !s.is_null() {
            (*(inbox as *mut Inbox)).push(CStr::from_ptr(s).to_bytes());
            g_free(s as P);
        }
    }
}

/// Every request of the page to `app://m/`: `a/<name>` gets the asset (404 if there is none), `r/...` carries a
/// result in its body. Both are answered with a response; replies get an empty 200.
unsafe extern "C" fn on_scheme(req: P, _data: P) {
    unsafe {
        let uri = CStr::from_ptr(webkit_uri_scheme_request_get_uri(req)).to_bytes();
        let path = uri.strip_prefix(web::BASE.as_bytes()).unwrap_or(&[]);
        let mut asset = None;
        if let Some(name) = path.strip_prefix(b"a/") {
            asset = core::str::from_utf8(&web::unescape(name))
                .ok()
                .and_then(platform::read_asset);
        } else if path.starts_with(b"r/") {
            let body = webkit_uri_scheme_request_get_http_body(req);
            let mut data = Vec::new();
            if !body.is_null() {
                let mut buf = [0u8; 1 << 16];
                loop {
                    let mut n = 0;
                    let ok = g_input_stream_read_all(
                        body,
                        buf.as_mut_ptr().cast(),
                        buf.len(),
                        &mut n,
                        null_mut(),
                        null_mut(),
                    );
                    data.extend_from_slice(&buf[..n]);
                    if ok == 0 || n < buf.len() {
                        break;
                    }
                }
                g_object_unref(body);
            }
            let inbox = INBOX.load(Relaxed);
            if !inbox.is_null() {
                (*inbox).push_reply(path, &data);
            }
        }
        let is_asset = path.starts_with(b"a/");
        let (stream, len) = match &asset {
            Some(d) => {
                let copy = g_malloc(d.len().max(1)); // the stream frees it
                core::ptr::copy_nonoverlapping(d.as_ptr(), copy.cast(), d.len());
                let free = g_free as *const () as *const c_void;
                (
                    g_memory_input_stream_new_from_data(copy, d.len() as isize, free),
                    d.len(),
                )
            }
            None => (g_memory_input_stream_new(), 0),
        };
        let resp = webkit_uri_scheme_response_new(stream, len as i64);
        webkit_uri_scheme_response_set_status(
            resp,
            if is_asset && asset.is_none() {
                404
            } else {
                200
            },
            null(),
        );
        webkit_uri_scheme_response_set_content_type(resp, c"application/octet-stream".as_ptr());
        webkit_uri_scheme_request_finish_with_response(req, resp);
        g_object_unref(resp);
        g_object_unref(stream);
    }
}
