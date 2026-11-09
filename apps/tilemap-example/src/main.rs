//! Layered RPG Maker tilemaps (the `tilemap` crate): autotile and tile layers drawn by fragment shaders, y-sorted
//! sprites (trees, signs, towers) the hero walks behind, overlays above them; dialogue and map changes with a fade.
//! WASD / arrows walk, Shift runs, Q / E zoom (whole-pixel steps), Z talks to signs (turns the page, takes the picked
//! answer; W / S choose), 1-9 hide / show a layer (debug), Esc quits. Solid ground and objects block.
//! A level is assets/<name>.json in this app (it names its own image files), baked by build.rs (`tilemap::build`) into the workspace
//! assets/<name>.level and loaded at runtime like the images: ground kinds and object cells as two small textures,
//! plus invisible events (Z to talk, or step on a door to change map). Dialogue is a pixel font
//! (assets/pixel.png, as in bitmap-font-example) on flat quads that `view` pins to the bottom of the
//! screen in whole screen pixels, whatever the zoom. The fade is a post-process shader (src/fade.wgsl).
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{rc::Rc, string::String, vec::Vec};
use core::f32::consts::FRAC_1_SQRT_2;

use ecs::{App, Entity, Schedule::*, World};
use gfx::{
    Assets, Atlas, Camera2d, Color, Handle, Input, Material, Mesh, PostProcess, RunExt, Transform,
};
use math::{Vec2, Vec3};
use platform::{
    die,
    input::{
        ArrowDown, ArrowLeft, ArrowRight, ArrowUp, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6,
        Digit7, Digit8, Digit9, KeyA, KeyD, KeyE, KeyQ, KeyS, KeyW, KeyZ, ShiftLeft,
    },
};
use shader::{Align4, include_shader};
use tilemap::{Action, END, Level, Node, Prepared, Sheet, Sheets};

static FADE: &Align4<[u8]> = include_shader!("fade");

const TILE_PX: f32 = 48.0; // a tile in the sheet; world units are tiles
const WALK: f32 = 4.5; // tiles per second
const RUN: f32 = 8.0;
const FOLLOW: f32 = 8.0; // camera catch-up speed, 1 / seconds
const REACH: f32 = 1.1; // how close to a sign Z talks to it, tiles
const FADE_TIME: f32 = 0.45; // seconds to fade out, and again to fade in
const EM: f32 = 16.0; // dialogue text: one em in screen pixels (2 x the font's 8 px)
const WRAP_EM: f32 = 34.0; // text width in ems
const BOX_W: f32 = WRAP_EM * EM + 32.0; // dialogue box in screen pixels
const BOX_MIN: f32 = 112.0; // its least height: 4 lines + padding; it grows for longer texts
const INDENT: f32 = 2.0; // ems before an answer, room for the marker
const GAP: f32 = 16.0; // box to screen edge, and text to box edge
const HALF: Vec2 = Vec2::new(0.28, 0.14); // half size of the feet box, tiles
const BODY_W: f32 = 0.5; // the hero's drawn width, tiles (see `person`)
const SEE_THROUGH: f32 = 0.6; // opacity of a sprite or structure while it hides the hero

/// A script node's text, laid out once: question or line first, then the answers, all in one mesh (the engine
/// draws only a few distinct meshes per frame).
struct View {
    mesh: Handle<Mesh>,
    answers: Vec<f32>, // baseline of each answer, ems relative to the first baseline (negative: down)
    height: f32,       // ems, all lines
}
/// A level that has been loaded, kept so going back and forth costs nothing.
struct Map {
    name: String,
    level: Level,
    drawn: Rc<Prepared>, // its materials; shared, so entering the level again only spawns entities
    views: Vec<View>,    // by node index
}
struct Maps {
    list: Vec<Map>,
    cur: usize,
    entities: Vec<Entity>, // what the current map spawned: layers and sprites
    font: Rc<media::Font>, // the dialogue font, a grid in pixel.png
}
struct Talk {
    at: Option<(u32, usize)>, // (node, picked answer)
    ents: Vec<Entity>,        // the open dialogue's box, text and marker
    marker: Option<Entity>,   // beside the picked answer
    text_y: f32,              // first baseline, pixels from the bottom
}
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Idle,
    Out, // fading to black
    In,  // fading back
}
struct Fade {
    phase: Phase,
    t: f32,                         // 0 = scene, 1 = black
    to: Option<(String, u32, u32)>, // the map change under way: level asset, cell
    armed: bool, // false after arriving, until the hero has left the door's cells
}
#[derive(Clone, Copy)]
struct Ui {
    quad: Handle<Mesh>,
    flat: Handle<Material>,
    text: Handle<Material>,
}
/// Pinned to the screen: `off` is in pixels from the window's bottom center (x right, y up), `size` in pixels.
/// Text meshes are in ems, so their `size` is (EM, EM).
struct Hud {
    off: Vec2,
    size: Vec2,
}

struct Hero {
    pos: Vec2, // feet, world units (x right, y up)
}
struct Cam {
    pos: Vec2, // smoothed, before pixel snapping
    zoom: f32, // whole screen pixels per sheet pixel
    ready: bool,
}
struct Part {
    off: Vec2, // from the feet
    z: f32,    // among the figure's parts, added to the depth of its feet
}

fn quad_mesh() -> Mesh {
    let v = Vec3::new;
    Mesh::new(&[
        v(-0.5, -0.5, 0.0),
        v(0.5, -0.5, 0.0),
        v(0.5, 0.5, 0.0),
        v(-0.5, -0.5, 0.0),
        v(0.5, 0.5, 0.0),
        v(-0.5, 0.5, 0.0),
    ])
}

fn cur(w: &World) -> &Map {
    let m = w.resource::<Maps>().unwrap();
    &m.list[m.cur]
}

/// How `tilemap` loads the image files a level names.
fn load_sheet(name: &str) -> Sheet {
    let i = media::load_image(name).unwrap_or_else(|e| die(e));
    Sheet {
        w: i.w,
        h: i.h,
        rgba: i.rgba,
    }
}

/// The index of level asset `name` in `Maps`, loading it the first time.
fn add_map(w: &mut World, name: &str) -> usize {
    if let Some(i) = w
        .resource::<Maps>()
        .unwrap()
        .list
        .iter()
        .position(|m| m.name == name)
    {
        return i;
    }
    let level = tilemap::load(name);
    let drawn = Rc::new(tilemap::prepare(w, &level));
    let font = w.resource::<Maps>().unwrap().font.clone();
    // every node's text, laid out once; answers start on their own lines, indented
    let views = level
        .nodes
        .iter()
        .map(|n| {
            let (mut verts, mut pen, mut answers) = (Vec::new(), [0.0, 0.0], Vec::new());
            let (text, options) = match n {
                Node::Say { text, .. } => (text, &[][..]),
                Node::Ask { text, options } => (text, options.as_slice()),
            };
            for word in text.split_whitespace() {
                font.push_word(&mut verts, &mut pen, word, 1.0, WRAP_EM);
            }
            for o in options {
                pen = [INDENT, pen[1] - font.line];
                answers.push(pen[1]);
                for word in o.label.split_whitespace() {
                    font.push_word(&mut verts, &mut pen, word, 1.0, WRAP_EM);
                }
            }
            View {
                mesh: w
                    .resource_mut::<Assets<Mesh>>()
                    .unwrap()
                    .add(Mesh::new(&verts)),
                answers,
                height: font.line - pen[1],
            }
        })
        .collect();
    let maps = w.resource_mut::<Maps>().unwrap();
    maps.list.push(Map {
        name: String::from(name),
        level,
        drawn,
        views,
    });
    maps.list.len() - 1
}

/// A little person from flat quads: (offset from the feet, size, z among the parts, color); no sprite sheet needed.
/// The parts follow `Hero` and sort by depth among the level's sprites (see `view`).
fn person(w: &mut World, ui: Ui, at: Vec2, body: [f32; 3], hair: [f32; 3]) {
    let parts = [
        (
            Vec2::new(0.0, 0.02),
            Vec2::new(0.50, 0.14),
            0.000,
            [0.20, 0.42, 0.14],
        ), // shadow
        (Vec2::new(0.0, 0.30), Vec2::new(0.34, 0.34), 0.001, body),
        (
            Vec2::new(0.0, 0.62),
            Vec2::new(0.30, 0.28),
            0.002,
            [0.96, 0.78, 0.62],
        ), // head
        (Vec2::new(0.0, 0.74), Vec2::new(0.34, 0.12), 0.003, hair),
    ];
    for (off, size, z, rgb) in parts {
        let t = Transform {
            scale: size.extend(1.0),
            ..Transform::from_pos(at.x + off.x, at.y + off.y, tilemap::depth(at.y) + z)
        };
        w.spawn_with((Part { off, z }, t, ui.quad, ui.flat, Color(rgb)));
    }
}

fn setup(w: &mut World, _: f32) {
    media::spawn(); // the startup loads (the font, then the level's images) share one webview
    w.insert_resource(Sheets::new(load_sheet));
    let img = media::load_image("pixel.png").unwrap_or_else(|e| die(e));
    w.insert_resource(Maps {
        list: Vec::new(),
        cur: 0,
        entities: Vec::new(),
        font: Rc::new(media::Font::new(&img, media::ASCII, 1)), // 10 px cells, 8 px glyphs
    });

    // every channel is the PNG's alpha
    w.insert_resource(Atlas {
        w: img.w,
        h: img.h,
        rgb: media::alpha_atlas(&img.rgba).leak(),
        nearest: true,
    });
    let quad = w.resource_mut::<Assets<Mesh>>().unwrap().add(quad_mesh());
    let mats = w.resource_mut::<Assets<Material>>().unwrap();
    let ui = Ui {
        quad,
        flat: mats.add(Material::flat()),
        text: mats.add(Material::msdf()),
    };
    w.insert_resource(ui);
    w.insert_resource(Talk {
        at: None,
        ents: Vec::new(),
        marker: None,
        text_y: 0.0,
    });
    w.insert_resource(Fade {
        phase: Phase::Idle,
        t: 0.0,
        to: None,
        armed: true,
    });
    *w.resource_mut::<PostProcess>().unwrap() = PostProcess(Some(FADE), 0.0);

    let first = add_map(w, "town.level");
    media::despawn(); // later maps load on demand, each with a webview that comes and goes
    let (drawn, h) = {
        let m = &w.resource::<Maps>().unwrap().list[first];
        (m.drawn.clone(), m.level.h)
    };
    let entities = tilemap::spawn(w, &drawn);
    w.resource_mut::<Maps>().unwrap().entities = entities;

    let hero = Vec2::new(12.5, h as f32 - 13.5);
    person(w, ui, hero, [0.22, 0.35, 0.75], [0.35, 0.20, 0.10]);
    w.insert_resource(Hero { pos: hero });
    w.insert_resource(Cam {
        pos: Vec2::ZERO,
        zoom: 1.0,
        ready: false,
    });
}

/// Does the hero's feet box at `p` touch a solid cell (or the map's edge)? Level row 0 is the top, world y is up.
fn hits(m: &Level, p: Vec2) -> bool {
    [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)]
        .iter()
        .any(|&(sx, sy)| {
            let (x, y) = (p.x + sx * HALF.x, p.y + sy * HALF.y);
            x < 0.0 || y < 0.0 || m.solid_at(x as i32, m.h as i32 - 1 - y as i32)
        })
}

fn close(w: &mut World) {
    let t = w.resource_mut::<Talk>().unwrap();
    t.at = None;
    t.marker = None;
    for e in core::mem::take(&mut t.ents) {
        w.despawn(e);
    }
}

/// Where the marker sits for answer `k` of `node`: pixels from the bottom (centered on the answer's line).
fn marker_y(w: &World, node: u32, k: usize) -> f32 {
    w.resource::<Talk>().unwrap().text_y + cur(w).views[node as usize].answers[k] * EM + 6.0
}

/// Shows script node `node`: a border, a box that fits the text, the text, and for a question the marker; all
/// `Hud`s that `view` places.
fn show(w: &mut World, node: u32) {
    close(w);
    let v = &cur(w).views[node as usize];
    let (mesh, height, asking) = (v.mesh, v.height, !v.answers.is_empty());
    let ui = *w.resource::<Ui>().unwrap();
    let h = height * EM + 2.0 * GAP;
    let h = if h > BOX_MIN { h } else { BOX_MIN }; // (no libm f32::max here)
    let (size, at) = (Vec2::new(BOX_W, h), Vec2::new(0.0, GAP + h / 2.0));
    let text = Vec2::new(GAP - BOX_W / 2.0, h - EM); // first baseline: one em below the top padding
    let z = |z| Transform::from_pos(0.0, 0.0, z);
    let mut ents = Vec::from([
        w.spawn_with((
            Hud { off: at, size },
            z(5.0),
            ui.quad,
            ui.flat,
            Color([0.90, 0.85, 0.60]),
        )),
        w.spawn_with((
            Hud {
                off: at,
                size: size - Vec2::splat(8.0),
            },
            z(5.1),
            ui.quad,
            ui.flat,
            Color([0.08, 0.08, 0.18]),
        )),
        w.spawn_with((
            Hud {
                off: text,
                size: Vec2::splat(EM),
            },
            z(5.2),
            mesh,
            ui.text,
            Color([0.95, 0.95, 0.90]),
        )),
    ]);
    let t = w.resource_mut::<Talk>().unwrap();
    t.text_y = text.y;
    t.at = Some((node, 0));
    if asking {
        let off = Vec2::new(text.x + 4.0, marker_y(w, node, 0));
        let m = w.spawn_with((
            Hud {
                off,
                size: Vec2::splat(8.0),
            },
            z(5.3),
            ui.quad,
            ui.flat,
            Color([0.95, 0.80, 0.25]),
        ));
        ents.push(m);
        w.resource_mut::<Talk>().unwrap().marker = Some(m);
    }
    w.resource_mut::<Talk>().unwrap().ents = ents;
}

/// `next` of a node or answer: another node, or the end.
fn go(w: &mut World, next: u32) {
    if next == END {
        close(w);
    } else {
        show(w, next);
    }
}

/// Z talks to the nearest script event in reach. While a line is up, Z moves on; at a question W / S (or the
/// arrows) pick an answer and Z takes it.
fn talk(w: &mut World, _: f32) {
    if w.resource::<Fade>().unwrap().phase != Phase::Idle {
        return;
    }
    let input = *w.resource::<Input>().unwrap();
    let Some((node, pick)) = w.resource::<Talk>().unwrap().at else {
        if input.just_pressed(KeyZ) {
            let hero = w.resource::<Hero>().unwrap().pos;
            let m = &cur(w).level;
            let near = m.events.iter().find_map(|e| {
                let at = Vec2::new(e.x as f32 + 0.5, m.h as f32 - e.y as f32 - 0.5);
                match e.action {
                    Action::Script { start } if (at - hero).length() < REACH => Some(start),
                    _ => None,
                }
            });
            if let Some(start) = near {
                show(w, start);
            }
        }
        return;
    };
    let (n, next) = match &cur(w).level.nodes[node as usize] {
        Node::Say { next, .. } => (0, *next),
        Node::Ask { options, .. } => (options.len(), options[pick].then),
    };
    let step = (input.just_pressed(KeyS) || input.just_pressed(ArrowDown)) as i32
        - (input.just_pressed(KeyW) || input.just_pressed(ArrowUp)) as i32;
    if n > 0 && step != 0 {
        let pick = (pick as i32 + step).rem_euclid(n as i32) as usize;
        w.resource_mut::<Talk>().unwrap().at = Some((node, pick));
        let (m, y) = (
            w.resource::<Talk>().unwrap().marker.unwrap(),
            marker_y(w, node, pick),
        );
        w.get_mut::<Hud>(m).unwrap().off.y = y;
    } else if input.just_pressed(KeyZ) {
        go(w, next);
    }
}

/// One axis at a time, so walking into a wall at an angle slides along it.
fn walk(w: &mut World, dt: f32) {
    if w.resource::<Talk>().unwrap().at.is_some()
        || w.resource::<Fade>().unwrap().phase != Phase::Idle
    {
        return; // reading, or between maps
    }
    let input = *w.resource::<Input>().unwrap();
    let dt = if dt > 0.05 { 0.05 } else { dt }; // a stall must not teleport the hero through a wall
    let (x, y) = (
        input.axis(&[KeyD, ArrowRight], &[KeyA, ArrowLeft]),
        input.axis(&[KeyW, ArrowUp], &[KeyS, ArrowDown]),
    );
    let diag = if x != 0.0 && y != 0.0 {
        FRAC_1_SQRT_2
    } else {
        1.0
    };
    let step = if input.is_holding(ShiftLeft) {
        RUN
    } else {
        WALK
    } * diag
        * dt;
    let mut p = w.resource::<Hero>().unwrap().pos;
    let m = &cur(w).level;
    let nx = Vec2::new(p.x + x * step, p.y);
    if !hits(m, nx) {
        p = nx;
    }
    let ny = Vec2::new(p.x, p.y + y * step);
    if !hits(m, ny) {
        p = ny;
    }
    w.resource_mut::<Hero>().unwrap().pos = p;
}

/// Stepping on a `goto` event starts the fade to its map. Arriving puts the hero next to the door, not on it, but
/// the door only works again after the hero has left its cells, so a door can never bounce the hero back.
fn exits(w: &mut World, _: f32) {
    if w.resource::<Talk>().unwrap().at.is_some() {
        return;
    }
    let hero = w.resource::<Hero>().unwrap().pos;
    let m = &cur(w).level;
    let cell = (hero.x as i32, m.h as i32 - 1 - hero.y as i32);
    let door = m.events.iter().find_map(|e| match &e.action {
        Action::Goto { map, x, y } if (e.x as i32, e.y as i32) == cell => {
            Some((map.clone(), *x, *y))
        }
        _ => None,
    });
    let f = w.resource_mut::<Fade>().unwrap();
    match door {
        None => f.armed = true,
        Some(to) if f.armed && f.phase == Phase::Idle => {
            f.armed = false;
            f.phase = Phase::Out;
            f.t = 0.0;
            f.to = Some(to);
        }
        Some(_) => {}
    }
}

/// Switches to level asset `name` and puts the hero on cell (`x`, `y`); the camera jumps instead of gliding.
fn enter(w: &mut World, name: &str, x: u32, y: u32) {
    let i = add_map(w, name);
    let (drawn, h) = {
        let m = &w.resource::<Maps>().unwrap().list[i];
        (m.drawn.clone(), m.level.h)
    };
    for e in core::mem::take(&mut w.resource_mut::<Maps>().unwrap().entities) {
        w.despawn(e);
    }
    let entities = tilemap::spawn(w, &drawn);
    let maps = w.resource_mut::<Maps>().unwrap();
    maps.cur = i;
    maps.entities = entities;
    w.resource_mut::<Hero>().unwrap().pos = Vec2::new(x as f32 + 0.5, h as f32 - y as f32 - 0.5);
    w.resource_mut::<Cam>().unwrap().ready = false;
}

/// Drives the fade: out to black, change the map while nothing shows, back in. Smoothstepped, and handed to the
/// post-process shader as `PostProcess.1`.
fn fade(w: &mut World, dt: f32) {
    let dt = if dt > 0.05 { 0.05 } else { dt };
    let f = w.resource_mut::<Fade>().unwrap();
    let mut arrive = None;
    match f.phase {
        Phase::Idle => {}
        Phase::Out => {
            f.t += dt / FADE_TIME;
            if f.t >= 1.0 {
                f.t = 1.0;
                f.phase = Phase::In;
                arrive = f.to.take();
            }
        }
        Phase::In => {
            f.t -= dt / FADE_TIME;
            if f.t <= 0.0 {
                f.t = 0.0;
                f.phase = Phase::Idle;
            }
        }
    }
    if let Some((name, x, y)) = arrive {
        enter(w, &name, x, y);
    }
    let t = w.resource::<Fade>().unwrap().t;
    w.resource_mut::<PostProcess>().unwrap().1 = t * t * (3.0 - 2.0 * t);
}

/// Nearest multiple of 1 / `ppu`, shifted by half a pixel when the window has an odd number of pixels (`win`) so
/// texel edges land between screen pixels, never on a pixel center. (No libm round in this engine.)
fn snap(v: f32, ppu: f32, win: f32) -> f32 {
    let h = if (win as u32) & 1 == 1 { 0.5 } else { 0.0 };
    let n = v * ppu - h + 0.5;
    ((n as i32 - (n < 0.0) as i32) as f32 + h) / ppu // floor(n)
}

/// Camera center for hero coordinate `t`: follows, but never shows past the map; a map smaller than the view is
/// centered.
fn fit(t: f32, half: f32, size: f32) -> f32 {
    if size <= 2.0 * half {
        size / 2.0
    } else if t < half {
        half
    } else if t > size - half {
        size - half
    } else {
        t
    }
}

/// Zoom, camera follow and where the hero and the dialogue are drawn. The view is sized in whole screen pixels per
/// sheet pixel (`Camera2d.height = window height / (48 * zoom)`) and everything is snapped to that pixel grid, so
/// the pixel art never shimmers or swims while the camera glides.
fn view(w: &mut World, dt: f32) {
    let input = *w.resource::<Input>().unwrap();
    let [ww, wh] = input.window_size();
    if wh < 1.0 {
        return; // minimized
    }
    let hero = w.resource::<Hero>().unwrap().pos;
    let m = &cur(w).level;
    let (mw, mh) = (m.w as f32, m.h as f32);
    let cam = w.resource_mut::<Cam>().unwrap();
    if input.just_pressed(KeyE) && cam.zoom < 3.0 {
        cam.zoom += 1.0;
    }
    if input.just_pressed(KeyQ) && cam.zoom > 1.0 {
        cam.zoom -= 1.0;
    }
    let ppu = TILE_PX * cam.zoom; // screen pixels per world unit
    let target = Vec2::new(
        fit(hero.x, ww / ppu / 2.0, mw),
        fit(hero.y, wh / ppu / 2.0, mh),
    );
    let k = FOLLOW * dt;
    cam.pos = if cam.ready {
        cam.pos + (target - cam.pos) * (k / (1.0 + k))
    } else {
        target
    };
    cam.ready = true;
    let pos = Vec2::new(snap(cam.pos.x, ppu, ww), snap(cam.pos.y, ppu, wh));

    for e in w.query_of::<(Camera2d,)>() {
        let c = w.get_mut::<Camera2d>(e).unwrap();
        c.pos = pos;
        c.height = wh / ppu;
    }
    for e in w.query_of::<(Hud, Transform)>() {
        let (off, size) = {
            let h = w.get::<Hud>(e).unwrap();
            (h.off, h.size)
        };
        let t = w.get_mut::<Transform>(e).unwrap();
        t.pos.x = pos.x + off.x / ppu;
        t.pos.y = pos.y - wh / ppu / 2.0 + off.y / ppu;
        t.scale = (size / ppu).extend(1.0);
    }
    let feet = Vec2::new(snap(hero.x, ppu, 0.0), snap(hero.y, ppu, 0.0));
    for e in w.query_of::<(Part, Transform)>() {
        let (off, z) = {
            let p = w.get::<Part>(e).unwrap();
            (p.off, p.z)
        };
        let t = w.get_mut::<Transform>(e).unwrap();
        t.pos.x = feet.x + off.x;
        t.pos.y = feet.y + off.y;
        t.pos.z = tilemap::depth(feet.y) + z; // behind what is below it, in front of what is above
    }
    tilemap::see_through(w, hero, BODY_W, SEE_THROUGH, dt);
}

/// Debug: 1-9 hide / show the current map's layers (`tilemap::LayerOf`: tile layers first, then sprite sheets) by
/// pushing them past the camera's far end. Entering a map shows everything again.
fn layers(w: &mut World, _: f32) {
    let input = *w.resource::<Input>().unwrap();
    let keys = [
        Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9,
    ];
    let Some(k) = keys.iter().position(|&d| input.just_pressed(d)) else {
        return;
    };
    for e in w.query_of::<(tilemap::LayerOf, Transform)>() {
        if w.get::<tilemap::LayerOf>(e).unwrap().0 == k as u32 {
            let z = &mut w.get_mut::<Transform>(e).unwrap().pos.z;
            *z = if *z < -500.0 {
                *z + 1000.0
            } else {
                *z - 1000.0
            }; // past `gfx::DEPTH2D`
        }
    }
}

fn game() {
    let mut app = App::new();
    app.add_system(Startup, setup)
        .add_system(Update, talk)
        .add_system(Update, walk)
        .add_system(Update, exits)
        .add_system(Update, fade)
        .add_system(Update, layers)
        .add_system(Update, view); // last: the camera sees this frame's position, even right after a map change
    app.world.spawn_with((Camera2d::default(),));
    app.run(c"Tilemap: WASD / arrows walk, Shift runs, Q/E zoom, Z talks, Esc quits");
}

platform::entry!(game);
