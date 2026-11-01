//! Window, Vulkan and the render loop, behind a Bevy-ish API, split into cargo features so a game only links
//! what it uses.
//!
//! Game code only touches `Mesh`, `Material`, `Transform`, `Color`, `Camera2d`, `Camera3d`, `Handle`, `Assets`,
//! `Input`, `FixedInput` and `app.run(..)`.
//! Rendering is instanced: every frame all drawable entities go into one instance buffer, one indirect draw per
//! (mesh, material) pair draws them into an offscreen target (with depth), and a fullscreen post-process pass
//! (`PostProcess`) then writes the swapchain.
//!
//! | feature       | adds                                                                                  | without it |
//! |---------------|---------------------------------------------------------------------------------------|------------|
//! | `cam2d`       | `Camera2d`, `Material::flat`, `DEPTH2D`                                               | |
//! | `cam3d`       | `Camera3d`                                                                            | (enable at least one camera feature) |
//! | `cull`        | GPU frustum culling (`cull.wgsl`)                                                     | every instance is drawn |
//! | `multicam`    | several cameras with `viewport` and `scale`                                           | one fullscreen camera, drawn straight into the output |
//! | `post`        | `PostProcess` and the offscreen scene + fullscreen pass                               | cameras write the swapchain directly |
//! | `mouse`       | `MouseCapture`, `Input::{mouse_delta, cursor, cursor_world, mouse_down, clicked}`     | keyboard only |
//! | `fixed_input` | `FixedInput`                                                                          | `Input` only |
//! | `lit`         | `Vertex`, `Material::lit`                                                             | bring your own `Material` |
//! | `aa`          | 4x MSAA of the scene pass (3D edges and `Material::msdf` text)                      | jagged edges |
//! | `text`        | `Atlas`, `Material::msdf` (draws the `font` crate's text meshes)                      | no texture support at all |
#![no_std]
#![allow(unsafe_op_in_unsafe_fn)]

extern crate alloc;

pub use ash::vk;

#[cfg(not(any(feature = "cam2d", feature = "cam3d")))]
compile_error!("enable at least one of the features `cam2d` and `cam3d`");

// always compiled
mod assets;
mod cams;
mod device;
mod gpu;
mod input;
mod renderer;
mod run;
mod scene;
mod swapchain;
mod transform;

// one module per optional feature
#[cfg(feature = "cam2d")]
mod cam2d;
#[cfg(feature = "cam3d")]
mod cam3d;
#[cfg(feature = "fixed_input")]
mod fixed_input;
#[cfg(feature = "lit")]
mod lit;
#[cfg(feature = "mouse")]
mod mouse;

// features that change how a frame is drawn: same module name and interface, the `_off` file is the cheap version
#[cfg_attr(feature = "cull", path = "cull.rs")]
#[cfg_attr(not(feature = "cull"), path = "cull_off.rs")]
mod cull;
#[cfg_attr(feature = "multicam", path = "multicam.rs")]
#[cfg_attr(not(feature = "multicam"), path = "single.rs")]
mod multicam;
#[cfg_attr(feature = "post", path = "post.rs")]
#[cfg_attr(not(feature = "post"), path = "post_off.rs")]
mod post;
#[cfg_attr(feature = "text", path = "text.rs")]
#[cfg_attr(not(feature = "text"), path = "text_off.rs")]
mod text;

pub use assets::{Assets, Handle, Material, Mesh};
pub use cams::MAX_CAMERAS;
pub use input::Input;
pub use run::RunExt;
pub use transform::{Color, Transform};

#[cfg(feature = "cam2d")]
pub use cam2d::{Camera2d, DEPTH2D};
#[cfg(feature = "cam3d")]
pub use cam3d::Camera3d;
#[cfg(feature = "fixed_input")]
pub use fixed_input::FixedInput;
#[cfg(feature = "lit")]
pub use lit::Vertex;
#[cfg(feature = "mouse")]
pub use mouse::MouseCapture;
#[cfg(feature = "post")]
pub use post::PostProcess;
#[cfg(feature = "text")]
pub use text::Atlas;
