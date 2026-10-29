#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use core::f32::consts::{FRAC_PI_2, PI, TAU};
use ecs::{App, World};
use gfx::{Assets, Camera, Color, Handle, Input, Material, Mesh, RunExt, Transform, Vertex};
use platform::input::{ArrowDown, ArrowLeft, ArrowRight, ArrowUp, KeyA, KeyD, KeyS, KeyW, KeyZ};
use shader::{Align4, include_shader};

static SHADER: &Align4<[u8]> = include_shader!("flat");

const ARENA: f32 = 50.0; // player is kept inside +-ARENA; stars fill it
const TURN: f32 = 3.0; // rad/s
const FORWARD: f32 = 6.0; // units/s
const BACKWARD: f32 = 3.0;
const PELLET_SPEED: f32 = 14.0;
const FIRE_EVERY: f32 = 0.02; // s
const MAX_ENEMIES: usize = 120;
const EXPLOSION: f32 = 0.35; // s

// ---- components
struct Player {
    cooldown: f32,
}
struct Enemy;
struct Pellet;
struct Explosion {
    r0: f32,
}
struct Velocity {
    x: f32,
    y: f32,
}
struct Life(f32); // seconds left

// ---- resources
#[derive(Clone, Copy)]
struct Art {
    circle: Handle<Mesh>,
    material: Handle<Material>,
}
struct Timers {
    spawn: f32,
    compact: f32,
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
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x9e37_79b9);
        hash(self.0)
    }
    /// Uniform in [0, 1).
    fn f32(&mut self) -> f32 {
        (self.next() >> 8) as f32 * (1.0 / 16_777_216.0)
    }
}

/// There is no libm in no_std: sin on [-pi, pi] via symmetry + a 9th-order Taylor series (error < 4e-6).
fn sin(x: f32) -> f32 {
    let mut r = x - (x * (1.0 / TAU)) as i32 as f32 * TAU; // (-tau, tau)
    if r > PI {
        r -= TAU;
    } else if r < -PI {
        r += TAU;
    }
    if r > FRAC_PI_2 {
        r = PI - r;
    } else if r < -FRAC_PI_2 {
        r = -PI - r;
    }
    let r2 = r * r;
    r * (1.0
        + r2 * (-1.0 / 6.0 + r2 * (1.0 / 120.0 + r2 * (-1.0 / 5040.0 + r2 * (1.0 / 362_880.0)))))
}
fn cos(x: f32) -> f32 {
    sin(x + FRAC_PI_2)
}

/// atan2 without libm: an odd polynomial for atan on [-1, 1] (error ~1e-5), folded into the right octant.
fn atan2(y: f32, x: f32) -> f32 {
    let (ax, ay) = (if x < 0.0 { -x } else { x }, if y < 0.0 { -y } else { y });
    let (lo, hi) = if ax < ay { (ax, ay) } else { (ay, ax) };
    if hi == 0.0 {
        return 0.0;
    }
    let (a, s) = (lo / hi, lo / hi * (lo / hi));
    let mut r =
        a * (0.999_866 + s * (-0.330_299_5 + s * (0.180_141 + s * (-0.085_133 + s * 0.020_835_1))));
    if ay > ax {
        r = FRAC_PI_2 - r;
    }
    if x < 0.0 {
        r = PI - r;
    }
    if y < 0.0 { -r } else { r }
}

fn circle_mesh() -> Mesh {
    const N: usize = 24;
    let rim = |i: usize| {
        let a = i as f32 * TAU / N as f32;
        Vertex {
            pos: [cos(a), sin(a)],
            color: [1.0; 3],
        }
    };
    let mut v = Vec::new();
    for i in 0..N {
        v.push(Vertex {
            pos: [0.0, 0.0],
            color: [1.0; 3],
        });
        v.push(rim(i));
        v.push(rim(i + 1));
    }
    Mesh::new(&v)
}

fn setup(w: &mut World, _: f32) {
    let ship = Mesh::new(&[
        Vertex {
            pos: [0.0, 0.5],
            color: [1.0, 0.0, 0.0],
        },
        Vertex {
            pos: [0.4, -0.5],
            color: [0.0, 1.0, 0.0],
        },
        Vertex {
            pos: [-0.4, -0.5],
            color: [0.0, 0.0, 1.0],
        },
    ]);
    let ship = w.resource_mut::<Assets<Mesh>>().unwrap().add(ship);
    let circle = w.resource_mut::<Assets<Mesh>>().unwrap().add(circle_mesh());
    let material = w
        .resource_mut::<Assets<Material>>()
        .unwrap()
        .add(Material::colored(SHADER));
    let art = Art { circle, material };

    w.resource_mut::<Camera>().unwrap().half_height = 4.0;
    w.insert_resource(art);
    w.insert_resource(Timers {
        spawn: 0.0,
        compact: 0.0,
    });
    let mut rng = Rng(hash((platform::now() * 1e6) as u32));

    w.spawn_with((
        Player { cooldown: 0.0 },
        Transform::default(),
        ship,
        material,
    ));

    // a static starfield, so there is something to fly past (and plenty to cull)
    for _ in 0..1500 {
        let g = 0.15 + 0.35 * rng.f32();
        w.spawn_with((
            Transform {
                x: (rng.f32() * 2.0 - 1.0) * ARENA,
                y: (rng.f32() * 2.0 - 1.0) * ARENA,
                rot: 0.0,
                scale: 0.04 + 0.06 * rng.f32(),
            },
            circle,
            material,
            Color([g, g, g]),
        ));
    }
    w.insert_resource(rng);
}

/// The ship aims at the mouse whenever the mouse moves; A/D (or left/right) turn it otherwise.
/// W/S (or up/down) drive it along its nose.
fn steer(w: &mut World, dt: f32) {
    let input = *w.resource::<Input>().unwrap();
    let turn = input.axis(&[KeyA, ArrowLeft], &[KeyD, ArrowRight]);
    let drive = input.axis(&[KeyW, ArrowUp], &[KeyS, ArrowDown]);
    let speed = drive * if drive > 0.0 { FORWARD } else { BACKWARD };
    let [mx, my] = input.mouse_world();
    for e in w.query_of::<(Player, Transform)>() {
        let t = w.get_mut::<Transform>(e).unwrap();
        if input.mouse_moved() {
            // the nose is (-sin rot, cos rot), so pointing at (dx, dy) means rot = atan2(-dx, dy)
            t.rot = atan2(t.x - mx, my - t.y);
        } else {
            t.rot += turn * TURN * dt;
        }
        // the nose points along +y rotated by `rot`: (-sin, cos)
        t.x = (t.x - sin(t.rot) * speed * dt).clamp(-ARENA, ARENA);
        t.y = (t.y + cos(t.rot) * speed * dt).clamp(-ARENA, ARENA);
    }
}

fn follow_camera(w: &mut World, _: f32) {
    if let Some(&e) = w.query_of::<(Player, Transform)>().first() {
        let t = *w.get::<Transform>(e).unwrap();
        let c = w.resource_mut::<Camera>().unwrap();
        (c.x, c.y) = (t.x, t.y);
    }
}

fn shoot(w: &mut World, dt: f32) {
    let held = w.resource::<Input>().unwrap().is_holding(KeyZ);
    let art = *w.resource::<Art>().unwrap();
    for e in w.query_of::<(Player, Transform)>() {
        let t = *w.get::<Transform>(e).unwrap();
        let p = w.get_mut::<Player>(e).unwrap();
        p.cooldown -= dt;
        if held && p.cooldown <= 0.0 {
            p.cooldown = FIRE_EVERY;
            let (dx, dy) = (-sin(t.rot), cos(t.rot));
            w.spawn_with((
                Pellet,
                Life(1.5),
                Velocity {
                    x: dx * PELLET_SPEED,
                    y: dy * PELLET_SPEED,
                },
                Transform {
                    x: t.x + dx * 0.5,
                    y: t.y + dy * 0.5,
                    rot: 0.0,
                    scale: 0.1,
                },
                art.circle,
                art.material,
                Color([1.0; 3]),
            ));
        }
    }
}

/// Enemies appear on a ring outside the view and drift across the player's area.
fn spawn_enemies(w: &mut World, dt: f32) {
    let timers = w.resource_mut::<Timers>().unwrap();
    timers.spawn -= dt;
    if timers.spawn > 0.0 {
        return;
    }
    let Some(&player) = w.query_of::<(Player, Transform)>().first() else {
        return;
    };
    if w.query_of::<(Enemy,)>().len() >= MAX_ENEMIES {
        return;
    }
    let p = *w.get::<Transform>(player).unwrap();
    let r = w.resource_mut::<Rng>().unwrap();
    let (angle, size, aim, speed, wait, rgb) = (
        r.f32() * TAU,
        r.f32(),
        r.f32() - 0.5,
        r.f32(),
        r.f32(),
        r.next(),
    );
    w.resource_mut::<Timers>().unwrap().spawn = 0.1 + 0.4 * wait;

    let art = *w.resource::<Art>().unwrap();
    let toward = angle + PI + aim; // roughly back at the player
    let v = 0.8 + 1.7 * speed;
    let chan = |shift: u32| 0.4 + 0.6 * ((rgb >> shift) & 255) as f32 / 255.0;
    w.spawn_with((
        Enemy,
        Transform {
            x: p.x + 12.0 * cos(angle),
            y: p.y + 12.0 * sin(angle),
            rot: 0.0,
            scale: 0.3 + 0.6 * size,
        },
        Velocity {
            x: v * cos(toward),
            y: v * sin(toward),
        },
        art.circle,
        art.material,
        Color([chan(0), chan(8), chan(16)]),
    ));
}

fn motion(w: &mut World, dt: f32) {
    for e in w.query_of::<(Velocity, Transform)>() {
        let (vx, vy) = {
            let v = w.get::<Velocity>(e).unwrap();
            (v.x, v.y)
        };
        let t = w.get_mut::<Transform>(e).unwrap();
        t.x += vx * dt;
        t.y += vy * dt;
    }
}

/// A pellet touching an enemy removes both and leaves an explosion.
fn collide(w: &mut World, _: f32) {
    let enemies = w.query_of::<(Enemy, Transform)>();
    let art = *w.resource::<Art>().unwrap();
    for p in w.query_of::<(Pellet, Transform)>() {
        let pt = *w.get::<Transform>(p).unwrap();
        for &e in &enemies {
            if !w.alive(e) {
                continue;
            }
            let et = *w.get::<Transform>(e).unwrap();
            let (dx, dy, r) = (pt.x - et.x, pt.y - et.y, pt.scale + et.scale);
            if dx * dx + dy * dy < r * r {
                w.despawn(p);
                w.despawn(e);
                w.spawn_with((
                    Explosion { r0: et.scale },
                    Life(EXPLOSION),
                    Transform {
                        scale: et.scale,
                        ..et
                    },
                    art.circle,
                    art.material,
                    Color([1.0, 0.6, 0.1]),
                ));
                break;
            }
        }
    }
}

/// Explosions grow and fade to black.
fn animate_explosions(w: &mut World, _: f32) {
    for e in w.query_of::<(Explosion, Life, Transform, Color)>() {
        let r0 = w.get::<Explosion>(e).unwrap().r0;
        let f = (1.0 - w.get::<Life>(e).unwrap().0 / EXPLOSION).clamp(0.0, 1.0);
        w.get_mut::<Transform>(e).unwrap().scale = r0 * (1.0 + 1.5 * f);
        w.get_mut::<Color>(e).unwrap().0 = [1.0 - f, 0.6 * (1.0 - f), 0.1 * (1.0 - f)];
    }
}

/// Timed entities expire, far-away enemies are dropped, and the ECS arena is compacted once a second.
fn cleanup(w: &mut World, dt: f32) {
    for e in w.query_of::<(Life,)>() {
        let l = w.get_mut::<Life>(e).unwrap();
        l.0 -= dt;
        if l.0 <= 0.0 {
            w.despawn(e);
        }
    }
    if let Some(&player) = w.query_of::<(Player, Transform)>().first() {
        let p = *w.get::<Transform>(player).unwrap();
        for e in w.query_of::<(Enemy, Transform)>() {
            let t = w.get::<Transform>(e).unwrap();
            let (dx, dy) = (t.x - p.x, t.y - p.y);
            if dx * dx + dy * dy > 400.0 {
                w.despawn(e);
            }
        }
    }
    let timers = w.resource_mut::<Timers>().unwrap();
    timers.compact -= dt;
    if timers.compact <= 0.0 {
        timers.compact = 1.0;
        w.compact();
    }
}

fn game() {
    let mut app = App::new(60.0);
    app.add_startup(setup)
        .add_update(steer)
        .add_update(follow_camera)
        .add_update(shoot)
        .add_update(spawn_enemies)
        .add_update(motion)
        .add_update(collide)
        .add_update(animate_explosions)
        .add_update(cleanup);
    app.run(c"Circles: mouse or A/D aim, W/S move, Space shoots, Esc quits");
}

platform::entry!(game);
