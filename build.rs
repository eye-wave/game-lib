use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/tri.wgsl");

    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let src = fs::read_to_string("src/tri.wgsl").unwrap();

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
    let raw = out.join("tri.raw.spv");
    let dst = out.join("tri.spv");
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
