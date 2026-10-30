#![no_std]

#[macro_use]
mod vec;
mod sys;
mod vec2;
mod vec3;

pub use sys::FloatExt;
pub use vec2::Vec2;
pub use vec3::Vec3;
