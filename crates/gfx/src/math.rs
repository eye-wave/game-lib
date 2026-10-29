//! no_std has no libm: the little trigonometry games need.
use core::f32::consts::{FRAC_PI_2, PI, TAU};

/// sin via range reduction to [-pi/2, pi/2] and a 9th-order Taylor series (error < 4e-6).
pub fn sin(x: f32) -> f32 {
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

pub fn cos(x: f32) -> f32 {
    sin(x + FRAC_PI_2)
}

/// |x| (f32::abs is not in core).
pub fn abs(x: f32) -> f32 {
    if x < 0.0 { -x } else { x }
}
