fn main() {
    // the baked levels are assets: they go next to the images, where `tilemap::load` looks
    tilemap::build("assets/town.json", "../../assets/town.level");
    tilemap::build("assets/garden.json", "../../assets/garden.level");
    font::build("assets/pixel.json");
    shader::build("src"); // fade.wgsl
    win32_sys::embed_manifest(None);
}
