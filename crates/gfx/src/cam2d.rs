//! Feature `cam2d`: `Camera2d`, its orthographic view-projection, `Material::flat`, and (with `mouse`) `cursor_world`.
use crate::assets::Material;
use ash::vk;
use math::{FloatExt, Vec2, Vec3};
use shader::Align4;

static FLAT: &Align4<[u8]> = shader::include_shader!("flat");

/// Component: an orthographic 2D viewpoint looking at the xy plane (x right, y up). `viewport`, `scale`, `active`
/// and drawing order work as for `Camera3d` (`viewport` and `scale` need `multicam`). `height` is how many world
/// units fit top to bottom (the width follows the window's aspect ratio).
/// `rotation` turns the view counterclockwise, in radians. A `Transform`'s z is its layer: higher z draws in front,
/// within +-`DEPTH2D`. Spawn one before `app.run(..)` to skip the default `Camera3d`.
#[derive(Clone, Copy)]
pub struct Camera2d {
    pub pos: Vec2,
    pub height: f32,
    pub rotation: f32,
    pub sky: [f32; 3],
    pub viewport: [f32; 4],
    pub scale: f32,
    pub active: bool,
}
impl Default for Camera2d {
    fn default() -> Self {
        Self {
            pos: Vec2::ZERO,
            height: 16.0,
            rotation: 0.0,
            sky: [0.53, 0.68, 0.85],
            viewport: [0.0, 0.0, 1.0, 1.0],
            scale: 1.0,
            active: true,
        }
    }
}
/// Layers of a `Camera2d` scene span z in [-DEPTH2D, DEPTH2D].
pub const DEPTH2D: f32 = 100.0;

/// Orthographic view * projection for `Camera2d`, same conventions as `view_proj`: depth 0..1, higher z nearer.
pub(crate) fn view_proj_2d(c: &Camera2d, aspect: f32) -> [f32; 16] {
    let (s, co) = (c.rotation.sin(), c.rotation.cos());
    let (sy, sx) = (2.0 / c.height, 2.0 / (c.height * aspect));
    let (px, py) = (c.pos.x, c.pos.y);
    let rows = [
        [co * sx, s * sx, 0.0, -(px * co + py * s) * sx],
        [-s * sy, co * sy, 0.0, (px * s - py * co) * sy],
        [0.0, 0.0, -0.5 / DEPTH2D, 0.5],
        [0.0, 0.0, 0.0, 1.0],
    ];
    core::array::from_fn(|i| rows[i % 4][i / 4]) // column-major
}

impl Material {
    /// Built in, for 2D shapes: the mesh is plain `Vec3` positions, filled with the entity's `Color` (white if it has
    /// none). `Transform.yaw` rotates about z instead of y and `scale.z` is ignored. Pairs with `Camera2d`.
    pub fn flat() -> Self {
        Self::new(
            FLAT,
            size_of::<Vec3>() as u32,
            &[vk::Format::R32G32B32_SFLOAT],
        )
    }
}

#[cfg(feature = "mouse")]
impl crate::input::Input {
    /// The pointer in `cam`'s world (x right, y up), or `None` if it is outside the window or the camera's viewport.
    pub fn cursor_world(&self, cam: &Camera2d) -> Option<Vec2> {
        let [cx, cy] = self.mouse.cursor?;
        let [ew, eh] = self.mouse.size;
        let [vx, vy, vw, vh] = crate::cams::rect(cam.viewport); // fullscreen without `multicam`
        let u = (cx / ew - vx) / vw; // 0..1 across the viewport, left to right
        let v = ((1.0 - cy / eh) - vy) / vh; // 0..1 up the viewport, bottom to top
        if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
            return None;
        }
        let aspect = vw * ew / (vh * eh);
        let (x, y) = (
            (2.0 * u - 1.0) * cam.height * aspect / 2.0,
            (2.0 * v - 1.0) * cam.height / 2.0,
        );
        let (s, c) = (cam.rotation.sin(), cam.rotation.cos()); // view -> world is the inverse of view_proj_2d
        Some(Vec2::new(
            cam.pos.x + c * x - s * y,
            cam.pos.y + s * x + c * y,
        ))
    }
}
