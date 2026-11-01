//! Feature `build`: font file -> atlas blob (layout in lib.rs), for a build.rs.
use msdfgen::{Bitmap, FontExt, MsdfGeneratorConfig, Range, Rgb};
use std::{collections::BTreeSet, env, fs, io::Read, path::Path};
use ttf_parser::Face;

/// Distance field range in atlas px. Text is drawn with a hard 0.5 threshold, so it only has to be wide enough
/// for the corner-preserving trick to work; 4 is msdfgen's usual.
const RANGE: f64 = 4.0;

/// For build.rs: bake `chars` (a space is always added) of the .ttf, .otf or .woff at `src` into
/// `$OUT_DIR/<file stem>.font`, `cell` x `cell` px per glyph (32 is plenty for UI text, raise it for big titles).
/// Characters the font lacks are skipped. `include_font!("<file stem>")` loads the result.
pub fn build(src: &str, chars: &str, cell: u32) {
    println!("cargo:rerun-if-changed={src}");
    let data = sfnt(fs::read(src).unwrap());
    let face = Face::parse(&data, 0).unwrap();
    let em = face.units_per_em() as f64;

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
        if let Some(mut shape) = face.glyph_shape(id) {
            let b = shape.get_bound();
            let frame = b
                .autoframe(cell, cell, Range::Px(RANGE), None)
                .expect("glyph cell too small for the distance range, raise `cell`");
            shape.edge_coloring_simple(3.0, 0);
            let mut bmp = Bitmap::<Rgb<f32>>::new(cell, cell);
            shape.generate_msdf(&mut bmp, &frame, &MsdfGeneratorConfig::default());

            // msdfgen rows run bottom to top, the atlas top to bottom
            let px: Vec<f32> = bmp
                .raw_pixels()
                .chunks_exact(4)
                .map(|v| f32::from_ne_bytes(v.try_into().unwrap()))
                .collect();
            for y in 0..cell {
                for x in 0..cell * 3 {
                    let v = px[((cell - 1 - y) * cell * 3 + x) as usize];
                    rgb[(((cy * cell + y) * w) * 3 + cx * cell * 3 + x) as usize] =
                        (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
            }

            // autoframe fits the glyph centred, uniformly scaled, with RANGE px free on every side (checked by
            // eye on the demo text; msdfgen does not hand the scale back), so the cell covers this much font space
            let size = (b.right - b.left).max(b.top - b.bottom);
            let half = cell as f64 / 2.0 / ((cell as f64 - 2.0 * RANGE) / size) / em;
            let (mx, my) = ((b.left + b.right) / 2.0 / em, (b.bottom + b.top) / 2.0 / em);
            quad = [mx - half, my - half, mx + half, my + half];
            uv = [
                (cx * cell) as f64 / w as f64,
                (cy * cell) as f64 / h as f64,
                ((cx + 1) * cell) as f64 / w as f64,
                ((cy + 1) * cell) as f64 / h as f64,
            ];
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
