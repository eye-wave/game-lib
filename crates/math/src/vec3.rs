use core::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign};

use crate::Vec2;

vec_impl!(Vec3, 3, x, y, z);

impl Vec3 {
    #[inline]
    #[must_use]
    pub fn cross(self, o: Self) -> Self {
        Self::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    /// Drops `z`.
    #[inline]
    #[must_use]
    pub const fn truncate(self) -> Vec2 {
        Vec2::new(self.x, self.y)
    }
}
