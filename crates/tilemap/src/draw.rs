//! Turns a `Level` into things gfx draws: a quad per tile layer (a material with the sheet, the layer's cells and
//! which structure owns each cell as textures), an instance per sprite, and per structure an instance of each
//! layer's material over its rectangle. Everything that fades has a blending twin material. `prepare` creates the GPU resources, once per level; `spawn` creates the
//! entities, as often as the level is entered.
use crate::{Kind, Level};
use alloc::{string::String, vec, vec::Vec};
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
const STRUCT_LAYER_STEP: f32 = 0.0001; // between a structure's layers: 100 layers stay under a row's SORT_STEP
const FADE_TIME: f32 = 0.25; // seconds for `see_through` to fade out, and back in

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

/// Component on what fades (sprites, structures): its opaque material and the blending twin it wears while
/// see-through. The opacity is `Color.0[2]`.
#[derive(Clone, Copy)]
struct Fades(Handle<Material>, Handle<Material>);

/// Component on everything `spawn` makes: which layer it is drawn for. Tile layers are 0.. in `Level::layers`
/// order (a structure's quads count as their layer's), then each sprite sheet, in the order of `Level::tilesets`.
/// For debugging and such: hide a layer by moving its entities' z out of the camera's range.
pub struct LayerOf(pub u32);

/// A quad of the level: where it goes, and its `Color` as the shader wants it.
struct Draw {
    layer: u32,
    mats: Fades,
    pos: Vec3,
    scale: Vec3,
    tint: [f32; 3],
}

/// A level's GPU side: materials, and where everything goes. Not tied to a `World` borrow, so it can sit in a
/// resource and be handed to `spawn`.
pub struct Prepared {
    size: (u32, u32),
    layers: Vec<(Handle<Material>, f32, bool)>, // material, z, has structures
    sprites: Vec<Draw>,
    structures: Vec<Draw>, // one per structure and layer with structures
}

fn make_material(
    w: &mut World,
    shader: &'static Align4<[u8]>,
    textures: &[Handle<Texture>],
) -> Fades {
    let m = || {
        Material::new(
            shader,
            size_of::<[f32; 3]>() as u32,
            &[vk::Format::R32G32B32_SFLOAT],
        )
        .with_textures(textures)
    };
    let mats = w.resource_mut::<Assets<Material>>().unwrap();
    Fades(mats.add(m()), mats.add(Material { blend: true, ..m() }))
}

/// Loads the level's image files (through `Sheets`), uploads its layers' cells and builds the materials. The
/// textures reach the GPU the first frame they are drawn.
pub fn prepare(w: &mut World, level: &Level) -> Prepared {
    let sheets: Vec<_> = level.tilesets.iter().map(|t| sheet(w, &t.name)).collect();
    let (lw, lh) = (level.w as usize, level.h as usize);
    let mut owner = vec![0u8; lw * lh]; // which structure has each cell, + 1
    for (i, s) in level.structures.iter().enumerate() {
        for row in (s.y + 1 - s.h) as usize..=s.y as usize {
            owner[row * lw + s.x as usize..][..s.w as usize].fill(i as u8 + 1);
        }
    }
    let owner = w.resource_mut::<Assets<Texture>>().unwrap().add(Texture {
        w: level.w,
        h: level.h,
        format: TexFormat::R8,
        px: owner,
    });
    let (mut layers, mut structures) = (Vec::new(), Vec::new());
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
        let mats = make_material(w, shader, &[sheets[l.tileset as usize], cells, owner]);
        let z = i as f32 * LAYER_STEP + if l.over { OVER_Z } else { 0.0 };
        layers.push((mats.0, z, l.structures));
        if !l.structures {
            continue;
        }
        for (k, s) in level.structures.iter().enumerate() {
            let (pos, scale) = place(level, s.x, s.y, s.w, s.h);
            structures.push(Draw {
                layer: i as u32,
                mats,
                // its layers in order, all well within one row's step
                pos: Vec3::new(pos.x, pos.y, pos.z + i as f32 * STRUCT_LAYER_STEP),
                scale,
                tint: [k as f32 + 1.0, 1.0, 1.0],
            });
        }
    }
    let mut mats: Vec<(u32, Fades)> = Vec::new(); // one pair per sprite sheet
    let mut sprites = Vec::new();
    let sheet_rank = |t: u32| {
        let sheet = |k: &&crate::Tileset| k.kind == Kind::Sprites;
        level.tilesets[..t as usize].iter().filter(sheet).count() as u32
    };
    for s in &level.sprites {
        let m = match mats.iter().find(|m| m.0 == s.tileset) {
            Some(m) => m.1,
            None => {
                let m = make_material(w, SPRITES, &[sheets[s.tileset as usize]]);
                mats.push((s.tileset, m));
                m
            }
        };
        let (pos, scale) = place(level, s.x, s.y, s.w, s.h);
        sprites.push(Draw {
            layer: level.layers.len() as u32 + sheet_rank(s.tileset),
            mats: m,
            pos,
            scale,
            tint: [s.col as f32, s.row as f32, 1.0],
        });
    }
    Prepared {
        size: (level.w, level.h),
        layers,
        sprites,
        structures,
    }
}

/// Center (z: its depth) and size of a `w` x `h` cell rectangle whose bottom left cell is (`x`, `y`).
fn place(level: &Level, x: u32, y: u32, w: u32, h: u32) -> (Vec3, Vec3) {
    let base = (level.h - 1 - y) as f32; // world y of its bottom edge; world y is up, rows go down
    let (w, h) = (w as f32, h as f32);
    (
        Vec3::new(x as f32 + w / 2.0, base + h / 2.0, depth(base)),
        Vec3::new(w, h, 1.0),
    )
}

struct Quad(Handle<Mesh>); // the one unit square everything uses

/// Spawns the level: a quad per tile layer, an entity per sprite, and per structure one per layer that has
/// structures. A cell is one world unit, the map's bottom left corner is the origin and its top row is at the top
/// (world y up). Pairs with `Camera2d`. Despawn the returned entities to leave the level.
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
    for (i, &(material, z, structures)) in p.layers.iter().enumerate() {
        let t = Transform {
            scale: Vec3::new(mw, mh, 1.0),
            ..Transform::from_pos(mw / 2.0, mh / 2.0, z)
        };
        // structure 0: the cells no structure has
        let tint = Color([0.0, structures as u32 as f32, 1.0]);
        out.push(w.spawn_with((t, quad, material, tint, LayerOf(i as u32))));
    }
    for d in p.sprites.iter().chain(&p.structures) {
        let t = Transform {
            scale: d.scale,
            ..Transform::from_pos(d.pos.x, d.pos.y, d.pos.z)
        };
        let layer = LayerOf(d.layer);
        out.push(w.spawn_with((t, quad, d.mats.0, Color(d.tint), d.mats, layer)));
    }
    out
}

/// Fades the sprites and structures that hide part of a figure to `opacity` (0..1), and the others back to opaque,
/// over `FADE_TIME` seconds. A figure is `width` wide, centered on `feet` (world units); it is behind something
/// when its feet are inside it, above its bottom edge. Call it every frame, after the figure moved.
pub fn see_through(w: &mut World, feet: Vec2, width: f32, opacity: f32, dt: f32) {
    let step = dt * (1.0 - opacity) / FADE_TIME;
    for e in w.query_of::<(Fades, Transform, Color)>() {
        let t = w.get::<Transform>(e).unwrap();
        let (lo, hi) = (t.pos - t.scale / 2.0, t.pos + t.scale / 2.0);
        let hides = feet.y > lo.y
            && feet.y < hi.y
            && feet.x + width / 2.0 > lo.x
            && feet.x - width / 2.0 < hi.x;
        let goal = if hides { opacity } else { 1.0 };
        let a = &mut w.get_mut::<Color>(e).unwrap().0[2];
        *a = if *a < goal {
            if *a + step < goal { *a + step } else { goal } // (no libm f32::min here)
        } else if *a - step > goal {
            *a - step
        } else {
            goal
        };
        let opaque = *a >= 1.0;
        let m = *w.get::<Fades>(e).unwrap();
        *w.get_mut::<Handle<Material>>(e).unwrap() = if opaque { m.0 } else { m.1 };
    }
}

/// Reads and parses the level asset `name` (from the assets directory, see `platform::asset_path`), or dies.
pub fn load(name: &str) -> Level {
    platform::read_asset(name)
        .and_then(|b| Level::parse(&b))
        .unwrap_or_else(|| die(LEVEL_MISSING_OR_CORRUPT))
}
