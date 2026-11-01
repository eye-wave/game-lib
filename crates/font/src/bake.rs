//! Feature `build`: font file -> atlas blob (layout in lib.rs), for a build.rs.
use std::{collections::BTreeSet, env, fs, io::Read, path::Path};
use ttf_parser::{Face, OutlineBuilder};

/// Distance field range in atlas px (total, so half of it is free around the glyph in its cell). Text is drawn with a
/// hard 0.5 threshold, so it only has to be wide enough for the corner-preserving trick to work; 4 is msdfgen's usual.
const RANGE: f64 = 4.0;

// msdf/shim.cpp over the msdfgen submodule: outline commands (0 move, 1 line, 2 quad, 3 cubic, 4 close; points in
// `pts`) -> `cell` x `cell` x 3 floats, top row first, and the cell's quad in font units. 0 ok, 1 no outline, 2 invalid.
unsafe extern "C" {
    fn msdf_glyph(
        ops: *const u8,
        n_ops: usize,
        pts: *const f64,
        cell: u32,
        range_px: f64,
        out: *mut f32,
        quad: *mut f64,
    ) -> i32;
}

#[derive(Default)]
struct Outline {
    ops: Vec<u8>,
    pts: Vec<f64>,
}
impl Outline {
    fn add(&mut self, op: u8, p: &[f32]) {
        self.ops.push(op);
        self.pts.extend(p.iter().map(|&v| v as f64));
    }
}
impl OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.add(0, &[x, y])
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.add(1, &[x, y])
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.add(2, &[x1, y1, x, y])
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.add(3, &[x1, y1, x2, y2, x, y])
    }
    fn close(&mut self) {
        self.add(4, &[])
    }
}

/// For build.rs: bake `chars` (a space is always added) of the .ttf, .otf or .woff at `src` into
/// `$OUT_DIR/<file stem>.font`, `cell` x `cell` px per glyph (32 is plenty for UI text, raise it for big titles).
/// Characters the font lacks are skipped. `include_font!("<file stem>")` loads the result.
pub fn build(src: &str, chars: &str, cell: u32) {
    println!("cargo:rerun-if-changed={src}");
    let data = sfnt(fs::read(src).unwrap());
    let face = Face::parse(&data, 0).unwrap();
    let em = face.units_per_em() as f64;
    assert!(
        cell as f64 > RANGE,
        "glyph cell too small for the distance range, raise `cell`"
    );

    let set: BTreeSet<char> = chars.chars().chain([' ']).collect();
    let glyphs: Vec<_> = set
        .into_iter()
        .filter_map(|c| Some((c, face.glyph_index(c)?)))
        .collect();
    let cols = (glyphs.len() as f64).sqrt().ceil() as u32;
    let rows = (glyphs.len() as u32).div_ceil(cols);
    let (w, h) = (cols * cell, rows * cell);
    let mut rgb = vec![0u8; (w * h * 3) as usize]; // 0 = far outside: never drawn
    let mut recs: Vec<u32> = Vec::new();

    for (i, &(c, id)) in glyphs.iter().enumerate() {
        let (cx, cy) = (i as u32 % cols, i as u32 / cols);
        let advance = face.glyph_hor_advance(id).unwrap_or(0) as f64 / em;
        let (mut quad, mut uv) = ([0.0f64; 4], [0.0f64; 4]);
        // no outline (a space) -> advance only
        let mut o = Outline::default();
        if face.outline_glyph(id, &mut o).is_some() {
            let mut px = vec![0f32; (cell * cell * 3) as usize]; // top row first
            let mut q = [0.0f64; 4];
            let r = unsafe {
                msdf_glyph(
                    o.ops.as_ptr(),
                    o.ops.len(),
                    o.pts.as_ptr(),
                    cell,
                    RANGE,
                    px.as_mut_ptr(),
                    q.as_mut_ptr(),
                )
            };
            assert!(r != 2, "invalid outline for {c:?}");
            if r == 0 {
                for y in 0..cell {
                    for x in 0..cell * 3 {
                        let v = px[(y * cell * 3 + x) as usize];
                        rgb[(((cy * cell + y) * w) * 3 + cx * cell * 3 + x) as usize] =
                            (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                    }
                }
                quad = q.map(|v| v / em); // the cell in em, y up
                uv = [
                    (cx * cell) as f64 / w as f64,
                    (cy * cell) as f64 / h as f64,
                    ((cx + 1) * cell) as f64 / w as f64,
                    ((cy + 1) * cell) as f64 / h as f64,
                ];
            }
        }
        recs.push(c as u32);
        recs.extend(
            [
                advance as f32,
                quad[0] as f32,
                quad[1] as f32,
                quad[2] as f32,
                quad[3] as f32,
            ]
            .map(f32::to_bits),
        );
        recs.extend(uv.map(|v| (v as f32).to_bits()));
    }

    let line = (face.ascender() as f64 - face.descender() as f64 + face.line_gap() as f64) / em;
    let mut out: Vec<u8> = [w, h, glyphs.len() as u32, (line as f32).to_bits()]
        .into_iter()
        .chain(recs)
        .flat_map(u32::to_le_bytes)
        .collect();
    out.extend(rgb);
    let name = Path::new(src).file_stem().unwrap();
    fs::write(
        Path::new(&env::var("OUT_DIR").unwrap())
            .join(name)
            .with_extension("font"),
        out,
    )
    .unwrap();
}

/// .woff (zlib per table) -> plain sfnt, which is what ttf-parser reads; .ttf/.otf pass through.
/// no .woff2 (needs brotli), @fontsource ships .woff next to it.
fn sfnt(d: Vec<u8>) -> Vec<u8> {
    if !d.starts_with(b"wOFF") {
        return d;
    }
    let be = |o: usize| u32::from_be_bytes(d[o..o + 4].try_into().unwrap()) as usize;
    let n = u16::from_be_bytes([d[12], d[13]]) as usize;
    let sel = n.ilog2() as usize; // entrySelector; searchRange = 16 * 2^sel
    let mut out = Vec::new();
    out.extend_from_slice(&d[4..8]); // flavor
    out.extend((n as u16).to_be_bytes());
    out.extend(((16usize << sel) as u16).to_be_bytes());
    out.extend((sel as u16).to_be_bytes());
    out.extend(((n * 16 - (16usize << sel)) as u16).to_be_bytes());
    let mut body: Vec<u8> = Vec::new();
    let base = 12 + n * 16;
    for i in 0..n {
        let e = 44 + i * 20; // tag, offset, compLength, origLength, checksum
        let (off, comp, orig) = (be(e + 4), be(e + 8), be(e + 12));
        let t = &d[off..off + comp];
        out.extend_from_slice(&d[e..e + 4]);
        out.extend_from_slice(&d[e + 16..e + 20]);
        out.extend(((base + body.len()) as u32).to_be_bytes());
        out.extend((orig as u32).to_be_bytes());
        if comp < orig {
            let start = body.len();
            flate2::read::ZlibDecoder::new(t)
                .read_to_end(&mut body)
                .unwrap();
            assert_eq!(body.len() - start, orig, "corrupt .woff table");
        } else {
            body.extend_from_slice(t);
        }
        body.resize(body.len().next_multiple_of(4), 0);
    }
    out.extend(body);
    out
}
