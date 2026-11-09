//! MSDF text: two lorem ipsum paragraphs, word-wrapped, with some words in other colors.
//! `Color` tints a whole mesh instance and a frame draws at most 16 (mesh, material) pairs, so the text is one
//! mesh per color (3), each holding only that color's words, all laid out along one shared pen.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use ecs::{App, Schedule::*, World};
use gfx::{Assets, Atlas, Camera2d, Color, Material, Mesh, RunExt, Transform};

const LOREM: [&str; 2] = [
    "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et \
     dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip \
     ex ea commodo consequat.",
    "Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. \
     Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est \
     laborum.",
];
const COLORS: [[f32; 3]; 3] = [[0.9, 0.9, 0.85], [1.0, 0.6, 0.2], [0.4, 0.8, 1.0]];
const SIZE: f32 = 0.7; // one em, in world units (the camera is 12 units tall)
const WRAP: f32 = 14.0; // line width in world units; the window is 16 wide at its default 4:3

fn setup(w: &mut World, _: f32) {
    let font = font::include_font!("ComicMono");

    let mut verts: [Vec<[f32; 5]>; 3] = Default::default(); // one buffer per color
    let mut pen = [0.0, 0.0];
    let mut n = 0; // words so far: the 4th of every 9 is orange, the 8th blue
    for paragraph in LOREM {
        for word in paragraph.split_whitespace() {
            let color = match n % 9 {
                3 => 1,
                7 => 2,
                _ => 0,
            };
            font.push_word(&mut verts[color], &mut pen, word, SIZE, WRAP);
            n += 1;
        }
        pen = [0.0, pen[1] - font.line * SIZE * 1.5]; // a half-line gap between paragraphs
    }

    w.insert_resource(Atlas {
        w: font.w,
        h: font.h,
        rgb: font.rgb,
        nearest: false,
    });
    let material = w
        .resource_mut::<Assets<Material>>()
        .unwrap()
        .add(Material::msdf());
    for (v, color) in verts.iter().zip(COLORS) {
        let mesh = w.resource_mut::<Assets<Mesh>>().unwrap().add(Mesh::new(v));
        // the pen started at the top left, so the paragraph hangs down from here
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
    app.run(c"Font: lorem ipsum, Esc quits");
}

platform::entry!(game);
