//! One-octave piano. Keys: white A S D F G H J = C D E F G A B, black W E T Y U = C# D# F# G# A#.
#![no_std]
#![no_main]

use audio::Audio;
use ecs::{App, Entity, Schedule};
use gfx::{Assets, Camera2d, Color, Handle, Input, Material, Mesh, RunExt, Transform};
use math::{FloatExt, Vec3};
use platform::input::*;

// Index = semitone from C4.
const KEYS: [u8; 12] = [
    KeyA, KeyW, KeyS, KeyE, KeyD, KeyF, KeyT, KeyG, KeyY, KeyH, KeyU, KeyJ,
];
fn hz(n: usize) -> f32 {
    261.63 * (n as f32 / 12.0).exp2() // equal temperament from C4
}
const X: [f32; 12] = [0.5, 1.0, 1.5, 2.0, 2.5, 3.5, 4.0, 4.5, 5.0, 5.5, 6.0, 6.5]; // white keys are 1 wide
const fn black(n: usize) -> bool {
    matches!(n, 1 | 3 | 6 | 8 | 10)
}

struct Piano([Entity; 12]);

fn color(n: usize, down: bool) -> Color {
    Color(match (black(n), down) {
        (false, false) => [0.95, 0.95, 0.92],
        (true, false) => [0.08, 0.08, 0.1],
        (false, true) => [0.5, 0.72, 1.0],
        (true, true) => [0.25, 0.45, 0.85],
    })
}

unsafe fn run() {
    let mut app = App::new();
    app.world.spawn_with((Camera2d {
        pos: math::Vec2::new(3.5, 0.0),
        height: 6.0,
        sky: [0.1, 0.1, 0.12],
        ..Default::default()
    },));

    app.add_system(Schedule::Startup, |w, _| {
        let sq = [
            (-0.5, -0.5),
            (0.5, -0.5),
            (0.5, 0.5),
            (-0.5, -0.5),
            (0.5, 0.5),
            (-0.5, 0.5),
        ]
        .map(|(x, y)| Vec3::new(x, y, 0.0));
        let mesh: Handle<Mesh> = w
            .resource_mut::<Assets<Mesh>>()
            .unwrap()
            .add(Mesh::new(&sq));
        let mat: Handle<Material> = w
            .resource_mut::<Assets<Material>>()
            .unwrap()
            .add(Material::flat());
        let ents = core::array::from_fn(|n| {
            let (size, y, z) = if black(n) {
                ((0.6, 2.5), 0.75, 1.0)
            } else {
                ((0.94, 4.0), 0.0, 0.0)
            };
            let mut t = Transform::from_pos(X[n], y, z);
            t.scale = Vec3::new(size.0, size.1, 1.0);
            w.spawn_with((t, color(n, false), mesh, mat))
        });
        w.insert_resource(Piano(ents));
    });

    app.add_system(Schedule::Update, |w, _| {
        let input = *w.resource::<Input>().unwrap();
        let ents = w.resource::<Piano>().unwrap().0;
        for n in 0..12 {
            let down = input.is_holding(KEYS[n]);
            w.resource_mut::<Audio>().unwrap().tone(n, hz(n), down);
            *w.get_mut::<Color>(ents[n]).unwrap() = color(n, down);
        }
    });
    audio::add(&mut app);
    app.run(c"piano");
}

platform::entry!(run);
