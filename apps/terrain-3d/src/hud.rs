//! HUD layer: a `Camera2d` whose viewport is a fixed-size pixel box in the window's top-left corner, drawn over the
//! 3D view. For now its only widget is an FPS-over-time bar graph with the current fps as bitmap-font digits.
//!
//! Every camera draws the whole scene, so the HUD entities sit far from the world (at x = FAR) and the HUD camera
//! looks there: the 3D camera (far plane 200) culls them, and the HUD camera culls the terrain.
use alloc::string::ToString;
use ecs::{App, Schedule::*, World};
use gfx::{Assets, Atlas, Camera2d, Color, Handle, Input, Material, Mesh, Transform};
use math::{FloatExt, Vec2, Vec3};
use platform::die;

const FAR: f32 = 1000.0;
const BOX: [f32; 2] = [240.0, 80.0]; // widget size in pixels
const MARGIN: f32 = 8.0;
const N: usize = 120; // samples shown
// Acceptable range: at or above FPS_GOOD is green, below it orange, below FPS_BAD red.
const FPS_GOOD: f32 = 55.0;
const FPS_BAD: f32 = 30.0;
const FPS_SCALE: f32 = 60.0; // the graph's top is at least this; it grows to the highest visible sample, so nothing clips

fn tint(fps: f32) -> [f32; 3] {
    if fps < FPS_BAD {
        [0.95, 0.2, 0.2]
    } else if fps < FPS_GOOD {
        [1.0, 0.6, 0.15]
    } else {
        [0.3, 0.9, 0.4]
    }
}

struct Bar(usize);
struct Samples([f32; N]); // fps, oldest first
struct HudCam;
struct Digit(usize); // slot of the fps readout, 0 = leftmost
struct Glyphs([Handle<Mesh>; 10]); // one mesh per digit, 0..=9
struct Shown {
    fps: f32, // what the readout says; refreshed 4x a second so it can be read
    t: f32,
}

const FONT: &str = "ComicMono.ttf"; // TTF / OTF / WOFF in the assets directory
const FONT_PX: u32 = 32; // glyph grid resolution
const DIGITS: &str = "0123456789";
const TEXT: f32 = 0.2; // digit height (one em) in world units: 16 px of the 80 px box
const SLOTS: usize = 3;

fn quad() -> Mesh {
    let c = |x: f32, y: f32| Vec3::new(x, y, 0.0);
    let (a, b, d, e) = (c(-0.5, -0.5), c(0.5, -0.5), c(0.5, 0.5), c(-0.5, 0.5));
    Mesh::new(&[a, b, d, a, d, e])
}

fn setup(w: &mut World, _: f32) {
    let mesh = w.resource_mut::<Assets<Mesh>>().unwrap().add(quad());
    let mat = w
        .resource_mut::<Assets<Material>>()
        .unwrap()
        .add(Material::flat());
    // the box is 1 world unit tall, so it is BOX[0] / BOX[1] wide
    let width = BOX[0] / BOX[1];
    let bg = Transform {
        pos: Vec3::new(FAR, 0.0, -1.0),
        yaw: 0.0,
        scale: Vec3::new(width, 1.0, 1.0),
    };
    w.spawn_with((bg, mesh, mat, Color([0.05, 0.05, 0.08])));
    for i in 0..N {
        let t = Transform {
            pos: Vec3::new(
                FAR + (i as f32 + 0.5) * width / N as f32 - width / 2.0,
                0.0,
                0.0,
            ),
            yaw: 0.0,
            scale: Vec3::new(width / N as f32, 0.0, 1.0),
        };
        w.spawn_with((Bar(i), t, mesh, mat, Color([0.3, 0.9, 0.4])));
    }
    w.insert_resource(Samples([0.0; N]));

    // digits: the webview draws FONT into a glyph grid, `media::Font` lays it out, `Material::msdf` draws it
    // (every channel = alpha)
    let id = media::font(FONT, DIGITS, FONT_PX);
    media::wait();
    let img = media::take_image(id).unwrap_or_else(|| die(media::failed().unwrap()));
    let font = media::Font::new(&img, DIGITS, 0);
    let advance = (img.w / media::COLS) as f32 / img.h as f32; // glyph cell width in ems (one row of cells)
    w.insert_resource(Atlas {
        w: img.w,
        h: img.h,
        rgb: media::alpha_atlas(&img.rgba).leak(),
        nearest: false, // smooth TTF edges, unlike a pixel font
    });
    let text = w
        .resource_mut::<Assets<Material>>()
        .unwrap()
        .add(Material::msdf());
    let meshes = w.resource_mut::<Assets<Mesh>>().unwrap();
    let glyphs: [Handle<Mesh>; 10] = core::array::from_fn(|d| {
        let c = char::from(b'0' + d as u8);
        meshes.add(Mesh::new(&font.layout(c.encode_utf8(&mut [0; 4]), TEXT)))
    });
    for k in 0..SLOTS {
        let t = Transform {
            pos: Vec3::new(
                FAR - width / 2.0 + 0.04 + k as f32 * TEXT * advance,
                0.5 - 0.04 - TEXT,
                1.0,
            ),
            yaw: 0.0,
            scale: Vec3::new(0.0, 1.0, 1.0), // hidden until it has a digit
        };
        w.spawn_with((Digit(k), t, glyphs[0], text, Color([1.0; 3])));
    }
    w.insert_resource(Glyphs(glyphs));
    w.insert_resource(Shown { fps: 0.0, t: 1.0 });
}

fn update(w: &mut World, dt: f32) {
    let win = w.resource::<Input>().unwrap().window_size();
    let s = w.resource_mut::<Samples>().unwrap();
    s.0.rotate_left(1);
    s.0[N - 1] = 1.0 / dt.max(1e-4);
    let fps = s.0;

    // pin the viewport to the top-left corner at a fixed pixel size (viewport y counts up from the bottom)
    let e = w.query_of::<(HudCam, Camera2d)>()[0];
    let c = w.get_mut::<Camera2d>(e).unwrap();
    c.viewport = [
        MARGIN / win[0],
        1.0 - (MARGIN + BOX[1]) / win[1],
        BOX[0] / win[0],
        BOX[1] / win[1],
    ];

    let n = w.resource_mut::<Shown>().unwrap();
    n.t += dt;
    if n.t >= 0.25 {
        n.t = 0.0;
        n.fps = fps[N - 15..].iter().sum::<f32>() / 15.0; // mean of the last 15 frames
    }
    let shown = n.fps;
    let digits = (n.fps.round() as u32).min(999).to_string(); // SLOTS digits at most
    let glyphs = w.resource::<Glyphs>().unwrap().0;
    for e in w.query_of::<(Digit, Handle<Mesh>, Transform)>() {
        let k = w.get::<Digit>(e).unwrap().0;
        let d = digits.as_bytes().get(k).map(|c| (c - b'0') as usize);
        if let Some(d) = d {
            *w.get_mut::<Handle<Mesh>>(e).unwrap() = glyphs[d];
        }
        w.get_mut::<Transform>(e).unwrap().scale.x = d.is_some() as u8 as f32;
        w.get_mut::<Color>(e).unwrap().0 = tint(shown);
    }

    let top = fps.iter().copied().fold(FPS_SCALE, f32::max);
    for e in w.query_of::<(Bar, Transform)>() {
        let i = w.get::<Bar>(e).unwrap().0;
        let h = fps[i] / top;
        w.get_mut::<Color>(e).unwrap().0 = tint(fps[i]);
        let t = w.get_mut::<Transform>(e).unwrap();
        (t.scale.y, t.pos.y) = (h, (h - 1.0) * 0.5);
    }
}

pub fn add(app: &mut App) {
    app.add_system(Startup, setup).add_system(Update, update);
    app.world.spawn_with((
        HudCam,
        Camera2d {
            pos: Vec2::new(FAR, 0.0),
            height: 1.0,
            sky: [0.0; 3],
            ..Default::default()
        },
    ));
}
