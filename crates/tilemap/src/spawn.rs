use crate::Level;
use ecs::{Entity, World};
use errors::prelude::*;
use gfx::{Assets, Handle, Material, Mesh, Transform};
use math::Vec3;
use platform::die;

/// Reads and parses the level asset `name` (from the assets directory, see `platform::asset_path`), or dies.
pub fn load(name: &str) -> Level {
    platform::read_asset(name)
        .and_then(|b| Level::parse(&b))
        .unwrap_or_else(|| die(LEVEL_MISSING_OR_CORRUPT))
}

struct Quad(Handle<Mesh>); // the one unit square every level uses

/// One quad for the whole level: a cell is one world unit, the map's bottom left corner is the origin and its
/// top row is at the top (world y up). `z` is 0, so sprites at higher z draw in front. Pairs with `Camera2d`.
pub fn spawn(w: &mut World, material: Handle<Material>, size: (u32, u32)) -> Entity {
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
    let e = w.spawn_with((Transform::from_pos(0.0, 0.0, 0.0), quad, material));
    set(w, e, material, size);
    e
}

/// Points the entity `spawn` made at another level: its material, and its size and place. `size` is the level's
/// `(w, h)`; taking it instead of the `Level` lets the level stay in a world resource while the world is mutated.
pub fn set(w: &mut World, e: Entity, material: Handle<Material>, size: (u32, u32)) {
    *w.get_mut::<Handle<Material>>(e).unwrap() = material;
    let (mw, mh) = (size.0 as f32, size.1 as f32);
    let t = w.get_mut::<Transform>(e).unwrap();
    t.scale = Vec3::new(mw, mh, 1.0);
    t.pos = Vec3::new(mw / 2.0, mh / 2.0, 0.0);
}
