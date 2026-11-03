//! Turns a `Level` into things gfx draws: a quad per tile layer (a material with the sheet and the layer's cells as
//! textures), an instance per sprite. `prepare` creates the GPU resources, once per level; `spawn` creates the
//! entities, as often as the level is entered.
use crate::{Kind, Level};
use alloc::{string::String, vec::Vec};
use ecs::{Entity, World};
use errors::prelude::*;
use gfx::{Assets, Color, Handle, Material, Mesh, TexFormat, Texture, Transform, vk};
use math::{Vec2, Vec3};
use platform::die;
use shader::Align4;

static AUTOTILE: &Align4<[u8]> = shader::include_shader!("autotile");
static TILES: &Align4<[u8]> = shader::include_shader!("tiles");
static SPRITES: &Align4<[u8]> = shader::include_shader!("sprites");

// z bands: tile layers below the sprites (z = layer index * LAYER_STEP, so at most 100 layers), the sprites and
// anything that walks among them (`depth`), then the tile layers listed after a sprites layer. Hud and such go above.
const LAYER_STEP: f32 = 0.01;
const SORT_Z: f32 = 2.0;
const SORT_STEP: f32 = 0.01; // per world unit of y: maps up to 100 rows tall sort correctly
const OVER_Z: f32 = 3.0;

/// The z for something standing at world y `y` (its feet, or a sprite's bottom edge) among the sprites: the lower on
/// the screen, the more in front. Give the parts of one figure small offsets (a thousandth) on top of this.
pub fn depth(y: f32) -> f32 {
    SORT_Z - y * SORT_STEP
}

/// An RGBA8 image, top row first (`media::Image` has the same fields).
pub struct Sheet {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// Resource the game inserts: how to load an image file of the assets directory by name. A level's image files are
/// loaded through it once each, however many levels and layers use them.
pub struct Sheets {
    load: fn(&str) -> Sheet,
    cache: Vec<(String, Handle<Texture>)>,
}

impl Sheets {
    pub fn new(load: fn(&str) -> Sheet) -> Self {
        Self {
            load,
            cache: Vec::new(),
        }
    }
}

fn sheet(w: &mut World, name: &str) -> Handle<Texture> {
    let sheets = w.resource::<Sheets>().unwrap();
    if let Some(s) = sheets.cache.iter().find(|s| s.0 == name) {
        return s.1;
    }
    let s = (sheets.load)(name);
    let t = w.resource_mut::<Assets<Texture>>().unwrap().add(Texture {
        w: s.w,
        h: s.h,
        format: TexFormat::Rgba8,
        px: s.rgba,
    });
    w.resource_mut::<Sheets>()
        .unwrap()
        .cache
        .push((String::from(name), t));
    t
}

struct SpriteDraw {
    material: Handle<Material>,
    pos: Vec3,
    scale: Vec3,
    src: [f32; 3], // the sheet cell and fade, as the shader wants them in `Color`
}

/// A level's GPU side: materials, and where everything goes. Not tied to a `World` borrow, so it can sit in a
/// resource and be handed to `spawn`.
pub struct Prepared {
    size: (u32, u32),
    layers: Vec<(Handle<Material>, f32)>, // material, z
    sprites: Vec<SpriteDraw>,
}

fn make_material(
    w: &mut World,
    shader: &'static Align4<[u8]>,
    textures: &[Handle<Texture>],
) -> Handle<Material> {
    let m = Material::new(
        shader,
        size_of::<[f32; 3]>() as u32,
        &[vk::Format::R32G32B32_SFLOAT],
    );
    w.resource_mut::<Assets<Material>>()
        .unwrap()
        .add(m.with_textures(textures))
}

/// Loads the level's image files (through `Sheets`), uploads its layers' cells and builds the materials. The
/// textures reach the GPU the first frame they are drawn.
pub fn prepare(w: &mut World, level: &Level) -> Prepared {
    let sheets: Vec<_> = level.tilesets.iter().map(|t| sheet(w, &t.name)).collect();
    let mut layers = Vec::new();
    for (i, l) in level.layers.iter().enumerate() {
        let shader = match level.tilesets[l.tileset as usize].kind {
            Kind::Autotile => AUTOTILE,
            _ => TILES, // `Level::parse` guarantees a tile layer is Autotile or Tiles
        };
        let cells = w.resource_mut::<Assets<Texture>>().unwrap().add(Texture {
            w: level.w,
            h: level.h,
            format: TexFormat::R8,
            px: l.cells.clone(),
        });
        let m = make_material(w, shader, &[sheets[l.tileset as usize], cells]);
        layers.push((m, i as f32 * LAYER_STEP + if l.over { OVER_Z } else { 0.0 }));
    }
    let mut mats: Vec<(u32, Handle<Material>)> = Vec::new(); // one material per sprite sheet
    let mut sprites = Vec::new();
    for s in &level.sprites {
        let material = match mats.iter().find(|m| m.0 == s.tileset) {
            Some(m) => m.1,
            None => {
                let m = make_material(w, SPRITES, &[sheets[s.tileset as usize]]);
                mats.push((s.tileset, m));
                m
            }
        };
        let base = (level.h - 1 - s.y) as f32; // world y of its bottom edge; world y is up, rows go down
        let (sw, sh) = (s.w as f32, s.h as f32);
        sprites.push(SpriteDraw {
            material,
            pos: Vec3::new(s.x as f32 + sw / 2.0, base + sh / 2.0, depth(base)),
            scale: Vec3::new(sw, sh, 1.0),
            src: [s.col as f32, s.row as f32, 0.0],
        });
    }
    Prepared {
        size: (level.w, level.h),
        layers,
        sprites,
    }
}

struct Quad(Handle<Mesh>); // the one unit square everything uses

/// Spawns the level: a quad per tile layer and an entity per sprite. A cell is one world unit, the map's bottom left
/// corner is the origin and its top row is at the top (world y up). Pairs with `Camera2d`. Despawn the returned
/// entities to leave the level.
pub fn spawn(w: &mut World, p: &Prepared) -> Vec<Entity> {
    if w.resource::<Quad>().is_none() {
        let v = Vec3::new;
        let quad = w.resource_mut::<Assets<Mesh>>().unwrap().add(Mesh::new(&[
            v(-0.5, -0.5, 0.0),
            v(0.5, -0.5, 0.0),
            v(0.5, 0.5, 0.0),
            v(-0.5, -0.5, 0.0),
            v(0.5, 0.5, 0.0),
            v(-0.5, 0.5, 0.0),
        ]));
        w.insert_resource(Quad(quad));
    }
    let quad = w.resource::<Quad>().unwrap().0;
    let (mw, mh) = (p.size.0 as f32, p.size.1 as f32);
    let mut out = Vec::new();
    for &(material, z) in &p.layers {
        let t = Transform {
            scale: Vec3::new(mw, mh, 1.0),
            ..Transform::from_pos(mw / 2.0, mh / 2.0, z)
        };
        out.push(w.spawn_with((t, quad, material)));
    }
    for s in &p.sprites {
        let t = Transform {
            scale: s.scale,
            ..Transform::from_pos(s.pos.x, s.pos.y, s.pos.z)
        };
        out.push(w.spawn_with((t, quad, s.material, Color(s.src))));
    }
    out
}

/// Makes every sprite among `entities` (what `spawn` returned) that hides part of a figure see-through: `fade` of
/// its pixels (0..1) are dropped, so the figure shows. A figure is the box `size` wide and tall standing on `feet`
/// (world units); it is behind a sprite when its feet are above the sprite's bottom edge. Others go back to opaque.
/// Call it every frame, after the figure moved.
pub fn see_through(w: &mut World, entities: &[Entity], feet: Vec2, size: Vec2, fade: f32) {
    for &e in entities {
        let Some(t) = w.get::<Transform>(e) else { continue };
        let (lo, hi) = (t.pos - t.scale / 2.0, t.pos + t.scale / 2.0);
        let hides = feet.y > lo.y
            && feet.y < hi.y + size.y // ...and the box overlaps the sprite
            && feet.x + size.x / 2.0 > lo.x
            && feet.x - size.x / 2.0 < hi.x;
        if let Some(c) = w.get_mut::<Color>(e) {
            c.0[2] = if hides { fade } else { 0.0 }; // tile layers have no `Color`
        }
    }
}

/// Reads and parses the level asset `name` (from the assets directory, see `platform::asset_path`), or dies.
pub fn load(name: &str) -> Level {
    platform::read_asset(name)
        .and_then(|b| Level::parse(&b))
        .unwrap_or_else(|| die(LEVEL_MISSING_OR_CORRUPT))
}
