//! Same API on every OS: panic handler, allocator, `die`, window, surface, event poll, `entry!`.
#![no_std]
#![allow(unsafe_op_in_unsafe_fn, clippy::missing_safety_doc)]

extern crate alloc;

use alloc::{vec, vec::Vec};
use ash::{khr, vk};
use core::{ffi::CStr, ptr};
use errors::prelude::*;
use vulkan_sys::*;

pub mod input;

/// Debug builds only: id of the one thread that must not touch the heap (0 = none). The audio thread sets it,
/// and the global allocator then aborts with `ALLOC_IN_AUDIO_THREAD` if that thread allocates or frees.
#[cfg(debug_assertions)]
pub static NO_ALLOC: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

#[cfg(debug_assertions)]
fn rt_check() {
    let n = NO_ALLOC.load(core::sync::atomic::Ordering::Relaxed);
    if n != 0 && n == thread_id() {
        die(ALLOC_IN_AUDIO_THREAD)
    }
}
#[cfg(not(debug_assertions))]
#[inline(always)]
fn rt_check() {}

#[cfg_attr(unix, path = "linux.rs")]
#[cfg_attr(windows, path = "windows.rs")]
mod sys;
pub use sys::*;

/// What a non-blocking `Child::read` found.
pub enum Read {
    Data(usize),
    /// Nothing right now; the process is still running.
    Empty,
    /// The process closed its stdout: call `Child::wait`.
    Eof,
}

/// NUL-terminated path of asset `name`: `<workspace>/assets` in debug builds (found from the source tree, so any
/// working directory does), `assets/` next to the executable in release builds (a bundle is the executables plus
/// one `assets/` beside them).
pub fn asset_path(name: &str) -> Vec<u8> {
    let mut p = asset_dir();
    p.extend_from_slice(name.as_bytes());
    p.push(0);
    p
}

/// Whole file from the assets directory, or `None`.
pub fn read_asset(name: &str) -> Option<Vec<u8>> {
    read_file(CStr::from_bytes_with_nul(&asset_path(name)).ok()?)
}

#[cfg(debug_assertions)]
fn asset_dir() -> Vec<u8> {
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/")
        .as_bytes()
        .to_vec()
}

#[cfg(not(debug_assertions))]
fn asset_dir() -> Vec<u8> {
    let mut p = exe_path();
    while p.last().is_some_and(|&c| c != b'/' && c != b'\\') {
        p.pop(); // keep the directory, with its separator; if the path is unknown, `assets/` is relative to the cwd
    }
    p.extend_from_slice(b"assets/"); // Windows takes '/' too
    p
}

unsafe fn surface_ck(r: vk::Result, s: vk::SurfaceKHR) -> vk::SurfaceKHR {
    if r != vk::Result::SUCCESS {
        die(CREATE_SURFACE_FAILED)
    }
    s
}
