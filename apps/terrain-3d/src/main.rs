#![no_std]
#![no_main]

extern crate alloc;

use core::f32::consts::{FRAC_1_SQRT_2, PI, TAU};

mod hud;

use alloc::{vec, vec::Vec};
use ecs::{App, Schedule::*, World};
use gfx::{
    Assets, Camera3d, Color, FixedInput, Handle, Input, Material, Mesh, MouseCapture, RunExt,
    Transform, Vertex,
};
use math::{FloatExt, Vec3};
use platform::input::{KeyA, KeyD, KeyE, KeyS, KeyW, Space};
use shader::{Align4, include_shader};

static LIT: &Align4<[u8]> = include_shader!("lit");
static TEX: &Align4<[u8]> = include_shader!("tex"); // lit + procedural checker

const N: usize = 96; // terrain cells per side
const CELL: f32 = 1.5; // cell size
const HALF: f32 = N as f32 * CELL * 0.5;
const HMAX: f32 = 12.0; // rough height range of the hills
const FREQ: f32 = 0.03; // noise frequency of the first octave, per world unit (hill width ~ 1/FREQ)
const CUBES: usize = 150;

// Two scenes, only one loaded at a time (`Level`): the terrain with the big cube (door on its +z face), and the
// voxel room behind that door. Walking through a door unloads one and loads the other.
const BX: f32 = 0.0;
const BZ: f32 = -24.0;
const BIG: f32 = 12.0; // footprint
const BIG_H: f32 = 8.0; // height above the door
const RW: i32 = 20; // room size in voxels (1 unit each)
const RH: i32 = 8;
const RD: i32 = 20;
const DOOR_REACH: f32 = 3.5;

const RADIUS: f32 = 0.4; // capsule
const HEIGHT: f32 = 1.8;
const CAM_DIST: f32 = 7.0;
const CAM_LOOK_AT: f32 = 1.2; // height above the feet the camera orbits
const SENSITIVITY: f32 = 0.001;

const JUMP_SLACK: f32 = 0.5; // you can still jump this far above the ground, once per airtime
const GRAVITY: f32 = 30.0;
const JUMP_FORCE: f32 = 12.0;
const ACCEL: f32 = 40.0;
const FRICTION: f32 = 12.0;
const MAX_SPEED: f32 = 8.0;

fn camera(w: &mut World) -> &mut Camera3d {
    let e = w.query_of::<(Camera3d,)>()[0];
    w.get_mut::<Camera3d>(e).unwrap()
}

struct Player;

struct Scenery; // everything the current scene spawned; despawned on a scene change

/// Voxel grid of the room, x fastest then z then y. Out of bounds counts as solid.
struct Room(Vec<bool>);

/// The loaded scene and its collision model: heightfield outdoors, voxels indoors. Never both.
enum Level {
    Out(Terrain),
    In(Room),
}

/// Meshes, materials and the world seed (so the outdoors comes back identical after a visit indoors).
#[derive(Clone, Copy)]
struct Shared {
    ground: Handle<Mesh>,
    cube: Handle<Mesh>,
    plain: Handle<Material>,
    tex: Handle<Material>,
    seed: u32,
}
struct Solid; // axis-aligned cube that blocks the player

#[derive(Clone, Copy)]
struct Velocity {
    x: f32,
    y: f32,
    z: f32,
    grounded: bool,
    jumped: bool, // jumped since last touching ground: no second jump in the air
}

/// lowbias32 integer hash.
fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

struct Rng(u32);
impl Rng {
    /// Uniform in [0, 1): a counter through the hash.
    fn f32(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9e37_79b9);
        (hash(self.0) >> 8) as f32 * (1.0 / 16_777_216.0)
    }
}

/// 2D Perlin noise, roughly in [-0.7, 0.7]. Needs x, y >= 0 (truncation stands in for floor).
fn perlin(x: f32, y: f32, seed: u32) -> f32 {
    let (xi, yi) = (x as u32, y as u32);
    let (fx, fy) = (x - xi as f32, y - yi as f32);
    // dot of the offset to a lattice corner with that corner's random unit gradient
    let corner = |dx: u32, dy: u32| {
        let h =
            hash(seed ^ (xi + dx).wrapping_mul(0x27d4_eb2d) ^ (yi + dy).wrapping_mul(0x1656_67b1));
        let a = h as f32 * (TAU / 4_294_967_296.0);
        (fx - dx as f32) * a.cos() + (fy - dy as f32) * a.sin()
    };
    let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (u, v) = (fade(fx), fade(fy));
    let top = corner(0, 0) + u * (corner(1, 0) - corner(0, 0));
    let bot = corner(0, 1) + u * (corner(1, 1) - corner(0, 1));
    top + v * (bot - top)
}

fn fl(x: f32) -> i32 {
    x as i32 - (x < 0.0) as i32
}

impl Room {
    fn ix(x: i32, y: i32, z: i32) -> usize {
        ((y * RD + z) * RW + x) as usize
    }
    fn solid(&self, x: i32, y: i32, z: i32) -> bool {
        x < 0 || y < 0 || z < 0 || x >= RW || y >= RH || z >= RD || self.0[Self::ix(x, y, z)]
    }
    /// Does the player's box with feet at `p` (room coordinates) overlap a solid voxel?
    fn hits(&self, p: [f32; 3]) -> bool {
        for y in fl(p[1])..=fl(p[1] + HEIGHT) {
            for z in fl(p[2] - RADIUS)..=fl(p[2] + RADIUS) {
                for x in fl(p[0] - RADIUS)..=fl(p[0] + RADIUS) {
                    if self.solid(x, y, z) {
                        return true;
                    }
                }
            }
        }
        false
    }
    /// Move by vel*dt one axis at a time; a blocked axis snaps flush against the voxel face and loses its velocity.
    /// Returns whether we stand on something. (Assumes a step moves less than a voxel.)
    fn step(&self, p: &mut [f32; 3], vel: &mut [f32; 3], dt: f32) -> bool {
        const EPS: f32 = 1e-3;
        let mut grounded = false;
        for a in 0..3 {
            let (lo, hi) = if a == 1 {
                (0.0, HEIGHT)
            } else {
                (RADIUS, RADIUS)
            }; // box extent below / above p
            let mut q = *p;
            q[a] += vel[a] * dt;
            if self.hits(q) {
                q[a] = if vel[a] > 0.0 {
                    fl(q[a] + hi) as f32 - hi - EPS
                } else {
                    (fl(q[a] - lo) + 1) as f32 + lo + EPS
                };
                grounded |= a == 1 && vel[a] <= 0.0;
                vel[a] = 0.0;
            }
            *p = q;
        }
        grounded
    }
}

/// (N+1)^2 random heights on a regular grid. Height lookup is O(1): index the cell, interpolate its triangle.
struct Terrain(Vec<f32>);

impl Terrain {
    /// Grid heights from 3 octaves of Perlin noise: smooth hills instead of white-noise spikes.
    fn new(seed: u32) -> Self {
        let height = |i: usize, j: usize| {
            let (mut sum, mut amp, mut freq) = (0.0, 1.0, FREQ);
            for o in 0..3 {
                sum += amp
                    * perlin(
                        i as f32 * CELL * freq + 100.0,
                        j as f32 * CELL * freq + 100.0,
                        seed + o,
                    );
                (amp, freq) = (amp * 0.5, freq * 2.0);
            }
            (0.5 + sum * 0.5) * HMAX // octave amplitudes sum to 1.75, noise tops out near 0.7
        };
        Self(
            (0..(N + 1) * (N + 1))
                .map(|k| height(k % (N + 1), k / (N + 1)))
                .collect(),
        )
    }

    fn h(&self, i: usize, j: usize) -> f32 {
        self.0[j * (N + 1) + i]
    }

    /// Ground height at (x, z), exactly the surface `mesh` draws: each cell is split into the triangles
    /// (0,0)(1,0)(0,1) and (1,0)(1,1)(0,1). Outside the grid it clamps to the edge cell.
    fn at(&self, x: f32, z: f32) -> f32 {
        let (u, v) = ((x + HALF) / CELL, (z + HALF) / CELL);
        // `as usize` saturates negatives to 0, so no floor() needed
        let (i, j) = ((u as usize).min(N - 1), (v as usize).min(N - 1));
        let (fx, fz) = (u - i as f32, v - j as f32);
        let (a, b, c, d) = (
            self.h(i, j),
            self.h(i + 1, j),
            self.h(i, j + 1),
            self.h(i + 1, j + 1),
        );
        if fx + fz <= 1.0 {
            a + fx * (b - a) + fz * (c - a)
        } else {
            d + (1.0 - fx) * (c - d) + (1.0 - fz) * (b - d)
        }
    }

    /// Smooth-shaded triangles: each grid point gets a normal from its neighbours' height difference (central
    /// difference) and a color from its height, so lighting blends across triangles.
    fn mesh(&self) -> Mesh {
        let vertex = |i: usize, j: usize| {
            let h = self.h(i, j);
            let dx = self.h((i + 1).min(N), j) - self.h(i.saturating_sub(1), j);
            let dz = self.h(i, (j + 1).min(N)) - self.h(i, j.saturating_sub(1));
            let w = if i == 0 || i == N { CELL } else { 2.0 * CELL };
            let wz = if j == 0 || j == N { CELL } else { 2.0 * CELL };
            let k = h / HMAX;
            Vertex {
                pos: Vec3::new(i as f32 * CELL - HALF, h, j as f32 * CELL - HALF),
                normal: Vec3::new(-dx / w, 1.0, -dz / wz).normalize_or_zero(),
                color: Vec3::new(0.25 + 0.5 * k, 0.6 - 0.2 * k, 0.2 + 0.2 * k),
            }
        };
        let mut v = Vec::new();
        for j in 0..N {
            for i in 0..N {
                for t in [[(0, 0), (1, 0), (0, 1)], [(1, 0), (1, 1), (0, 1)]] {
                    v.extend(t.map(|(di, dj)| vertex(i + di, j + dj)));
                }
            }
        }
        Mesh::new(&v)
    }
}

/// A unit cube centered on the origin.
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

/// Capsule standing on the origin: radius RADIUS, total height HEIGHT. Rings of latitude, smooth normals.
fn capsule_mesh() -> Mesh {
    const LAT: usize = 8; // rings per hemisphere
    const LON: usize = 16;
    let body = HEIGHT - 2.0 * RADIUS;
    // ring k: latitude -90..90 degrees; the top hemisphere is lifted by the cylinder length
    let vert = |k: usize, l: usize| {
        let a = (k as f32 / (2 * LAT) as f32 - 0.5) * TAU * 0.5;
        let t = l as f32 / LON as f32 * TAU;
        let n = Vec3::new(a.cos() * t.cos(), a.sin(), a.cos() * t.sin());
        let lift = if k >= LAT { body } else { 0.0 };
        Vertex {
            pos: Vec3::new(n.x * RADIUS, RADIUS + lift + n.y * RADIUS, n.z * RADIUS),
            normal: n,
            color: Vec3::ONE,
        }
    };
    let mut v = Vec::new();
    for k in 0..2 * LAT {
        for l in 0..LON {
            for (dk, dl) in [(0, 0), (0, 1), (1, 1), (0, 0), (1, 1), (1, 0)] {
                v.push(vert(k + dk, l + dl));
            }
        }
    }
    Mesh::new(&v)
}

fn spawn_cube(w: &mut World, t: Transform, mesh: Handle<Mesh>, mat: Handle<Material>, c: [f32; 3]) {
    w.spawn_with((Scenery, t, mesh, mat, Color(c)));
}

/// Unload the current scene and load the outdoors or the room. Returns where the player should appear.
fn load(w: &mut World, inside: bool) -> Vec3 {
    for e in w.query_of::<(Scenery,)>() {
        w.despawn(e);
    }
    let sh = *w.resource::<Shared>().unwrap();
    let mut rng = Rng(sh.seed);
    let door = |x: f32, y: f32, z: f32| Transform {
        pos: Vec3::new(x, y, z),
        yaw: 0.0,
        scale: Vec3::new(2.0, 3.4, 0.3),
    };
    let brown = [0.5, 0.28, 0.1];

    if inside {
        // border walls, a staircase and some pillars, one cube per visible voxel
        let mut room = Room(vec![false; (RW * RH * RD) as usize]);
        for y in 0..RH {
            for z in 0..RD {
                for x in 0..RW {
                    room.0[Room::ix(x, y, z)] =
                        x == 0 || y == 0 || z == 0 || x == RW - 1 || y == RH - 1 || z == RD - 1;
                }
            }
        }
        for i in 0..4 {
            for z in 3..7 {
                for y in 1..=1 + i {
                    room.0[Room::ix(3 + i, y, z)] = true;
                }
            }
        }
        for _ in 0..8 {
            let (x, z, h) = (
                8 + (rng.f32() * 9.0) as i32,
                3 + (rng.f32() * 11.0) as i32,
                1 + (rng.f32() * 3.0) as i32,
            );
            for y in 1..=h {
                room.0[Room::ix(x, y, z)] = true;
            }
        }
        for y in 0..RH {
            for z in 0..RD {
                for x in 0..RW {
                    let open = |dx, dy, dz| !room.solid(x + dx, y + dy, z + dz);
                    if !room.solid(x, y, z)
                        || !(open(1, 0, 0)
                            || open(-1, 0, 0)
                            || open(0, 1, 0)
                            || open(0, -1, 0)
                            || open(0, 0, 1)
                            || open(0, 0, -1))
                    {
                        continue;
                    }
                    let g = 0.6 + 0.4 * rng.f32();
                    let tint = if y == 0 {
                        [0.8, 0.65, 0.45]
                    } else {
                        [g, g * 0.9, 1.0]
                    };
                    spawn_cube(
                        w,
                        Transform::from_pos(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5),
                        sh.cube,
                        sh.tex,
                        tint,
                    );
                }
            }
        }
        spawn_cube(
            w,
            door(RW as f32 * 0.5, 2.7, RD as f32 - 1.15),
            sh.cube,
            sh.plain,
            brown,
        ); // exit, inside of the +z wall
        w.insert_resource(Level::In(room));
        return Vec3::new(RW as f32 * 0.5, 1.0, RD as f32 - 3.0);
    }

    let terrain = Terrain::new(sh.seed);
    w.spawn_with((Scenery, Transform::default(), sh.ground, sh.plain));

    // Cubes sunk into the ground: the bottom is far below the surface so slopes never expose it.
    for _ in 0..CUBES {
        let x = (rng.f32() * 2.0 - 1.0) * (HALF - 3.0);
        let z = (rng.f32() * 2.0 - 1.0) * (HALF - 3.0);
        let (width, height, c) = (
            0.8 + 1.7 * rng.f32(),
            0.5 + 3.0 * rng.f32(),
            [
                0.3 + 0.7 * rng.f32(),
                0.3 + 0.7 * rng.f32(),
                0.3 + 0.7 * rng.f32(),
            ],
        );
        if x.abs() < 3.0 && z.abs() < 3.0 || (x - BX).abs() < 10.0 && z > BZ - 10.0 && z < BZ + 14.0
        {
            continue; // keep the spawn and the big cube with its doorstep clear
        }
        let (bottom, top) = (terrain.at(x, z) - 4.0, terrain.at(x, z) + height);
        w.spawn_with((
            Scenery,
            Solid,
            Transform {
                pos: Vec3::new(x, (bottom + top) * 0.5, z),
                yaw: 0.0,
                scale: Vec3::new(width, top - bottom, width),
            },
            sh.cube,
            sh.plain,
            Color(c),
        ));
    }

    // Big cube, sunk below its lowest corner so slopes never expose its bottom; a door box sits on its +z face.
    let mut low = f32::MAX;
    for i in 0..=BIG as i32 {
        for j in 0..=BIG as i32 {
            low = low.min(terrain.at(BX - BIG * 0.5 + i as f32, BZ - BIG * 0.5 + j as f32));
        }
    }
    let face = BZ + BIG * 0.5;
    let door_y = terrain.at(BX, face + 0.2);
    let (bottom, top) = (low - 1.0, door_y + BIG_H);
    w.spawn_with((
        Scenery,
        Solid,
        Transform {
            pos: Vec3::new(BX, (bottom + top) * 0.5, BZ),
            yaw: 0.0,
            scale: Vec3::new(BIG, top - bottom, BIG),
        },
        sh.cube,
        sh.tex,
        Color([0.8, 0.75, 0.65]),
    ));
    spawn_cube(
        w,
        door(BX, door_y + 1.2, face + 0.1),
        sh.cube,
        sh.plain,
        brown,
    );
    let out = Vec3::new(BX, terrain.at(BX, face + 2.5), face + 2.5);
    w.insert_resource(Level::Out(terrain));
    out
}

fn setup(w: &mut World, _: f32) {
    let seed = hash(platform::now() as u32 ^ 0xdead_beef);
    let meshes = w.resource_mut::<Assets<Mesh>>().unwrap();
    let (ground, cube, capsule) = (
        meshes.add(Terrain::new(seed).mesh()),
        meshes.add(cube_mesh()),
        meshes.add(capsule_mesh()),
    );
    let mats = w.resource_mut::<Assets<Material>>().unwrap();
    let (plain, tex) = (mats.add(Material::lit(LIT)), mats.add(Material::lit(TEX)));
    w.insert_resource(Shared {
        ground,
        cube,
        plain,
        tex,
        seed,
    });

    load(w, false);
    let Some(Level::Out(t)) = w.resource::<Level>() else {
        return;
    };
    let start = Vec3::new(0.0, t.at(0.0, 0.0), 0.0);
    w.spawn_with((
        Player,
        Transform {
            pos: start,
            ..Transform::default()
        },
        Velocity {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            grounded: false,
            jumped: false,
        },
        capsule,
        plain,
        Color([0.95, 0.45, 0.2]),
    ));
    camera(w).pitch = -0.4;
}

/// Mouse orbits the camera around the player.
fn look(w: &mut World, _: f32) {
    let d = w.resource::<Input>().unwrap().mouse_delta();
    let c = camera(w);
    c.yaw -= d[0] * SENSITIVITY;
    c.pitch = (c.pitch - d[1] * SENSITIVITY).clamp(-1.4, 0.3);
}

/// WASD relative to the camera heading; gravity; collide with whichever scene is loaded.
fn walk(w: &mut World, dt: f32) {
    let input = *w.resource::<FixedInput>().unwrap();
    let yaw = camera(w).yaw;
    let (fwd, side) = (input.axis(&[KeyW], &[KeyS]), input.axis(&[KeyD], &[KeyA]));
    let k = if fwd != 0.0 && side != 0.0 {
        FRAC_1_SQRT_2
    } else {
        1.0
    };
    let (s, c) = (yaw.sin(), yaw.cos());
    let (ax, az) = (
        (-s * fwd + c * side) * k * ACCEL,
        (-c * fwd - s * side) * k * ACCEL,
    );

    for e in w.query_of::<(Player, Transform, Velocity)>() {
        let (mut t, mut v) = {
            let (t, v) = w.getn_mut::<(Transform, Velocity)>(e).unwrap();
            (*t, *v)
        };
        let level = w.resource::<Level>().unwrap();
        if v.grounded {
            v.jumped = false;
        }
        // close enough to the ground to still jump
        let near = match level {
            Level::Out(g) => t.pos.y - g.at(t.pos.x, t.pos.z) < JUMP_SLACK,
            Level::In(room) => room.hits([t.pos.x, t.pos.y - JUMP_SLACK, t.pos.z]),
        };
        if input.just_pressed(Space) && !v.jumped && (v.grounded || near) {
            (v.y, v.jumped) = (JUMP_FORCE, true);
        }
        v.y -= GRAVITY * dt;

        if fwd != 0.0 || side != 0.0 {
            (v.x, v.z) = (v.x + ax * dt, v.z + az * dt);
        }
        let speed = (v.x * v.x + v.z * v.z).sqrt();
        let target = if fwd != 0.0 || side != 0.0 {
            speed.min(MAX_SPEED)
        } else {
            (speed - FRICTION * dt).max(0.0)
        };
        if speed > 0.0 {
            (v.x, v.z) = (v.x * target / speed, v.z * target / speed);
        }

        match level {
            Level::In(room) => {
                let mut p = t.pos.to_array();
                let mut vel = [v.x, v.y, v.z];
                room.step(&mut p, &mut vel, dt);
                // standing = a thin probe under the feet touches a voxel (independent of which axis resolved last)
                v.grounded = vel[1] <= 0.0 && room.hits([p[0], p[1] - 0.02, p[2]]);
                (t.pos, (v.x, v.y, v.z)) = (p.into(), (vel[0], vel[1], vel[2]));
            }
            Level::Out(g) => {
                let lim = HALF - RADIUS;
                t.pos.x = (t.pos.x + v.x * dt).clamp(-lim, lim);
                t.pos.z = (t.pos.z + v.z * dt).clamp(-lim, lim);
                t.pos.y += v.y * dt;
                let ground = g.at(t.pos.x, t.pos.z);
                v.grounded = false; // collide() sets it again when we land on a cube
                if t.pos.y <= ground {
                    t.pos.y = ground;
                    v.y = v.y.max(0.0);
                    v.grounded = true;
                }
            }
        }
        let (tt, vv) = w.getn_mut::<(Transform, Velocity)>(e).unwrap();
        (*tt, *vv) = (t, v);
    }
}

/// Push the player out of cubes along the least-penetrated axis (AABB vs the capsule's bounding box).
fn collide(w: &mut World, _: f32) {
    let solids = w.query_of::<(Solid, Transform)>();
    for p in w.query_of::<(Player, Transform, Velocity)>() {
        let mut pos = w.get::<Transform>(p).unwrap().pos;
        for &s in &solids {
            let t = w.get::<Transform>(s).unwrap();
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
                    v.grounded = true;
                    v.y = v.y.max(0.0);
                } else {
                    v.y = v.y.min(0.0);
                }
            }
        }
        w.get_mut::<Transform>(p).unwrap().pos = pos;
    }
}

/// E at a door: unload this scene, load the other, drop the player in at its entrance.
fn interact(w: &mut World, _: f32) {
    if !w.resource::<Input>().unwrap().just_pressed(KeyE) {
        return;
    }
    let Some(&p) = w.query_of::<(Player, Transform)>().first() else {
        return;
    };
    let pos = w.get::<Transform>(p).unwrap().pos;
    let inside = matches!(w.resource::<Level>(), Some(Level::In(_)));
    let (dx, dz) = if inside {
        (RW as f32 * 0.5, RD as f32 - 1.0)
    } else {
        (BX, BZ + BIG * 0.5)
    };
    if (pos.x - dx) * (pos.x - dx) + (pos.z - dz) * (pos.z - dz) > DOOR_REACH * DOOR_REACH {
        return;
    }
    let at = load(w, !inside);
    w.get_mut::<Transform>(p).unwrap().pos = at;
    *w.get_mut::<Velocity>(p).unwrap() = Velocity {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        grounded: false,
        jumped: false,
    };
    camera(w).yaw = if inside { PI } else { 0.0 }; // face away from the door you came through
}

/// Third person: sit CAM_DIST behind the view direction; outdoors stay above the ground, indoors stop before a wall.
fn follow(w: &mut World, _: f32) {
    let Some(&p) = w.query_of::<(Player, Transform)>().first() else {
        return;
    };
    let target = w.get::<Transform>(p).unwrap().pos + Vec3::new(0.0, CAM_LOOK_AT, 0.0);
    let (yaw, pitch) = {
        let c = camera(w);
        (c.yaw, c.pitch)
    };
    let f = [
        -yaw.sin() * pitch.cos(),
        pitch.sin(),
        -yaw.cos() * pitch.cos(),
    ];
    let at = |d: f32| {
        Vec3::new(
            target.x - f[0] * d,
            target.y - f[1] * d,
            target.z - f[2] * d,
        )
    };
    let pos = match w.resource::<Level>().unwrap() {
        Level::Out(g) => {
            let mut pos = at(CAM_DIST);
            pos.y = pos.y.max(g.at(pos.x, pos.z) + 0.5);
            pos
        }
        Level::In(room) => {
            // coarse march to the first wall voxel, then bisect: the distance is continuous, so no stepping jitter
            let wall = |d: f32| {
                let q = at(d);
                room.solid(fl(q.x), fl(q.y), fl(q.z))
            };
            let (mut lo, mut hi) = (0.0, CAM_DIST);
            let mut hit = false;
            for k in 1..=32 {
                let d = CAM_DIST * k as f32 / 32.0;
                if wall(d) {
                    (hi, hit) = (d, true);
                    break;
                }
                lo = d;
            }
            if hit {
                for _ in 0..8 {
                    let mid = (lo + hi) * 0.5;
                    if wall(mid) { hi = mid } else { lo = mid }
                }
                lo = (lo - 0.3).max(0.0); // keep the near plane off the wall
            }
            at(lo)
        }
    };
    camera(w).pos = pos;
}

fn game() {
    let mut app = App::new();
    app.add_system(Startup, setup)
        .add_system(Update, look)
        .add_system(Fixed, walk)
        .add_system(Fixed, collide)
        .add_system(Update, interact)
        .add_system(Update, follow);
    app.world.spawn_with((Camera3d::default(),)); // the engine only auto-spawns one when the app has no camera
    hud::add(&mut app);
    app.world.insert_resource(MouseCapture(true));
    app.run(c"3d game");
}

platform::entry!(game);
