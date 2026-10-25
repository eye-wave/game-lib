fn main() {
    shader::build("src");
    #[cfg(target_os = "windows")]
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|o| o == "windows") {
        let name = std::env::var("CARGO_PKG_NAME").unwrap();
        winresource::WindowsResource::new()
            .set("CompanyName", "YOUR NAME")
            .set("FileDescription", &name)
            .set("ProductName", &name)
            .set("LegalCopyright", "Copyright (c) YOUR NAME")
            .set("OriginalFilename", &(name + ".exe"))
            .set_manifest(include_str!("app.manifest"))
            .compile()
            .unwrap();
    }
}
