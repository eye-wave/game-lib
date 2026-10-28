#![no_std]
#![no_main]

extern crate alloc;

use ecs::{App, World};
use gfx::{Assets, Input, Material, Mesh, RunExt, Transform, Vertex};
use platform::input::{self as key};
use shader::{Align4, include_shader};

static SHADER: &Align4<[u8]> = include_shader!("tri");

const SPEED: f32 = 1.0; // clip-space units per second

struct Player;

fn setup(w: &mut World, _: f32) {
    let mesh = w.resource_mut::<Assets<Mesh>>().unwrap().add(Mesh::new(&[
        Vertex {
            pos: [0.0, 0.2],
            color: [1.0, 0.0, 0.0],
        },
        Vertex {
            pos: [0.2, -0.2],
            color: [0.0, 1.0, 0.0],
        },
        Vertex {
            pos: [-0.2, -0.2],
            color: [0.0, 0.0, 1.0],
        },
    ]));
    let material = w
        .resource_mut::<Assets<Material>>()
        .unwrap()
        .add(Material::colored(SHADER));
    w.spawn_with((Player, Transform::default(), mesh, material));
}

fn steer(w: &mut World, dt: f32) {
    let input = *w.resource::<Input>().unwrap();
    for e in w.query_of::<(Player, Transform)>() {
        let t = w.get_mut::<Transform>(e).unwrap();
        if input.just_pressed(key::Space) {
            *t = Transform::default();
        }
        // diagonals are ~1.4x faster; normalise if it matters.
        t.x = (t.x
            + input.axis(&[key::KeyD, key::ArrowRight], &[key::KeyA, key::ArrowLeft]) * SPEED * dt)
            .clamp(-1.0, 1.0);
        t.y = (t.y
            + input.axis(&[key::KeyW, key::ArrowUp], &[key::KeyS, key::ArrowDown]) * SPEED * dt)
            .clamp(-1.0, 1.0);
    }
}

fn game() {
    let mut app = App::new(240.0);
    app.add_startup(setup).add_update(steer);
    app.run(c"Vulkan mesh demo (arrows / WASD, Space recentres, Esc quits)");
}

platform::entry!(game);
