/// `vec_impl!(Vec2, 2, x, y)`: a `repr(C)` struct of `f32`s plus the methods and operators every vec type shares.
macro_rules! vec_impl {
    ($T:ident, $N:literal, $($f:ident),+) => {
        #[derive(Clone, Copy, PartialEq, Default, Debug)]
        #[repr(C)]
        pub struct $T {
            $(pub $f: f32),+
        }

        impl $T {
            pub const ZERO: Self = Self::splat(0.0);
            pub const ONE: Self = Self::splat(1.0);

            #[inline(always)]
            #[must_use]
            pub const fn new($($f: f32),+) -> Self {
                Self { $($f),+ }
            }

            #[inline]
            #[must_use]
            pub const fn splat(v: f32) -> Self {
                Self { $($f: v),+ }
            }

            #[inline]
            #[must_use]
            pub const fn to_array(self) -> [f32; $N] {
                [$(self.$f),+]
            }

            /// Zero-copy view as an array: `repr(C)`, all `f32`, so the layouts are identical (no padding).
            #[inline]
            #[must_use]
            pub fn as_array(&self) -> &[f32; $N] {
                // SAFETY: same size, alignment and field order as `[f32; N]`.
                unsafe { &*core::ptr::from_ref(self).cast() }
            }

            #[inline]
            #[must_use]
            pub fn dot(self, o: Self) -> f32 {
                0.0 $(+ self.$f * o.$f)+
            }

            #[inline]
            #[must_use]
            pub fn length_squared(self) -> f32 {
                self.dot(self)
            }

            #[inline]
            #[must_use]
            pub fn length(self) -> f32 {
                crate::FloatExt::sqrt(self.length_squared())
            }

            #[inline]
            #[must_use]
            pub fn distance(self, o: Self) -> f32 {
                (self - o).length()
            }

            /// Unit length, or `ZERO` for a zero-length vector (so no NaNs).
            #[inline]
            #[must_use]
            pub fn normalize_or_zero(self) -> Self {
                let l = self.length();
                if l > 0.0 { self / l } else { Self::ZERO }
            }

            /// `t` = 0 gives `self`, 1 gives `o`.
            #[inline]
            #[must_use]
            pub fn lerp(self, o: Self, t: f32) -> Self {
                self + (o - self) * t
            }

            #[inline]
            #[must_use]
            pub fn min(self, o: Self) -> Self {
                Self { $($f: self.$f.min(o.$f)),+ }
            }

            #[inline]
            #[must_use]
            pub fn max(self, o: Self) -> Self {
                Self { $($f: self.$f.max(o.$f)),+ }
            }

            #[inline]
            #[must_use]
            pub fn clamp(self, lo: Self, hi: Self) -> Self {
                self.max(lo).min(hi)
            }

            #[inline]
            #[must_use]
            pub fn abs(self) -> Self {
                Self { $($f: crate::FloatExt::abs(self.$f)),+ }
            }
        }

        impl From<[f32; $N]> for $T {
            #[inline]
            fn from([$($f),+]: [f32; $N]) -> Self {
                Self { $($f),+ }
            }
        }

        impl From<$T> for [f32; $N] {
            #[inline]
            fn from(v: $T) -> Self {
                v.to_array()
            }
        }

        impl Add for $T {
            type Output = Self;
            #[inline]
            fn add(self, o: Self) -> Self {
                Self { $($f: self.$f + o.$f),+ }
            }
        }

        impl Sub for $T {
            type Output = Self;
            #[inline]
            fn sub(self, o: Self) -> Self {
                Self { $($f: self.$f - o.$f),+ }
            }
        }

        /// Component-wise.
        impl Mul for $T {
            type Output = Self;
            #[inline]
            fn mul(self, o: Self) -> Self {
                Self { $($f: self.$f * o.$f),+ }
            }
        }

        impl Mul<f32> for $T {
            type Output = Self;
            #[inline]
            fn mul(self, s: f32) -> Self {
                Self { $($f: self.$f * s),+ }
            }
        }

        impl Mul<$T> for f32 {
            type Output = $T;
            #[inline]
            fn mul(self, v: $T) -> $T {
                v * self
            }
        }

        impl Div<f32> for $T {
            type Output = Self;
            #[inline]
            fn div(self, s: f32) -> Self {
                Self { $($f: self.$f / s),+ }
            }
        }

        impl Neg for $T {
            type Output = Self;
            #[inline]
            fn neg(self) -> Self {
                Self { $($f: -self.$f),+ }
            }
        }

        impl AddAssign for $T {
            #[inline]
            fn add_assign(&mut self, o: Self) {
                $(self.$f += o.$f;)+
            }
        }

        impl SubAssign for $T {
            #[inline]
            fn sub_assign(&mut self, o: Self) {
                $(self.$f -= o.$f;)+
            }
        }

        impl MulAssign<f32> for $T {
            #[inline]
            fn mul_assign(&mut self, s: f32) {
                $(self.$f *= s;)+
            }
        }

        impl DivAssign<f32> for $T {
            #[inline]
            fn div_assign(&mut self, s: f32) {
                $(self.$f /= s;)+
            }
        }
    };
}
