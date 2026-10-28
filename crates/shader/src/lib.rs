#![cfg_attr(not(feature = "build"), no_std)]

#[repr(C, align(4))]
pub struct Align4<T: ?Sized>(pub T);

/// `include_shader!("tri")` -> the SPIR-V that `shader::build("src/tri.wgsl")` wrote to OUT_DIR.
#[macro_export]
macro_rules! include_shader {
    ($name:literal) => {
        &$crate::Align4(*include_bytes!(concat!(
            env!("OUT_DIR"),
            "/",
            $name,
            ".spv"
        )))
    };
}

pub fn words(b: &'static Align4<[u8]>) -> &'static [u32] {
    unsafe { core::slice::from_raw_parts(b.0.as_ptr() as *const u32, b.0.len() / 4) }
}

/// For build.rs: compile every `*.wgsl` under `dir` (recursively) to `$OUT_DIR/<path relative to dir>.spv`,
/// running spirv-opt if present. `src/gfx/tri.wgsl` with `build("src")` -> `include_shader!("gfx/tri")`.
#[cfg(feature = "build")]
pub fn build(dir: &str) {
    // reruns on any change under dir, which also picks up newly added shaders
    println!("cargo:rerun-if-changed={dir}");
    walk(std::path::Path::new(dir), std::path::Path::new(dir));
}

#[cfg(feature = "build")]
fn walk(root: &std::path::Path, dir: &std::path::Path) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            walk(root, &p);
        } else if p.extension().is_some_and(|x| x == "wgsl") {
            compile(&p, &p.strip_prefix(root).unwrap().with_extension(""));
        }
    }
}

#[cfg(feature = "build")]
fn compile(path: &std::path::Path, name: &std::path::Path) {
    use std::{env, fs, path::PathBuf, process::Command};

    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let raw = out.join(name).with_extension("raw.spv");
    let dst = out.join(name).with_extension("spv");
    fs::create_dir_all(dst.parent().unwrap()).unwrap();
    let src = fs::read_to_string(path).unwrap();

    let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| {
        panic!(
            "{}: WGSL parse error:\n{}",
            path.display(),
            e.emit_to_string(&src)
        )
    });

    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::IMMEDIATES,
    )
    .validate(&module)
    .unwrap_or_else(|e| {
        panic!(
            "{}: WGSL validation error:\n{}",
            path.display(),
            e.emit_to_string(&src)
        )
    });

    let words =
        naga::back::spv::write_vec(&module, &info, &naga::back::spv::Options::default(), None)
            .expect("SPIR-V generation failed");

    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    fs::write(&raw, &bytes).unwrap();

    let have_opt = Command::new("spirv-opt").arg("--version").output().is_ok();
    let optimized = have_opt
        && Command::new("spirv-opt")
            .args(["-Os", "--strip-debug", "--target-env=vulkan1.3"])
            .arg(&raw)
            .arg("-o")
            .arg(&dst)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

    if !optimized {
        if have_opt {
            println!("cargo:warning=spirv-opt failed, using unoptimized SPIR-V");
        }
        fs::copy(&raw, &dst).unwrap();
    }
}
