// Feature `build`: compile msdfgen's core (the `msdfgen` git submodule, no freetype/skia/png) and `msdf/shim.cpp`.
#[cfg(feature = "build")]
fn main() {
    let core: Vec<_> = std::fs::read_dir("msdfgen/core")
        .expect("msdfgen submodule missing: git submodule update --init")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "cpp"))
        .collect();
    cc::Build::new()
        .cpp(true)
        .std("c++11") // ignored by MSVC (always at least C++14 there)
        .include("msdfgen")
        .define("MSDFGEN_PUBLIC", "") // static build: skips the generated msdfgen-config.h
        .define("MSDFGEN_USE_CPP11", None)
        .define("NDEBUG", None)
        .warnings(false)
        .files(&core)
        .file("msdf/shim.cpp")
        .compile("msdfgen");
    println!("cargo:rerun-if-changed=msdf");
    println!("cargo:rerun-if-changed=msdfgen/core");
}

#[cfg(not(feature = "build"))]
fn main() {}
