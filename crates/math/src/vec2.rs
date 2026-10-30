use core::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign};

use crate::{Vec3, sys::FloatExt};

vec_impl!(Vec2, 2, x, y);

impl Vec2 {
    /// Unit vector at `angle` radians counterclockwise from +x.
    #[inline]
    #[must_use]
    pub fn from_angle(angle: f32) -> Self {
        Self::new(angle.cos(), angle.sin())
    }

    /// Rotated 90 degrees counterclockwise.
    #[inline]
    #[must_use]
    pub const fn perp(self) -> Self {
        Self::new(-self.y, self.x)
    }

    /// Rotated `angle` radians counterclockwise.
    #[inline]
    #[must_use]
    pub fn rotate(self, angle: f32) -> Self {
        let (s, c) = (angle.sin(), angle.cos());
        Self::new(self.x * c - self.y * s, self.x * s + self.y * c)
    }

    #[inline]
    #[must_use]
    pub const fn extend(self, z: f32) -> Vec3 {
        Vec3::new(self.x, self.y, z)
    }
}
