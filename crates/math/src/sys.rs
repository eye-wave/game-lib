macro_rules! float_ext {
    (
        unary   { $($u:ident = $uc:ident;)* }
        binary  { $($b:ident = $bc:ident;)* }
        ternary { $($t:ident = $tc:ident;)* }
    ) => {
        ::paste::paste! {
            mod libm_sys {
                #[cfg_attr(unix, link(name = "m"))]
                #[cfg_attr(windows, link(name = "ucrt"))]
                unsafe extern "C" {
                    $(
                        pub safe fn [<$uc f>](x: f32) -> f32;
                        pub safe fn $uc(x: f64) -> f64;
                    )*
                    $(
                        pub safe fn [<$bc f>](x: f32, y: f32) -> f32;
                        pub safe fn $bc(x: f64, y: f64) -> f64;
                    )*
                    $(
                        pub safe fn [<$tc f>](x: f32, y: f32, z: f32) -> f32;
                        pub safe fn $tc(x: f64, y: f64, z: f64) -> f64;
                    )*
                }

                // Re-export under the plain C name, one module per float type,
                // so the impls below can just say `libm_sys::<mod>::<name>`.
                pub mod single {
                    $(pub use super::[<$uc f>] as $uc;)*
                    $(pub use super::[<$bc f>] as $bc;)*
                    $(pub use super::[<$tc f>] as $tc;)*
                }
                pub mod double {
                    $(pub use super::$uc;)*
                    $(pub use super::$bc;)*
                    $(pub use super::$tc;)*
                }
            }
        }

        pub trait FloatExt: Sized {
            $(fn $u(self) -> Self;)*
            $(fn $b(self, y: Self) -> Self;)*
            $(fn $t(self, y: Self, z: Self) -> Self;)*
        }

        float_ext!(@impl f32, single;
            unary { $($u = $uc;)* } binary { $($b = $bc;)* } ternary { $($t = $tc;)* });
        float_ext!(@impl f64, double;
            unary { $($u = $uc;)* } binary { $($b = $bc;)* } ternary { $($t = $tc;)* });
    };

    (@impl $ty:ty, $m:ident;
        unary   { $($u:ident = $uc:ident;)* }
        binary  { $($b:ident = $bc:ident;)* }
        ternary { $($t:ident = $tc:ident;)* }
    ) => {
        impl FloatExt for $ty {
            $(#[inline] fn $u(self) -> Self { libm_sys::$m::$uc(self) })*
            $(#[inline] fn $b(self, y: Self) -> Self { libm_sys::$m::$bc(self, y) })*
            $(#[inline] fn $t(self, y: Self, z: Self) -> Self { libm_sys::$m::$tc(self, y, z) })*
        }
    };
}

float_ext! {
    unary {
        sqrt = sqrt;
        cbrt = cbrt;
        abs = fabs;
        floor = floor;
        ceil = ceil;
        round = round;
        trunc = trunc;
        rint = rint;
        exp = exp;
        exp2 = exp2;
        exp_m1 = expm1;
        ln = log;
        log2 = log2;
        log10 = log10;
        ln_1p = log1p;
        sin = sin;
        cos = cos;
        tan = tan;
        asin = asin;
        acos = acos;
        atan = atan;
        sinh = sinh;
        cosh = cosh;
        tanh = tanh;
        asinh = asinh;
        acosh = acosh;
        atanh = atanh;
        erf = erf;
        erfc = erfc;
        tgamma = tgamma;
        lgamma = lgamma;
    }
    binary {
        powf = pow;
        atan2 = atan2;
        hypot = hypot;
        fmod = fmod;
        remainder = remainder;
        copysign = copysign;
        fmin = fmin;  fmax = fmax;
        fdim = fdim;
        nextafter = nextafter;
    }
    ternary {
        mul_add = fma;
    }
}
