/// `rust_name = libm_name;` generates the extern decl, the `FloatExt` method, and its f32 impl.
macro_rules! float_ext {
    ($($name:ident = $c:ident;)*) => {
        #[cfg_attr(unix, link(name = "m"))]
        #[cfg_attr(windows, link(name = "ucrt"))]
        unsafe extern "C" {
            $(fn $c(x: f32) -> f32;)*
        }

        pub trait FloatExt {
            $(fn $name(self) -> Self;)*
        }

        impl FloatExt for f32 {
            $(
                #[inline]
                fn $name(self) -> Self {
                    // SAFETY: pure libm function, every f32 is a valid input.
                    unsafe { $c(self) }
                }
            )*
        }
    };
}

float_ext! {
    sqrt = sqrtf;
    sin = sinf;
    cos = cosf;
    abs = fabsf;
}
