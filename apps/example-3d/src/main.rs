#![no_std]
#![no_main]

extern crate alloc;

use core::f32::consts::FRAC_1_SQRT_2;

use alloc::vec::Vec;
use ecs::{App, Schedule::*, World};
use gfx::{
    Assets, Camera3d, Color, Input, Material, Mesh, MouseCapture, PostProcess, RunExt, Transform,
    Vertex,
};
use math::{FloatExt, Vec3};
use platform::input::{KeyA, KeyD, KeyS, KeyW, Space};
use shader::{Align4, include_shader};

static LIT: &Align4<[u8]> = include_shader!("lit");
static VIGNETTE: &Align4<[u8]> = include_shader!("vignette");

const ARENA: f32 = 32.0; // half-size of the floor
const CUBES: usize = 300;
const EYE: f32 = 1.7; // eye height
const RADIUS: f32 = 0.4; // player collision radius
const SENSITIVITY: f32 = 0.001; // radians per unit of mouse motion (raw counts; ~8 inches per turn at 800 dpi)

const HEIGHT: f32 = EYE + 0.2; // player collision height (feet to head)

const GRAVITY: f32 = 30.0;
const JUMP_FORCE: f32 = 20.0;
const TERMINAL_VELOCITY: f32 = 50.0;

/// The (first) 3D camera entity; `run` spawns one if the app didn't.
fn camera(w: &mut World) -> &mut Camera3d {
    let e = w.query_of::<(Camera3d,)>()[0];
    w.get_mut::<Camera3d>(e).unwrap()
}

// ---- components
struct Player;
struct Solid; // blocks the player; must be axis-aligned (yaw 0)

#[derive(Clone, Copy)]
struct Velocity {
    x: f32,
    y: f32,
    z: f32,
    grounded: bool,
}

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
                pos: core::array::from_fn::<f32, 3, _>(|i| 0.5 * (n[i] + su * u[i] + sw * w[i]))
                    .into(),
                normal: n.into(),
                color: Vec3::ONE,
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
                pos: Vec3::new(
                    (i + di) as f32 * 2.0 - ARENA,
                    0.0,
                    (j + dj) as f32 * 2.0 - ARENA,
                ),
                normal: Vec3::new(0.0, 1.0, 0.0),
                color: Vec3::new(g * 0.85, g, g * 0.85),
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
    w.spawn_with((
        Player,
        Transform::default(),
        Velocity {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            grounded: false,
        },
    ));

    let mut rng = Rng(hash((platform::now() * 1e6) as u32));
    for _ in 0..CUBES {
        let x = (rng.f32() * 2.0 - 1.0) * (ARENA - 2.0);
        let mut z = (rng.f32() * 2.0 - 1.0) * (ARENA - 2.0);
        if (x).abs() < 4.0 && (z).abs() < 4.0 {
            z += 8.0; // keep the spawn point clear
        }
        let (width, height) = (0.8 + 1.7 * rng.f32(), 2.0 + 10.0 * rng.f32());
        w.spawn_with((
            Solid,
            Transform {
                pos: Vec3::new(x, height * 0.5, z),
                yaw: 0.0,
                scale: Vec3::new(width, height, width),
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
    let c = camera(w);
    c.yaw -= d[0] * SENSITIVITY;
    c.pitch = (c.pitch - d[1] * SENSITIVITY).clamp(-1.5, 1.5);
}

/// WASD relative to where the camera faces, on the ground plane.
const ACCEL: f32 = 40.0;
const FRICTION: f32 = 12.0;
const MAX_SPEED: f32 = 8.0;

fn walk(w: &mut World, dt: f32) {
    let input = *w.resource::<Input>().unwrap();
    let yaw = camera(w).yaw;

    let (fwd, side) = (input.axis(&[KeyW], &[KeyS]), input.axis(&[KeyD], &[KeyA]));

    let k = if fwd != 0.0 && side != 0.0 {
        FRAC_1_SQRT_2
    } else {
        1.0
    };

    let (s, c) = ((yaw).sin(), (yaw).cos());
    let (ax, az) = (
        (-s * fwd + c * side) * k * ACCEL,
        (-c * fwd - s * side) * k * ACCEL,
    );

    for e in w.query_of::<(Player, Transform, Velocity)>() {
        let (t, v) = w.getn_mut::<(Transform, Velocity)>(e).unwrap();

        if input.just_pressed(Space) && v.grounded {
            v.y = JUMP_FORCE;
        }

        v.y = (v.y - GRAVITY * dt).max(-TERMINAL_VELOCITY);

        if fwd != 0.0 || side != 0.0 {
            v.x += ax * dt;
            v.z += az * dt;

            let speed = (v.x * v.x + v.z * v.z).sqrt();

            if speed > MAX_SPEED {
                let scale = MAX_SPEED / speed;
                v.x *= scale;
                v.z *= scale;
            }
        } else {
            let speed = (v.x * v.x + v.z * v.z).sqrt();

            if speed > 0.0 {
                let scale = (speed - FRICTION * dt).max(0.0) / speed;
                v.x *= scale;
                v.z *= scale;
            }
        }

        let (vx, vy, vz) = (v.x, v.y, v.z);

        t.pos.x = (t.pos.x + vx * dt).clamp(-ARENA + RADIUS, ARENA - RADIUS);
        t.pos.y = (t.pos.y + vy * dt).max(0.0);
        t.pos.z = (t.pos.z + vz * dt).clamp(-ARENA + RADIUS, ARENA - RADIUS);

        v.grounded = false; // collide() sets it again when we land on a box
        if t.pos.y == 0.0 && v.y < 0.0 {
            v.y = 0.0;
            v.grounded = true;
        }
    }
}

/// Push the player out of every cube along the axis it penetrates least (3D AABB).
fn collide(w: &mut World, _: f32) {
    let solids = w.query_of::<(Solid, Transform)>();

    for p in w.query_of::<(Player, Transform, Velocity)>() {
        let mut pos = w.get::<Transform>(p).unwrap().pos;

        for &s in &solids {
            let t = w.get::<Transform>(s).unwrap();
            // centre-to-centre; the player's centre is half its height above its feet
            let d = Vec3::new(
                pos.x - t.pos.x,
                pos.y + HEIGHT * 0.5 - t.pos.y,
                pos.z - t.pos.z,
            );
            let pen = Vec3::new(
                t.scale.x * 0.5 + RADIUS - d.x.abs(),
                t.scale.y * 0.5 + HEIGHT * 0.5 - d.y.abs(),
                t.scale.z * 0.5 + RADIUS - d.z.abs(),
            );
            if pen.x <= 0.0 || pen.y <= 0.0 || pen.z <= 0.0 {
                continue;
            }

            let v = w.get_mut::<Velocity>(p).unwrap();
            let sign = |x: f32| if x < 0.0 { -1.0 } else { 1.0 };

            if pen.x < pen.y && pen.x < pen.z {
                pos.x += sign(d.x) * pen.x;
                v.x = 0.0;
            } else if pen.z < pen.y {
                pos.z += sign(d.z) * pen.z;
                v.z = 0.0;
            } else {
                pos.y += sign(d.y) * pen.y;
                if d.y > 0.0 {
                    v.grounded = true; // landed on top
                    v.y = v.y.max(0.0);
                } else {
                    v.y = v.y.min(0.0); // bumped head
                }
            }
        }

        w.get_mut::<Transform>(p).unwrap().pos = pos;
    }
}

fn follow(w: &mut World, _: f32) {
    if let Some(&p) = w.query_of::<(Player, Transform)>().first() {
        let pos = w.get::<Transform>(p).unwrap().pos;
        camera(w).pos = Vec3::new(pos.x, pos.y + EYE, pos.z);
    }
}

fn game() {
    let mut app = App::new(60.0);
    app.add_system(Startup, setup)
        .add_system(Update, look)
        .add_system(Update, walk)
        .add_system(Update, collide)
        .add_system(Update, follow);
    app.world.insert_resource(MouseCapture(true)); // mouse look needs it
    app.run(c"3D walk: WASD + mouse, Esc quits");
}

platform::entry!(game);
