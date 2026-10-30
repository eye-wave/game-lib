#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use core::f32::consts::{FRAC_PI_2, PI};

use ecs::{App, Schedule::*, World};
use gfx::{Assets, Camera2d, Material, Mesh, RunExt, Transform};
use math::{FloatExt, Vec3};

/// Fraction of the window's resolution the right half renders at (it is then scaled up with nearest filtering).
const PIXEL_SCALE: f32 = 0.15;

struct Spin(f32); // radians per second

/// A "bean": a 1 x 1 rectangle capped with a half circle at each end, 2 x 1 overall, as a fan around the origin.
fn bean_mesh() -> Mesh {
    const SEGMENTS: usize = 16; // per half circle
    const HALF: f32 = 0.5; // half the rectangle's length, and the caps' radius
    // right cap from the bottom to the top, then left cap from the top to the bottom: a convex outline
    let outline: Vec<Vec3> = [(HALF, -FRAC_PI_2), (-HALF, FRAC_PI_2)]
        .into_iter()
        .flat_map(|(x, from)| {
            (0..=SEGMENTS).map(move |i| {
                let a = from + PI * i as f32 / SEGMENTS as f32;
                Vec3::new(x + HALF * a.cos(), HALF * a.sin(), 0.0)
            })
        })
        .collect();
    let mut tris = Vec::new();
    for i in 0..outline.len() {
        tris.extend([Vec3::ZERO, outline[i], outline[(i + 1) % outline.len()]]);
    }
    Mesh::new(&tris)
}

fn setup(w: &mut World, _: f32) {
    let bean = w.resource_mut::<Assets<Mesh>>().unwrap().add(bean_mesh());
    let material = w
        .resource_mut::<Assets<Material>>()
        .unwrap()
        .add(Material::flat());

    // one bean, seen by two cameras; no `Color`, so it is white
    w.spawn_with((
        Spin(1.0),
        Transform {
            pos: Vec3::ZERO,
            yaw: 0.0,
            scale: Vec3::new(3.0, 3.0, 1.0),
        },
        bean,
        material,
    ));
}

fn spin(w: &mut World, dt: f32) {
    for e in w.query_of::<(Spin, Transform)>() {
        let s = w.get::<Spin>(e).unwrap().0;
        w.get_mut::<Transform>(e).unwrap().yaw += s * dt;
    }
}

fn game() {
    let mut app = App::new();
    app.add_system(Startup, setup).add_system(Update, spin);
    app.world.spawn_with((Camera2d {
        viewport: [0.0, 0.0, 0.5, 1.0],
        sky: [0.09, 0.09, 0.13],
        ..Default::default()
    },));
    app.world.spawn_with((Camera2d {
        viewport: [0.5, 0.0, 0.5, 1.0],
        scale: PIXEL_SCALE,
        sky: [0.13, 0.09, 0.09],
        ..Default::default()
    },));
    app.run(c"Two cameras: full resolution (left), low-resolution framebuffer (right). Esc quits");
}

platform::entry!(game);
