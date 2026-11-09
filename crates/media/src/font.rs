//! Text from a glyph grid image: `Font::new` reads nothing but the image's size. Glyph `i` of `chars` sits in cell
//! `i`, `COLS` cells per row, left to right, top to bottom. The grid is a pixel font's PNG (`inset` = the padding
//! around each glyph in its cell) or one the webview drew from a TTF/OTF/WOFF (`Loader::font`, inset 0).
//! Layout is plain Rust, nothing goes through the webview. The glyph box is the cell minus the inset, monospaced;
//! one em is its height. Feed `layout`'s vertices to `Mesh::new` and `alpha_atlas(&img.rgba)` to `gfx::Atlas`.
use crate::Image;
use alloc::{string::String, vec::Vec};

/// Cells per row of the grid image.
pub const COLS: u32 = 16;
/// The 95 printable ASCII characters, the usual `chars` of a grid.
pub const ASCII: &str = " !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~";

/// RGB8 for `gfx::Atlas` from the grid's RGBA: every channel is the pixel's alpha, so `Material::msdf()`'s
/// 0.5 cut-off draws the glyph pixels that are at least half opaque.
pub fn alpha_atlas(rgba: &[u8]) -> Vec<u8> {
    rgba.as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[3]; 3])
        .collect()
}

pub struct Font {
    chars: String,
    cell: [u32; 2],
    atlas: [u32; 2],
    inset: u32,
    /// Distance between baselines, in ems.
    pub line: f32,
}

impl Font {
    pub fn new(img: &Image, chars: &str, inset: u32) -> Self {
        let rows = (chars.chars().count() as u32).div_ceil(COLS).max(1);
        let cell = [img.w / COLS, img.h / rows];
        Self {
            chars: chars.into(),
            cell,
            atlas: [img.w, img.h],
            inset,
            line: cell[1] as f32 / (cell[1] - 2 * inset) as f32,
        }
    }

    /// Quads for `text`, starting with the pen at the origin; `size` is one em in world units and `\n` goes down
    /// one `line`. Vertices are `[x, y, z, u, v]`, six per glyph. Characters the grid lacks are skipped.
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
        let width = word.chars().filter(|&c| self.index(c).is_some()).count() as f32 * self.advance() * size;
        if pen[0] > 0.0 && pen[0] + width > wrap {
            *pen = [0.0, pen[1] - self.line * size];
        }
        for c in word.chars().chain([' ']) {
            self.step(out, pen, c, size);
        }
    }

    fn index(&self, c: char) -> Option<u32> {
        self.chars.chars().position(|x| x == c).map(|i| i as u32)
    }

    /// Width of a glyph box in ems (the height is 1).
    fn advance(&self) -> f32 {
        (self.cell[0] - 2 * self.inset) as f32 / (self.cell[1] - 2 * self.inset) as f32
    }

    fn step(&self, out: &mut Vec<[f32; 5]>, pen: &mut [f32; 2], c: char, size: f32) {
        let Some(i) = self.index(c) else { return };
        if c != ' ' {
            let (x0, y0, x1, y1) = (pen[0], pen[1], pen[0] + self.advance() * size, pen[1] + size);
            let uv = |col: u32, px: u32, n: u32, a: u32| (col * n + px) as f32 / a as f32;
            let (cx, cy, p) = (i % COLS, i / COLS, self.inset);
            let [cw, ch] = self.cell;
            let [aw, ah] = self.atlas;
            let (u0, u1) = (uv(cx, p, cw, aw), uv(cx, cw - p, cw, aw));
            let (v0, v1) = (uv(cy, p, ch, ah), uv(cy, ch - p, ch, ah)); // v0 = top of the glyph, v1 = bottom
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
        pen[0] += self.advance() * size;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_advances_and_wraps() {
        // 160x10 grid, 10x10 cells with a 1 px padding: glyph boxes are 8x8, line 1.25 em; chars " AB"
        let img = Image { w: 160, h: 10, rgba: Vec::new() };
        let font = Font::new(&img, " AB", 1);
        assert_eq!(font.line, 1.25);
        let v = font.layout(" B\nA", 2.0);
        assert_eq!(v.len(), 12); // the space only moves the pen
        assert_eq!(v[0], [2.0, 0.0, 0.0, 21.0 / 160.0, 9.0 / 10.0]); // B (cell 2) after the space: bottom left
        assert_eq!(v[2], [4.0, 2.0, 0.0, 29.0 / 160.0, 1.0 / 10.0]); // top right, top of the glyph
        assert_eq!(v[6][0..2], [0.0, -2.5]); // second line: one 1.25 em line down

        let (mut out, mut pen) = (Vec::new(), [0.0, 0.0]);
        font.push_word(&mut out, &mut pen, "A", 2.0, 4.0); // 2 wide + a space of 2: the pen is at the limit
        assert_eq!(pen, [4.0, 0.0]);
        font.push_word(&mut out, &mut pen, "A", 2.0, 4.0); // would end at 6 > 4: wraps first
        assert_eq!(pen, [4.0, -2.5]);
        assert_eq!(out[6][0..2], [0.0, -2.5]);
    }
}
