//! RPG Maker style tiles in two layers.
//!
//! Ground: an A2 floor autotile sheet (8 x 4 kinds, kind = ground byte 0..31, each 2 x 3 tiles of 48 px made of 24 px
//! subtiles). The shader picks the subtiles from `FLOOR_AUTOTILE_TABLE` by looking at each cell's 8 neighbours (see
//! `rpgmaker.wgsl`). Kind 0 is drawn under every cell, so the see-through parts of bushes, fences... show it.
//!
//! Objects: a sheet of 16 x 16 cells of 48 px (RPG Maker's B sheet) where each cell is its own thing, drawn over the
//! ground with its alpha. Things bigger than a cell are not supported yet.
use crate::Level;
use alloc::vec::Vec;
use ecs::World;
use gfx::{Assets, Handle, Material, TexFormat, Texture, vk};
use shader::Align4;

static SHADER: &Align4<[u8]> = shader::include_shader!("rpgmaker");

/// An RGBA8 image, top row first (`media::Image` has the same fields).
pub struct Sheet {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// Registers a sheet as a texture: a 768 x 576 A2 sheet for the ground, a 768 x 768 B sheet for the objects. Do it once
/// per image file and share the handle between levels, so it is uploaded once.
pub fn sheet(w: &mut World, s: Sheet) -> Handle<Texture> {
    w.resource_mut::<Assets<Texture>>().unwrap().add(Texture {
        w: s.w,
        h: s.h,
        format: TexFormat::Rgba8,
        px: s.rgba,
    })
}

/// The material for `level` drawn with the sheets `level.ground_sheet` and `level.objects_sheet` (as handles from
/// `sheet`), uploaded the first frame it is drawn. Use it with `spawn` / `set`.
pub fn material(
    w: &mut World,
    level: &Level,
    ground: Handle<Texture>,
    objects: Handle<Texture>,
) -> Handle<Material> {
    let tex = w.resource_mut::<Assets<Texture>>().unwrap();
    let mut data = |px: &[u8]| {
        tex.add(Texture {
            w: level.w,
            h: level.h,
            format: TexFormat::R8,
            px: px.to_vec(),
        })
    };
    let (cells, props) = (data(&level.cells), data(&level.objects));
    w.resource_mut::<Assets<Material>>().unwrap().add(
        Material::new(
            SHADER,
            size_of::<[f32; 3]>() as u32,
            &[vk::Format::R32G32B32_SFLOAT],
        )
        // bindings 0..3 in rpgmaker.wgsl
        .with_textures(&[ground, cells, objects, props]),
    )
}
