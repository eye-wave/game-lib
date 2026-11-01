fn main() {
    font::build("assets/pixel.json");
    win32_sys::embed_manifest(None);
}
