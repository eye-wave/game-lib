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

/// For build.rs: compile `path` (WGSL) to `$OUT_DIR/<stem>.spv`, running spirv-opt if present.
#[cfg(feature = "build")]
pub fn build(path: &str) {
    use std::{
        env, fs,
        path::{Path, PathBuf},
        process::Command,
    };

    println!("cargo:rerun-if-changed={path}");
    let stem = Path::new(path).file_stem().unwrap().to_str().unwrap();
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let src = fs::read_to_string(path).unwrap();

    let module = naga::front::wgsl::parse_str(&src)
        .unwrap_or_else(|e| panic!("WGSL parse error:\n{}", e.emit_to_string(&src)));

    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("WGSL validation error:\n{}", e.emit_to_string(&src)));

    let words =
        naga::back::spv::write_vec(&module, &info, &naga::back::spv::Options::default(), None)
            .expect("SPIR-V generation failed");

    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    let raw = out.join(format!("{stem}.raw.spv"));
    let dst = out.join(format!("{stem}.spv"));
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
