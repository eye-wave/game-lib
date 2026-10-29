fn main() {
    shader::build("src");
    win32_sys::embed_manifest(None);
}
