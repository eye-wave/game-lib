//! Loads walk.png (7 frames, 24x24 each, in a row) and plays it once each time Space is pressed.
//! no textures in gfx yet, so every sprite pixel is a 1x1 quad whose Color is rewritten per frame (576 instances).
//! Add a textured material when sprites get big or numerous.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use ecs::{App, Entity, Schedule::*, World};
use errors::prelude::*;
use gfx::{Assets, Camera2d, Color, Input, Material, Mesh, RunExt, Transform};
use math::Vec3;
use media::{Grid, Image};
use platform::{die, input::Space};

const CELL: u32 = 32;
const FRAMES: u32 = 16;
const FPS: f32 = 24.0;

struct Sprite {
    img: Image,
    grid: Grid,
    px: Vec<Entity>, // CELL * CELL quads, row by row
    frame: u32,
    t: f32, // seconds since Space was pressed
    playing: bool,
}

fn quad_mesh() -> Mesh {
    let v = Vec3::new;
    Mesh::new(&[
        v(-0.5, -0.5, 0.0),
        v(0.5, -0.5, 0.0),
        v(0.5, 0.5, 0.0),
        v(-0.5, -0.5, 0.0),
        v(0.5, 0.5, 0.0),
        v(-0.5, 0.5, 0.0),
    ])
}

fn setup(w: &mut World, _: f32) {
    let bytes = platform::read_asset("walk.png").unwrap_or_else(|| die(READ_FILE_FAILED));
    let img = media::png(&bytes).unwrap_or_else(|e| die(e));
    let grid = Grid::new(&img, CELL, CELL);

    let quad = w.resource_mut::<Assets<Mesh>>().unwrap().add(quad_mesh());
    let material = w
        .resource_mut::<Assets<Material>>()
        .unwrap()
        .add(Material::flat());
    let half = (CELL - 1) as f32 / 2.0; // centers the sprite on the origin; image y grows down, world y grows up
    let px = (0..CELL * CELL)
        .map(|i| {
            let (x, y) = ((i % CELL) as f32, (i / CELL) as f32);
            w.spawn_with((
                Transform::from_pos(x - half, half - y, 0.0),
                quad,
                material,
                Color([0.0; 3]),
            ))
        })
        .collect();

    w.insert_resource(Sprite {
        img,
        grid,
        px,
        frame: 0,
        t: 0.0,
        playing: false,
    });
    show(w, 0);
}

/// Paint cell `frame` of the sheet onto the quads. Transparent pixels get scale 0 (no blending: alpha is on/off).
fn show(w: &mut World, frame: u32) {
    let s = w.resource::<Sprite>().unwrap();
    let paint: Vec<(Entity, [u8; 4])> =
        s.px.iter()
            .enumerate()
            .map(|(i, &e)| {
                (
                    e,
                    s.grid.px(&s.img, frame, i as u32 % CELL, i as u32 / CELL),
                )
            })
            .collect();
    for (e, [r, g, b, a]) in paint {
        w.get_mut::<Color>(e).unwrap().0 = [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0];
        w.get_mut::<Transform>(e).unwrap().scale = Vec3::splat((a >= 128) as u8 as f32);
    }
}

/// Space plays the animation once; presses while it is playing are ignored.
fn animate(w: &mut World, dt: f32) {
    let pressed = w.resource::<Input>().unwrap().just_pressed(Space);
    let s = w.resource_mut::<Sprite>().unwrap();
    if pressed && !s.playing {
        (s.playing, s.t) = (true, 0.0);
    }
    if !s.playing {
        return;
    }
    s.t += dt;
    let f = (s.t * FPS) as u32;
    if f >= FRAMES {
        s.playing = false;
    }
    let f = if s.playing { f } else { 0 }; // back to the rest pose
    if f != s.frame {
        s.frame = f;
        show(w, f);
    }
}

fn game() {
    let mut app = App::new();
    app.add_system(Startup, setup).add_system(Update, animate);
    app.world.spawn_with((Camera2d {
        height: 32.0,
        ..Camera2d::default()
    },));
    app.run(c"Sprite: Space plays, Esc quits");
}

platform::entry!(game);
