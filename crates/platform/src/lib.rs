//! Same API on every OS: panic handler, allocator, `die`, window, surface, event poll, `entry!`.
#![no_std]
#![allow(unsafe_op_in_unsafe_fn, clippy::missing_safety_doc)]

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

unsafe fn surface_ck(r: vk::Result, s: vk::SurfaceKHR) -> vk::SurfaceKHR {
    if r != vk::Result::SUCCESS {
        die(CREATE_SURFACE_FAILED)
    }
    s
}
