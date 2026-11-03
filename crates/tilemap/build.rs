// compiles src/*.wgsl for the runtime half; the `build` half has no shaders
#[cfg(feature = "runtime")]
fn main() {
    shader::build("src");
}

#[cfg(not(feature = "runtime"))]
fn main() {}
