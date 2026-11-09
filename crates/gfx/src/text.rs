//! Feature `text`: `Atlas` (the glyph texture) and `Material::msdf`. `textures.rs` uploads the atlas and binds it.
use crate::assets::Material;
use ash::vk;
use shader::Align4;

pub(crate) static MSDF: &Align4<[u8]> = shader::include_shader!("msdf");

/// Resource: the glyph atlas of the `font` crate (`Font::{w, h, rgb}`, or `font::alpha_atlas` for a bitmap font),
/// RGB8, top row first. Insert it before the
/// first frame that draws a `Material::msdf()`; it is uploaded once and later changes are ignored.
pub struct Atlas {
    pub w: u32,
    pub h: u32,
    pub rgb: &'static [u8],
    /// Sample without filtering: for bitmap fonts (`font::alpha_atlas`), whose 1 px strokes and counters the linear
    /// filter smears (and drivers round differently). Leave `false` for MSDF, which needs the interpolation.
    pub nearest: bool,
}

impl Material {
    /// Text: the mesh is `font::Font::layout`'s `[x, y, z, u, v]` vertices, filled with the entity's `Color`. Same
    /// transform rules as `Material::flat` (so it pairs with `Camera2d`; z is the layer). Needs the `Atlas` resource.
    /// Edges are one pixel wide in alpha (`fwidth`), which feature `aa` turns into MSAA coverage; without `aa` they
    /// are cut at the 0.5 contour, crisp but jagged.
    pub fn msdf() -> Self {
        Self::new(
            MSDF,
            size_of::<[f32; 5]>() as u32,
            &[vk::Format::R32G32B32_SFLOAT, vk::Format::R32G32_SFLOAT],
        )
    }
}
