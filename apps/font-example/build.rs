fn main() {
    let chars: String = (' '..='~').collect();
    font::build("assets/ComicMono.ttf", &chars, 32);
    win32_sys::embed_manifest(None);
}
