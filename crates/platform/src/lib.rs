//! Same API on every OS: panic handler, allocator, `die`, window, surface, event poll, `entry!`.
#![no_std]
#![allow(unsafe_op_in_unsafe_fn, clippy::missing_safety_doc)]

use ash::{khr, vk};
use core::{ffi::CStr, ptr};
use vulkan_sys::*;

#[cfg_attr(unix, path = "linux.rs")]
#[cfg_attr(windows, path = "windows.rs")]
mod sys;
pub use sys::*;

unsafe fn surface_ck(r: vk::Result, s: vk::SurfaceKHR) -> vk::SurfaceKHR {
    if r != vk::Result::SUCCESS {
        die(c"create_surface failed")
    }
    s
}
