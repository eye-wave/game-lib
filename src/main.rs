#![no_std]
#![no_main]

extern crate alloc;

use core::f32::consts::FRAC_1_SQRT_2;

use alloc::vec::Vec;
use ecs::{App, World};
use gfx::math::{abs, cos, sin};
use gfx::{Assets, Camera, Color, Input, Material, Mesh, PostProcess, RunExt, Transform, Vertex};
use platform::input::{KeyA, KeyD, KeyS, KeyW};
use shader::{Align4, include_shader};

static LIT: &Align4<[u8]> = include_shader!("lit");
static VIGNETTE: &Align4<[u8]> = include_shader!("vignette");

const ARENA: f32 = 32.0; // half-size of the floor
const CUBES: usize = 300;
const EYE: f32 = 1.7; // eye height
const RADIUS: f32 = 0.4; // player collision radius
const SPEED: f32 = 6.0; // units/s
const SENSITIVITY: f32 = 0.0025; // radians per pixel of mouse motion

// ---- components
struct Player;
struct Solid; // blocks the player; must be axis-aligned (yaw 0)

/// Hand-made PRNG: a counter pushed through an integer hash (lowbias32 finalizer).
struct Rng(u32);

fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

impl Rng {
    /// Uniform in [0, 1).
    fn f32(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9e37_79b9);
        (hash(self.0) >> 8) as f32 * (1.0 / 16_777_216.0)
    }
}

/// A unit cube centered on the origin: 6 faces x 2 triangles.
fn cube_mesh() -> Mesh {
    let mut v = Vec::new();
    for a in 0..3 {
        for sign in [-1.0f32, 1.0] {
            let (mut n, mut u, mut w) = ([0.0; 3], [0.0; 3], [0.0; 3]);
            (n[a], u[(a + 1) % 3], w[(a + 2) % 3]) = (sign, 1.0, 1.0);
            let corner = |su: f32, sw: f32| Vertex {
                pos: core::array::from_fn(|i| 0.5 * (n[i] + su * u[i] + sw * w[i])),
                normal: n,
                color: [1.0; 3],
            };
            for (su, sw) in [
                (-1.0, -1.0),
                (1.0, -1.0),
                (1.0, 1.0),
                (-1.0, -1.0),
                (1.0, 1.0),
                (-1.0, 1.0),
            ] {
                v.push(corner(su, sw));
            }
        }
    }
    Mesh::new(&v)
}

/// The floor: a checkerboard of 2x2 cells covering +-ARENA.
fn plane_mesh() -> Mesh {
    let n = (ARENA as i32) as usize; // 32 cells of size 2
    let mut v = Vec::new();
    for i in 0..n {
        for j in 0..n {
            let g = if (i + j) % 2 == 0 { 0.5 } else { 0.38 };
            let at = |di: usize, dj: usize| Vertex {
                pos: [
                    (i + di) as f32 * 2.0 - ARENA,
                    0.0,
                    (j + dj) as f32 * 2.0 - ARENA,
                ],
                normal: [0.0, 1.0, 0.0],
                color: [g * 0.85, g, g * 0.85],
            };
            for (di, dj) in [(0, 0), (1, 0), (1, 1), (0, 0), (1, 1), (0, 1)] {
                v.push(at(di, dj));
            }
        }
    }
    Mesh::new(&v)
}

fn setup(w: &mut World, _: f32) {
    let cube = w.resource_mut::<Assets<Mesh>>().unwrap().add(cube_mesh());
    let plane = w.resource_mut::<Assets<Mesh>>().unwrap().add(plane_mesh());
    let material = w
        .resource_mut::<Assets<Material>>()
        .unwrap()
        .add(Material::lit(LIT));
    w.resource_mut::<PostProcess>().unwrap().0 = Some(VIGNETTE);

    w.spawn_with((Transform::default(), plane, material));
    w.spawn_with((Player, Transform::default()));

    let mut rng = Rng(hash((platform::now() * 1e6) as u32));
    for _ in 0..CUBES {
        let x = (rng.f32() * 2.0 - 1.0) * (ARENA - 2.0);
        let mut z = (rng.f32() * 2.0 - 1.0) * (ARENA - 2.0);
        if abs(x) < 4.0 && abs(z) < 4.0 {
            z += 8.0; // keep the spawn point clear
        }
        let (width, height) = (0.8 + 1.7 * rng.f32(), 2.0 + 10.0 * rng.f32());
        w.spawn_with((
            Solid,
            Transform {
                pos: [x, height * 0.5, z],
                yaw: 0.0,
                scale: [width, height, width],
            },
            cube,
            material,
            Color([
                0.3 + 0.7 * rng.f32(),
                0.3 + 0.7 * rng.f32(),
                0.3 + 0.7 * rng.f32(),
            ]),
        ));
    }
}

/// Mouse look. Moving right turns right (yaw is counter-clockwise), moving down looks down.
fn look(w: &mut World, _: f32) {
    let d = w.resource::<Input>().unwrap().mouse_delta();
    let c = w.resource_mut::<Camera>().unwrap();
    c.yaw -= d[0] * SENSITIVITY;
    c.pitch = (c.pitch - d[1] * SENSITIVITY).clamp(-1.5, 1.5);
}

/// WASD relative to where the camera faces, on the ground plane.
fn walk(w: &mut World, dt: f32) {
    let input = *w.resource::<Input>().unwrap();
    let yaw = w.resource::<Camera>().unwrap().yaw;
    let (fwd, side) = (input.axis(&[KeyW], &[KeyS]), input.axis(&[KeyD], &[KeyA]));
    let k = (if fwd != 0.0 && side != 0.0 {
        FRAC_1_SQRT_2
    } else {
        1.0
    }) * SPEED
        * dt; // no faster diagonals
    let (s, c) = (sin(yaw), cos(yaw));
    // forward is (-sin, -cos) and right is (cos, -sin) in (x, z)
    let (dx, dz) = ((-s * fwd + c * side) * k, (-c * fwd - s * side) * k);
    for e in w.query_of::<(Player, Transform)>() {
        let t = w.get_mut::<Transform>(e).unwrap();
        t.pos[0] = (t.pos[0] + dx).clamp(-ARENA + RADIUS, ARENA - RADIUS);
        t.pos[2] = (t.pos[2] + dz).clamp(-ARENA + RADIUS, ARENA - RADIUS);
    }
}

/// Push the player out of every cube along the axis it penetrates least.
fn collide(w: &mut World, _: f32) {
    let solids = w.query_of::<(Solid, Transform)>();
    for p in w.query_of::<(Player, Transform)>() {
        let mut pos = w.get::<Transform>(p).unwrap().pos;
        for &s in &solids {
            let t = w.get::<Transform>(s).unwrap();
            let (dx, dz) = (pos[0] - t.pos[0], pos[2] - t.pos[2]);
            let px = t.scale[0] * 0.5 + RADIUS - abs(dx); // penetration on each axis
            let pz = t.scale[2] * 0.5 + RADIUS - abs(dz);
            if px > 0.0 && pz > 0.0 {
                if px < pz {
                    pos[0] += if dx < 0.0 { -px } else { px };
                } else {
                    pos[2] += if dz < 0.0 { -pz } else { pz };
                }
            }
        }
        w.get_mut::<Transform>(p).unwrap().pos = pos;
    }
}

fn follow(w: &mut World, _: f32) {
    if let Some(&p) = w.query_of::<(Player, Transform)>().first() {
        let pos = w.get::<Transform>(p).unwrap().pos;
        w.resource_mut::<Camera>().unwrap().pos = [pos[0], pos[1] + EYE, pos[2]];
    }
}

fn game() {
    let mut app = App::new(60.0);
    app.add_startup(setup)
        .add_update(look)
        .add_update(walk)
        .add_update(collide)
        .add_update(follow);
    app.run(c"3D walk: WASD + mouse, Esc quits");
}

platform::entry!(game);
