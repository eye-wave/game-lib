#![no_std]
#![no_main]

extern crate alloc;

use core::f32::consts::{FRAC_1_SQRT_2, TAU};

use ecs::{App, Schedule::*, World};
use gfx::{Assets, Camera, Color, Input, Material, Mesh, RunExt, Transform};
use platform::input::{ArrowDown, ArrowLeft, ArrowRight, ArrowUp, KeyA, KeyD, KeyS, KeyW};
use shader::{Align4, include_shader};

static FLAT: &Align4<[u8]> = include_shader!("flat");

const ARENA: f32 = 32.0; // half-size of the play field
const SQUARES: usize = 300;
const SPEED: f32 = 8.0;
const CAM_Z: f32 = 12.0; // the camera looks straight down -z at the xy plane

struct Player;
struct Spin(f32); // radians per second

/// Position and color only; the first three floats must be the position (see `Mesh::new`).
#[repr(C)]
struct Vertex2 {
    pos: [f32; 3],
    color: [f32; 3],
}

/// A unit square centered on the origin: 2 triangles.
fn quad_mesh() -> Mesh {
    let v = |x: f32, y: f32| Vertex2 {
        pos: [x, y, 0.0],
        color: [1.0; 3],
    };
    Mesh::new(&[
        v(-0.5, -0.5),
        v(0.5, -0.5),
        v(0.5, 0.5),
        v(-0.5, -0.5),
        v(0.5, 0.5),
        v(-0.5, 0.5),
    ])
}

fn setup(w: &mut World, _: f32) {
    let quad = w.resource_mut::<Assets<Mesh>>().unwrap().add(quad_mesh());
    let material = w.resource_mut::<Assets<Material>>().unwrap().add(Material {
        shader: FLAT,
        stride: size_of::<Vertex2>() as u32,
        attrs: &[gfx::vk::Format::R32G32B32_SFLOAT; 2],
    });
    w.resource_mut::<Camera>().unwrap().pos[2] = CAM_Z;

    // ponytail: squares are scattered by a fixed formula, not an RNG; swap in a real one if you want it to vary per run.
    for i in 0..SQUARES {
        let f = |k: usize, m: usize| (i * k % m) as f32 / m as f32; // 0..1, different pattern per k
        let size = 0.6 + f(5, 7);
        w.spawn_with((
            Spin((f(3, 11) - 0.5) * 2.0),
            Transform {
                pos: [
                    (f(37, 61) * 2.0 - 1.0) * ARENA,
                    (f(53, 59) * 2.0 - 1.0) * ARENA,
                    -1.0 + f(1, SQUARES) * 0.5,
                ],
                yaw: f(7, 13) * TAU,
                scale: [size, size, 1.0],
            },
            quad,
            material,
            Color([
                0.3 + 0.7 * f(13, 17),
                0.3 + 0.7 * f(29, 23),
                0.3 + 0.7 * f(41, 19),
            ]),
        ));
    }
    w.spawn_with((
        Player,
        Transform::default(),
        quad,
        material,
        Color([1.0; 3]),
    ));
}

/// WASD or arrows move the player on the plane; the camera follows.
fn steer(w: &mut World, dt: f32) {
    let input = *w.resource::<Input>().unwrap();
    let (x, y) = (
        input.axis(&[KeyD, ArrowRight], &[KeyA, ArrowLeft]),
        input.axis(&[KeyW, ArrowUp], &[KeyS, ArrowDown]),
    );
    let k = if x != 0.0 && y != 0.0 {
        FRAC_1_SQRT_2
    } else {
        1.0
    };

    let mut at = [0.0; 2];
    for e in w.query_of::<(Player, Transform)>() {
        let t = w.get_mut::<Transform>(e).unwrap();
        t.pos[0] = (t.pos[0] + x * k * SPEED * dt).clamp(-ARENA, ARENA);
        t.pos[1] = (t.pos[1] + y * k * SPEED * dt).clamp(-ARENA, ARENA);
        t.pos[2] = 0.5; // in front of every square
        at = [t.pos[0], t.pos[1]];
    }
    w.resource_mut::<Camera>().unwrap().pos = [at[0], at[1], CAM_Z];
}

fn spin(w: &mut World, dt: f32) {
    for e in w.query_of::<(Spin, Transform)>() {
        let s = w.get::<Spin>(e).unwrap().0;
        w.get_mut::<Transform>(e).unwrap().yaw += s * dt;
    }
}

fn game() {
    let mut app = App::new(60.0);
    app.add_system(Startup, setup)
        .add_system(Update, steer)
        .add_system(Update, spin);
    app.run(c"2D walk: WASD or arrows, Esc quits");
}

platform::entry!(game);
