//! Three buttons, each drawn from an image in a different format (PNG, JPEG, WebP). Pressing one plays a sound
//! decoded from a different format (WAV, MP3, Ogg Vorbis). Each button dims while it is held.
//! Like sprite-example, every image pixel is a 1x1 quad: no textures in gfx yet.
//! Startup queues all six files in a `media::Loader` (ffmpeg decodes them in the background) and shows a black
//! screen with a loading bar drawn by `loading.wgsl`; the buttons appear once everything is `Ready`.
//! A file that fails ends the program with its error code (703: ffmpeg missing, 702/704: bad image/sound).
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use audio::{Audio, Sample};
use ecs::{App, Entity, Schedule::*, World};
use gfx::{Assets, Camera2d, Color, Input, Material, Mesh, PostProcess, RunExt, Transform};
use media::{Clip, Id, Image, Loader};
use platform::die;
use shader::{Align4, include_shader};

static LOADING: &Align4<[u8]> = include_shader!("loading");

const SIZE: u32 = 16; // buttons are 16x16 images, one world unit per pixel
const GAP: f32 = 4.0;
const HEIGHT: f32 = 48.0; // world units top to bottom: three buttons fit even at 4:3

struct Button {
    img: Image,
    px: Vec<Entity>, // SIZE * SIZE quads, row by row
    x: f32,          // center; every button is centered vertically on y = 0
    sound: Sample,
    pressed: bool,
}

struct Buttons(Vec<Button>);

/// The files being loaded: (image, sound) per button, left to right. `shown` is the bar's eased progress.
struct Loading {
    loader: Loader,
    files: [(Id, Id); 3],
    shown: f32,
}

/// The camera never moves, so the same value serves the scene and the hit test.
fn camera() -> Camera2d {
    Camera2d {
        height: HEIGHT,
        ..Camera2d::default()
    }
}

fn quad_mesh() -> Mesh {
    let v = math::Vec3::new;
    Mesh::new(&[
        v(-0.5, -0.5, 0.0),
        v(0.5, -0.5, 0.0),
        v(0.5, 0.5, 0.0),
        v(-0.5, -0.5, 0.0),
        v(0.5, 0.5, 0.0),
        v(-0.5, 0.5, 0.0),
    ])
}

/// The audio thread keeps the samples for good, so they are leaked.
fn sample(c: Clip) -> Sample {
    Sample {
        rate: c.rate,
        channels: c.channels,
        pcm: Vec::leak(c.pcm),
    }
}

/// Queue everything and show the loading screen. Nothing else is on screen until `load` is done.
fn setup(w: &mut World, _: f32) {
    let mut loader = Loader::new();
    // image format -> sound format, left to right
    let files = [
        (loader.image("red.png"), loader.sound("c5.wav")),
        (loader.image("green.jpg"), loader.sound("e5.mp3")),
        (loader.image("blue.webp"), loader.sound("g5.ogg")),
    ];
    w.insert_resource(Loading {
        loader,
        files,
        shown: 0.0,
    });
    *w.resource_mut::<PostProcess>().unwrap() = PostProcess(Some(LOADING), 0.0);
}

/// Drives the loader until it is done; the bar eases toward the real progress so it never jumps.
fn load(w: &mut World, dt: f32) {
    if w.resource::<Buttons>().is_some() {
        return; // already built
    }
    let l = w.resource_mut::<Loading>().unwrap();
    l.loader.poll();
    if let Some(e) = l.loader.failed() {
        die(e);
    }
    l.shown += (l.loader.progress() - l.shown) * (dt * 10.0).min(1.0);
    let (shown, done) = (l.shown, l.loader.done());
    w.resource_mut::<PostProcess>().unwrap().1 = shown;
    if done && shown > 0.99 {
        // everything is Ready (a failure died above), so the takes cannot miss
        let l = w.resource_mut::<Loading>().unwrap();
        let loaded: Vec<(Image, Clip)> = l
            .files
            .iter()
            .map(|&(i, s)| {
                (
                    l.loader.take_image(i).unwrap(),
                    l.loader.take_clip(s).unwrap(),
                )
            })
            .collect();
        build(w, loaded);
        *w.resource_mut::<PostProcess>().unwrap() = PostProcess(None, 0.0);
    }
}

fn build(w: &mut World, loaded: Vec<(Image, Clip)>) {
    let quad = w.resource_mut::<Assets<Mesh>>().unwrap().add(quad_mesh());
    let material = w
        .resource_mut::<Assets<Material>>()
        .unwrap()
        .add(Material::flat());
    let half = (SIZE - 1) as f32 / 2.0; // centers a button on its x; image y grows down, world y grows up
    let buttons = loaded
        .into_iter()
        .enumerate()
        .map(|(n, (img, clip))| {
            let x = (n as f32 - 1.0) * (SIZE as f32 + GAP);
            let px = (0..SIZE * SIZE)
                .map(|i| {
                    let (px, py) = ((i % SIZE) as f32, (i / SIZE) as f32);
                    w.spawn_with((
                        Transform::from_pos(x + px - half, half - py, 0.0),
                        quad,
                        material,
                        Color([0.0; 3]),
                    ))
                })
                .collect();
            Button {
                img,
                px,
                x,
                sound: sample(clip),
                pressed: false,
            }
        })
        .collect();
    w.insert_resource(Buttons(buttons));
    for n in 0..3 {
        paint(w, n, false);
    }
}

/// Draw button `n`'s image onto its quads. Transparent pixels get scale 0 (no blending: alpha is on/off).
fn paint(w: &mut World, n: usize, dim: bool) {
    let b = &w.resource::<Buttons>().unwrap().0[n];
    let k = (if dim { 0.6 } else { 1.0 }) / 255.0;
    let paint: Vec<(Entity, [f32; 3], f32)> =
        b.px.iter()
            .enumerate()
            .map(|(i, &e)| {
                let [r, g, bl, a] = b.img.px(i as u32 % SIZE, i as u32 / SIZE);
                (e, [r, g, bl].map(|c| c as f32 * k), (a >= 128) as u8 as f32)
            })
            .collect();
    for (e, c, scale) in paint {
        w.get_mut::<Color>(e).unwrap().0 = c;
        w.get_mut::<Transform>(e).unwrap().scale = math::Vec3::splat(scale);
    }
}

/// A press inside a button plays its sound (on the way down, not on release).
fn click(w: &mut World, _: f32) {
    if w.resource::<Buttons>().is_none() {
        return; // still loading
    }
    let input = *w.resource::<Input>().unwrap();
    let at = input.cursor_world(&camera());
    for n in 0..3 {
        let (x, pressed, sound) = {
            let b = &w.resource::<Buttons>().unwrap().0[n];
            (b.x, b.pressed, b.sound)
        };
        let half = SIZE as f32 / 2.0;
        let over = at.is_some_and(|p| (p.x - x).abs() <= half && p.y.abs() <= half);
        if over && input.clicked() {
            w.resource_mut::<Audio>().unwrap().play_sample(sound, 1.0);
        }
        let held = over && input.mouse_down();
        if held != pressed {
            w.resource_mut::<Buttons>().unwrap().0[n].pressed = held;
            paint(w, n, held);
        }
    }
}

fn game() {
    let mut app = App::new();
    app.add_system(Startup, setup)
        .add_system(Update, load)
        .add_system(Update, click);
    audio::add(&mut app);
    app.world.spawn_with((camera(),));
    app.run(c"Media: click a button, Esc quits");
}

platform::entry!(game);
