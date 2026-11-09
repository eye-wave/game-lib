//! Bitmap font: `assets/pixel.png` (Press Start 2P at 8 px, SIL OFL; a 16-column grid of the ASCII characters) is both the
//! atlas and, through `media::Font`, the layout.
//! One mesh per color.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use ecs::{App, Schedule::*, World};
use gfx::{Assets, Atlas, Camera2d, Color, Material, Mesh, RunExt, Transform};
use platform::die;

const TEXT: &str = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut \
                    labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco \
                    laboris nisi ut aliquip ex ea commodo consequat.";
const COLORS: [[f32; 3]; 3] = [[0.9, 0.9, 0.85], [1.0, 0.6, 0.2], [0.4, 0.8, 1.0]];
const SIZE: f32 = 0.5; // one em (8 atlas px) in world units: 1 px = 0.1, a whole number of screen px at 800x600
const WRAP: f32 = 14.0;

fn setup(w: &mut World, _: f32) {
    let img = media::load_image("pixel.png").unwrap_or_else(|e| die(e));
    let font = media::Font::new(&img, media::ASCII, 1); // 16 x 6 cells of 10 px, 1 px padding around each 8 px glyph

    let mut verts: [Vec<[f32; 5]>; 3] = Default::default();
    let mut pen = [0.0, 0.0];
    for (n, word) in TEXT.split_whitespace().enumerate() {
        let color = match n % 9 {
            3 => 1,
            7 => 2,
            _ => 0,
        };
        font.push_word(&mut verts[color], &mut pen, word, SIZE, WRAP);
    }

    // every channel is the PNG's alpha
    w.insert_resource(Atlas {
        w: img.w,
        h: img.h,
        rgb: media::alpha_atlas(&img.rgba).leak(),
        nearest: true,
    });
    let material = w
        .resource_mut::<Assets<Material>>()
        .unwrap()
        .add(Material::msdf());
    for (v, color) in verts.iter().zip(COLORS) {
        let mesh = w.resource_mut::<Assets<Mesh>>().unwrap().add(Mesh::new(v));
        w.spawn_with((
            Transform::from_pos(-WRAP / 2.0, 5.0, 0.0),
            mesh,
            material,
            Color(color),
        ));
    }
}

fn game() {
    let mut app = App::new();
    app.add_system(Startup, setup);
    app.world.spawn_with((Camera2d {
        height: 12.0,
        sky: [0.08, 0.08, 0.1],
        ..Camera2d::default()
    },));
    app.run(c"Bitmap font: lorem ipsum, Esc quits");
}

platform::entry!(game);
