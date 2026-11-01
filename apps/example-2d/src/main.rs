#![no_std]
#![no_main]

extern crate alloc;

use core::f32::consts::{FRAC_1_SQRT_2, TAU};

use ecs::{App, Schedule::*, World};
use gfx::{Assets, Camera2d, Color, Input, Material, Mesh, RunExt, Transform};
use math::{FloatExt, Vec2, Vec3};
use platform::input::{
    ArrowDown, ArrowLeft, ArrowRight, ArrowUp, KeyA, KeyD, KeyS, KeyW, ShiftLeft,
};
use shader::{Align4, include_shader};

static FLAT: &Align4<[u8]> = include_shader!("flat");

const ARENA: f32 = 32.0; // half-size of the play field
const SQUARES: usize = 300;
const SPEED: f32 = 8.0;

const PLAYER_RADIUS: f32 = 0.25;
const COLLISION_SLOP: f32 = 0.001;

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
    // squares are scattered by a fixed formula, not an RNG; swap in a real one if you want it to vary per run.
    for i in 0..SQUARES {
        let f = |k: usize, m: usize| (i * k % m) as f32 / m as f32; // 0..1, different pattern per k
        let size = 0.6 + f(5, 7);
        w.spawn_with((
            Spin((f(3, 11) - 0.5) * 2.0),
            Transform {
                pos: Vec3::new(
                    (f(37, 61) * 2.0 - 1.0) * ARENA,
                    (f(53, 59) * 2.0 - 1.0) * ARENA,
                    -1.0 + f(1, SQUARES) * 0.5,
                ),
                yaw: f(7, 13) * TAU,
                scale: Vec3::new(size, size, 1.0),
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

fn circle_vs_rotated_square(
    player: Vec2,
    square_pos: Vec2,
    half_size: f32,
    angle: f32,
    radius: f32,
) -> Option<Vec2> {
    let delta = player - square_pos;

    let sin = angle.sin();
    let cos = angle.cos();

    // Transform into square-local coordinates.
    let local = Vec2::new(
        delta.x * cos + delta.y * sin,
        -delta.x * sin + delta.y * cos,
    );

    // Closest point on the square.
    let closest = Vec2::new(
        local.x.clamp(-half_size, half_size),
        local.y.clamp(-half_size, half_size),
    );

    let difference = local - closest;
    let dist_sq = difference.length_squared();

    if dist_sq >= radius * radius {
        return None;
    }

    // Calculate collision normal.
    let normal = if dist_sq > COLLISION_SLOP * COLLISION_SLOP {
        difference / dist_sq.sqrt()
    } else {
        // Player center is inside the square.
        let dx = half_size - local.x.abs();
        let dy = half_size - local.y.abs();

        if dx < dy {
            Vec2::new(if local.x >= 0.0 { 1.0 } else { -1.0 }, 0.0)
        } else {
            Vec2::new(0.0, if local.y >= 0.0 { 1.0 } else { -1.0 })
        }
    };

    // Rotate normal back into world space.
    let world_normal = Vec2::new(
        normal.x * cos - normal.y * sin,
        normal.x * sin + normal.y * cos,
    );

    let penetration = if dist_sq > COLLISION_SLOP * COLLISION_SLOP {
        radius - dist_sq.sqrt()
    } else {
        // Also push the player out if its center is inside.
        let dx = half_size - local.x.abs();
        let dy = half_size - local.y.abs();
        radius + dx.min(dy)
    };

    Some(world_normal * penetration)
}

/// WASD or arrows move the player on the plane; the camera follows.
fn steer(w: &mut World, dt: f32) {
    let input = *w.resource::<Input>().unwrap();

    let boost = input.is_holding(ShiftLeft) as u8 as f32 * 1.5 + 1.0;

    let (x, y) = (
        input.axis(&[KeyD, ArrowRight], &[KeyA, ArrowLeft]),
        input.axis(&[KeyW, ArrowUp], &[KeyS, ArrowDown]),
    );

    let k = if x != 0.0 && y != 0.0 {
        FRAC_1_SQRT_2
    } else {
        1.0
    };

    // Collect obstacle transforms before mutating the player.
    let mut obstacles = alloc::vec::Vec::new();

    for e in w.query_of::<(Spin, Transform)>() {
        let t = w.get::<Transform>(e).unwrap();

        obstacles.push((t.pos.truncate(), t.scale.x * 0.5, t.yaw));
    }

    for e in w.query_of::<(Player, Transform)>() {
        let t = w.get_mut::<Transform>(e).unwrap();

        let movement = Vec2::new(x, y) * (k * SPEED * boost * dt);

        let mut pos = t.pos.truncate() + movement;

        // Resolve collisions.
        // Multiple passes help when several squares overlap.
        for _ in 0..4 {
            let mut collided = false;

            for &(square_pos, half_size, angle) in &obstacles {
                if let Some(push) =
                    circle_vs_rotated_square(pos, square_pos, half_size, angle, PLAYER_RADIUS)
                {
                    pos += push;
                    collided = true;
                }
            }

            if !collided {
                break;
            }
        }

        pos.x = pos.x.clamp(-ARENA, ARENA);
        pos.y = pos.y.clamp(-ARENA, ARENA);

        t.pos.x = pos.x;
        t.pos.y = pos.y;

        for e in w.query_of::<(Camera2d,)>() {
            w.get_mut::<Camera2d>(e).unwrap().pos = pos;
        }
    }
}

fn spin(w: &mut World, dt: f32) {
    for e in w.query_of::<(Spin, Transform)>() {
        let s = w.get::<Spin>(e).unwrap().0;
        w.get_mut::<Transform>(e).unwrap().yaw += s * dt;
    }
}

fn game() {
    let mut app = App::new();
    app.add_system(Startup, setup)
        .add_system(Fixed, steer)
        .add_system(Update, spin);
    app.world.spawn_with((Camera2d::default(),)); // without a camera, `run` adds a 3D one
    app.run(c"2D walk: WASD or arrows, Esc quits");
}

platform::entry!(game);
