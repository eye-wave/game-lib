//! Which cameras draw this frame, and the `View` (view-projection, clear color, window rectangle) of each.
use alloc::vec::Vec;
use ecs::App;

#[cfg(feature = "cam2d")]
use crate::cam2d::{Camera2d, view_proj_2d};
#[cfg(feature = "cam3d")]
use crate::cam3d::{Camera3d, view_proj};

/// Cameras drawn per frame; extra active cameras are ignored. Without `multicam` it is 1.
pub const MAX_CAMERAS: usize = if cfg!(feature = "multicam") { 4 } else { 1 };

/// One camera, ready to draw.
#[cfg_attr(not(feature = "multicam"), allow(dead_code))]
pub(crate) struct View {
    pub vp: [f32; 16],
    pub sky: [f32; 3],
    pub rect: [f32; 4], // viewport, fractions of the window, y up from the bottom
    pub scale: f32,
}

/// A camera's viewport: honored with `multicam`, else always the whole window.
pub(crate) fn rect(r: [f32; 4]) -> [f32; 4] {
    if cfg!(feature = "multicam") {
        r
    } else {
        [0.0, 0.0, 1.0, 1.0]
    }
}

fn scale(s: f32) -> f32 {
    if cfg!(feature = "multicam") { s } else { 1.0 }
}

/// Spawns a default camera if the app has none (a 3D one when `cam3d` is on, else a 2D one).
pub(crate) fn ensure_camera(app: &mut App) {
    let mut has_camera = false;
    #[cfg(feature = "cam2d")]
    {
        has_camera |= !app.world.query_of::<(Camera2d,)>().is_empty();
    }
    #[cfg(feature = "cam3d")]
    {
        has_camera |= !app.world.query_of::<(Camera3d,)>().is_empty();
    }
    if !has_camera {
        #[cfg(feature = "cam3d")]
        app.world.spawn_with((Camera3d::default(),));
        #[cfg(not(feature = "cam3d"))]
        app.world.spawn_with((Camera2d::default(),));
    }
}

fn add(v: &mut Vec<View>, vp: [f32; 16], sky: [f32; 3], rect: [f32; 4], scale: f32) {
    if rect[2] > 0.0 && rect[3] > 0.0 {
        v.push(View {
            vp,
            sky,
            rect,
            scale,
        });
    }
}

/// 2D cameras first, then 3D; with none active a default camera is used.
pub(crate) fn collect(app: &App, ew: f32, eh: f32) -> Vec<View> {
    let world = &app.world;
    let aspect = |r: [f32; 4]| (r[2] * ew) / (r[3] * eh);
    let mut views: Vec<View> = Vec::new();
    #[cfg(feature = "cam2d")]
    for e in world.query_of::<(Camera2d,)>() {
        let c = world.get::<Camera2d>(e).unwrap();
        if c.active {
            let r = rect(c.viewport);
            let vp = view_proj_2d(c, aspect(r));
            add(&mut views, vp, c.sky, r, scale(c.scale));
        }
    }
    #[cfg(feature = "cam3d")]
    for e in world.query_of::<(Camera3d,)>() {
        let c = world.get::<Camera3d>(e).unwrap();
        if c.active {
            let r = rect(c.viewport);
            let vp = view_proj(c, aspect(r));
            add(&mut views, vp, c.sky, r, scale(c.scale));
        }
    }
    if views.is_empty() {
        #[cfg(feature = "cam3d")]
        let (vp, c) = {
            let c = Camera3d::default();
            (
                view_proj(&c, aspect(rect(c.viewport))),
                (c.sky, c.viewport, c.scale),
            )
        };
        #[cfg(not(feature = "cam3d"))]
        let (vp, c) = {
            let c = Camera2d::default();
            (
                view_proj_2d(&c, aspect(rect(c.viewport))),
                (c.sky, c.viewport, c.scale),
            )
        };
        add(&mut views, vp, c.0, rect(c.1), scale(c.2));
    }
    views.truncate(MAX_CAMERAS);
    views
}
