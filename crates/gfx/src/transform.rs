//! `Transform` and `Color`: the per-entity components every drawable carries.
use math::Vec3;

/// World position (y up), rotation about the y axis in radians, and per-axis scale.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Transform {
    pub pos: Vec3,
    pub yaw: f32,
    pub scale: Vec3,
}
impl Default for Transform {
    fn default() -> Self {
        Self::from_pos(0.0, 0.0, 0.0)
    }
}
impl Transform {
    pub const fn from_pos(x: f32, y: f32, z: f32) -> Self {
        Self {
            pos: Vec3::new(x, y, z),
            yaw: 0.0,
            scale: Vec3::splat(1.0),
        }
    }
}

/// Optional tint multiplied into the mesh's vertex colors (white when absent).
#[derive(Clone, Copy)]
pub struct Color(pub [f32; 3]);
