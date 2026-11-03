//! Feature `lit`: the `Vertex` type and `Material::lit` for position/normal/color meshes.
use crate::assets::Material;
use ash::vk;
use math::Vec3;
use shader::Align4;

/// Position, normal and color: the vertex of `Material::lit`.
#[repr(C)]
pub struct Vertex {
    pub pos: Vec3,
    pub normal: Vec3,
    pub color: Vec3,
}

impl Material {
    /// For `Vertex`: location 0 = pos, 1 = normal, 2 = color.
    pub const fn lit(shader: &'static Align4<[u8]>) -> Self {
        Self::new(
            shader,
            size_of::<Vertex>() as u32,
            &[vk::Format::R32G32B32_SFLOAT; 3],
        )
    }
}
