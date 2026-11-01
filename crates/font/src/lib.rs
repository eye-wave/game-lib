//! MSDF text. The runtime half is `no_std`: `Font::parse` reads an atlas baked at compile time and `Font::layout`
//! turns a string into quads for `gfx` (feature `text`). No texture, outline or font file is touched at runtime.
//!
//! Bake in the game's `build.rs` (needs `font = { features = ["build"] }` under `[build-dependencies]`):
//! ```ignore
//! font::build("assets/inter.woff", " !?.,0123456789ABC...abc...", 32); // .ttf/.otf/.woff, glyph cell px
//! ```
//! Use it (the atlas goes to `gfx` as the `Atlas` resource, text meshes use `Material::msdf()`):
//! ```ignore
//! let font = font::include_font!("inter");
//! w.insert_resource(gfx::Atlas { w: font.w, h: font.h, rgb: font.rgb });
//! let mesh = Mesh::new(&font.layout("Hello\nworld", 1.0)); // 1.0 = one em in world units
//! ```
//!
//! Pixel fonts work too: with feature `bitmap` (instead of `build`, no msdfgen) `font::build` takes a BMFont-style
//! JSON (`info.size`, `common.{lineHeight,base,scaleW,scaleH}`, `chars[]` with `id x y width height xoffset yoffset
//! xadvance`; one page, kerning ignored) and writes the same blob, minus pixels. The .png stays a normal asset:
//! ```ignore
//! font::build("assets/pixel.json"); // build.rs
//! let f = font::include_font!("pixel");
//! let img = media::load_image("pixel.png").unwrap_or_else(|e| die(e));
//! w.insert_resource(gfx::Atlas { w: img.w, h: img.h, rgb: font::alpha_atlas(&img.rgba).leak() });
//! ```
#![cfg_attr(not(any(feature = "build", feature = "bitmap", test)), no_std)]

extern crate alloc;

use alloc::vec::Vec;

#[cfg(all(feature = "build", feature = "bitmap"))]
compile_error!("features `build` (MSDF) and `bitmap` are alternatives: enable one");
#[cfg(feature = "build")]
mod bake;
#[cfg(feature = "build")]
pub use bake::build;
#[cfg(feature = "bitmap")]
mod bitmap;
#[cfg(feature = "bitmap")]
pub use bitmap::build;

/// `include_font!("inter")` -> the `Font` that `font::build("..../inter.woff", ..)` (or `inter.json`) wrote to `OUT_DIR`.
#[macro_export]
macro_rules! include_font {
    ($name:literal) => {
        $crate::Font::parse(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/",
            $name,
            ".font"
        )))
    };
}

// Blob, all little-endian words:
//   header: w, h (atlas px), glyph count, line height (f32, em)
//   per glyph (10 words): codepoint, advance, quad x0 y0 x1 y1 (em, y up, from the pen on the baseline),
//                         uv u0 v0 (top left) u1 v1 (bottom right); an empty quad (a space) is x0 == x1
//   then w * h * 3 bytes of RGB, top row first (MSDF fonts only: a bitmap font has none, its image is loaded apart)
const HEAD: usize = 4;
const REC: usize = 10;

fn word(b: &[u8], i: usize) -> [u8; 4] {
    b[i * 4..i * 4 + 4].try_into().unwrap()
}
fn u(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(word(b, i))
}
fn f(b: &[u8], i: usize) -> f32 {
    f32::from_le_bytes(word(b, i))
}

/// RGB8 for `gfx::Atlas` from a bitmap font's RGBA image: every channel is the pixel's alpha, so `Material::msdf()`'s
/// 0.5 cut-off draws the glyph pixels that are at least half opaque. (Glyphs drawn in a color on an opaque
/// background: use the luminance instead.)
pub fn alpha_atlas(rgba: &[u8]) -> Vec<u8> {
    rgba.as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[3]; 3])
        .collect()
}

/// A baked font, borrowed from the blob (no copy).
pub struct Font<'a> {
    pub w: u32,
    pub h: u32,
    /// Distance between baselines, in ems.
    pub line: f32,
    /// The atlas pixels, RGB8, top row first: `gfx::Atlas::rgb`. Empty for a bitmap font.
    pub rgb: &'a [u8],
    glyphs: &'a [u8],
}

impl<'a> Font<'a> {
    /// `b` comes from `font::build` (via `include_font!`), so it is trusted: a truncated blob panics.
    pub fn parse(b: &'a [u8]) -> Self {
        let (w, h, n) = (u(b, 0), u(b, 1), u(b, 2) as usize);
        let end = (HEAD + n * REC) * 4;
        Self {
            w,
            h,
            line: f(b, 3),
            glyphs: &b[HEAD * 4..end],
            rgb: &b[end..],
        }
    }

    // linear scan, a binary search over the sorted records if a font ever has thousands of glyphs
    fn glyph(&self, c: char) -> Option<&[u8]> {
        self.glyphs
            .as_chunks::<{ REC * 4 }>()
            .0
            .iter()
            .find(|g| u(*g, 0) == c as u32)
            .map(|g| &g[..])
    }

    /// Quads for `text`, starting with the pen at the origin on the baseline; `size` is one em in world units and
    /// `\n` goes down one `line`. Vertices are `[x, y, z, u, v]`, six per glyph: hand them to `Mesh::new`.
    /// Characters the font was not baked with are skipped. no kerning, add it from the GPOS/kern table
    /// in `build` if pairs like "AV" look loose.
    pub fn layout(&self, text: &str, size: f32) -> Vec<[f32; 5]> {
        let (mut out, mut pen) = (Vec::new(), [0.0, 0.0]);
        for c in text.chars() {
            if c == '\n' {
                pen = [0.0, pen[1] - self.line * size];
            } else {
                self.step(&mut out, &mut pen, c, size);
            }
        }
        out
    }

    /// Word wrap, one word at a time: appends `word` and a space to `out`, first moving `pen` (`[x, y]`, start it at
    /// `[0.0, 0.0]`) to the start of the next line if the word would end past `wrap` (world units from x = 0).
    /// Several `out`s can share one `pen`: lay each word out into the buffer of its color and they still line up.
    /// A word wider than `wrap` is not split.
    pub fn push_word(
        &self,
        out: &mut Vec<[f32; 5]>,
        pen: &mut [f32; 2],
        word: &str,
        size: f32,
        wrap: f32,
    ) {
        let width: f32 = word
            .chars()
            .filter_map(|c| self.glyph(c))
            .map(|g| f(g, 1) * size)
            .sum();
        if pen[0] > 0.0 && pen[0] + width > wrap {
            *pen = [0.0, pen[1] - self.line * size];
        }
        for c in word.chars().chain([' ']) {
            self.step(out, pen, c, size);
        }
    }

    /// One character at the pen: its quad (spaces have none), then the pen moves on.
    fn step(&self, out: &mut Vec<[f32; 5]>, pen: &mut [f32; 2], c: char, size: f32) {
        let Some(g) = self.glyph(c) else { return };
        if f(g, 4) > f(g, 2) {
            let (x0, y0) = (pen[0] + f(g, 2) * size, pen[1] + f(g, 3) * size);
            let (x1, y1) = (pen[0] + f(g, 4) * size, pen[1] + f(g, 5) * size);
            let (u0, v0, u1, v1) = (f(g, 6), f(g, 7), f(g, 8), f(g, 9));
            // y up, so the quad's top (y1) samples the cell's top (v0)
            for [x, y, u, v] in [
                [x0, y0, u0, v1],
                [x1, y0, u1, v1],
                [x1, y1, u1, v0],
                [x0, y0, u0, v1],
                [x1, y1, u1, v0],
                [x0, y1, u0, v0],
            ] {
                out.push([x, y, 0.0, u, v]);
            }
        }
        pen[0] += f(g, 1) * size;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_advances_and_wraps() {
        // two glyphs: ' ' (advance 0.5, no quad) and 'A' (advance 1, quad 0..1 x 0..1, uv 0..0.5), a 2x1 atlas
        let mut b: Vec<u8> = Vec::new();
        let mut w = |v: u32| b.extend(v.to_le_bytes());
        let _ = [2, 1, 2, 1.5f32.to_bits()].map(&mut w);
        let one = 1.0f32.to_bits();
        let _ = [' ' as u32, 0.5f32.to_bits(), 0, 0, 0, 0, 0, 0, 0, 0].map(&mut w);
        let _ = ['A' as u32, one, 0, 0, one, one, 0, 0, 0.5f32.to_bits(), one].map(&mut w);
        b.extend([0u8; 6]);
        let font = Font::parse(&b);
        let v = font.layout(" A\nA", 2.0);
        assert_eq!(v.len(), 12); // the space only moves the pen
        assert_eq!(v[0], [1.0, 0.0, 0.0, 0.0, 1.0]); // pen at 0.5 em * 2, bottom left, bottom of cell
        assert_eq!(v[2], [3.0, 2.0, 0.0, 0.5, 0.0]); // top right
        assert_eq!(v[6][0..2], [0.0, -3.0]); // second line: one 1.5 em line down

        let (mut out, mut pen) = (Vec::new(), [0.0, 0.0]);
        font.push_word(&mut out, &mut pen, "A", 2.0, 3.0); // 2 wide + a space of 1: the pen is at the limit
        assert_eq!(pen, [3.0, 0.0]);
        font.push_word(&mut out, &mut pen, "A", 2.0, 3.0); // would end at 5 > 3: wraps first
        assert_eq!(pen, [3.0, -3.0]);
        assert_eq!(out[6][0..2], [0.0, -3.0]);
    }
}
