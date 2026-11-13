//! Feature `cam3d`: `Camera3d` and its perspective view-projection.
use math::{FloatExt, Vec3};

/// Component: a 3D viewpoint. Right-handed, y up; at yaw 0 and pitch 0 it looks down -z, and a positive yaw turns
/// left. `focal` is 1 / tan(vertical fov / 2). `sky` is the clear color.
///
/// With `multicam`, every active camera (3D ones first, then 2D, at most `MAX_CAMERAS`) renders the whole scene into its own
/// target and is copied into `viewport` = [x, y, width, height], fractions of the window with y counted up from the
/// bottom. Later cameras draw over earlier ones. The target is `scale` times the viewport's pixel size: below 1 it
/// renders at a lower resolution and is upscaled with nearest filtering (big pixels), above 1 it is supersampled.
/// Without `multicam` only the first active camera is used, fullscreen, and `viewport` and `scale` are ignored.
/// With no active camera a default one is used.
#[derive(Clone, Copy)]
pub struct Camera3d {
    pub pos: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub focal: f32,
    pub near: f32,
    pub far: f32,
    pub sky: [f32; 3],
    pub viewport: [f32; 4],
    pub scale: f32,
    pub active: bool,
}
impl Default for Camera3d {
    fn default() -> Self {
        Self {
            pos: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            focal: 1.428, // 70 degrees
            near: 0.1,
            far: 200.0,
            sky: [0.53, 0.68, 0.85],
            viewport: [0.0, 0.0, 1.0, 1.0],
            scale: 1.0,
            active: true,
        }
    }
}

/// View * projection, column-major, depth range 0..1 (see `Camera3d` for the conventions).
pub(crate) fn view_proj(c: &Camera3d, aspect: f32) -> [f32; 16] {
    let (sy, cy, sp, cp) = (c.yaw.sin(), (c.yaw).cos(), (c.pitch).sin(), (c.pitch).cos());
    let f = [-sy * cp, sp, -cy * cp]; // forward
    let r = [cy, 0.0, -sy]; // right
    let u = [sp * sy, cp, sp * cy]; // up
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let (fx, fy) = (c.focal / aspect, c.focal);
    let a = c.far / (c.near - c.far);
    let b = c.near * c.far / (c.near - c.far);
    let p = c.pos.to_array();
    let fp = dot(f, p);
    let rows = [
        [r[0] * fx, r[1] * fx, r[2] * fx, -dot(r, p) * fx],
        [u[0] * fy, u[1] * fy, u[2] * fy, -dot(u, p) * fy],
        [-f[0] * a, -f[1] * a, -f[2] * a, fp * a + b],
        [f[0], f[1], f[2], -fp],
    ];
    core::array::from_fn(|i| rows[i % 4][i / 4]) // column-major
}
