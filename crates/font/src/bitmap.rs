//! Feature `bitmap`: BMFont-style JSON -> font blob (layout in lib.rs), for a build.rs. The atlas image is not
//! touched: it stays a runtime asset (see `alpha_atlas`).
use serde_json::Value;
use std::{env, fs, path::Path};

/// For build.rs: convert the JSON at `src` into `$OUT_DIR/<file stem>.font`; `include_font!("<file stem>")` loads it.
/// one atlas page, and kerning pairs are dropped (like the MSDF baker), add both when a font needs them.
pub fn build(src: &str) {
    println!("cargo:rerun-if-changed={src}");
    let out = convert(&fs::read_to_string(src).unwrap());
    let name = Path::new(src).file_stem().unwrap();
    let dst = Path::new(&env::var("OUT_DIR").unwrap())
        .join(name)
        .with_extension("font");
    fs::write(dst, out).unwrap();
}

fn convert(json: &str) -> Vec<u8> {
    let j: Value = serde_json::from_str(json).expect("font json does not parse");
    let n = |v: &Value, k: &str| {
        v[k].as_f64()
            .unwrap_or_else(|| panic!("font json: no number `{k}`"))
    };
    assert!(
        j["pages"].as_array().is_none_or(|p| p.len() == 1),
        "font json: one atlas page only"
    );
    let size = n(&j["info"], "size").abs(); // px per em (BMFont marks "match char height" with a negative size)
    let c = &j["common"];
    let (sw, sh, base) = (n(c, "scaleW"), n(c, "scaleH"), n(c, "base"));

    let chars = j["chars"].as_array().expect("font json: no `chars`");
    let mut recs: Vec<u32> = Vec::new();
    for g in chars {
        let (x, y, w, h) = (n(g, "x"), n(g, "y"), n(g, "width"), n(g, "height"));
        let (ox, oy) = (n(g, "xoffset"), n(g, "yoffset")); // pen -> left edge, line top -> glyph top (y down)
        let top = (base - oy) / size; // baseline up, as the runtime wants
        // a space has no pixels: advance only
        let quad = if w > 0.0 && h > 0.0 {
            [ox / size, top - h / size, (ox + w) / size, top]
        } else {
            [0.0; 4]
        };
        recs.push(n(g, "id") as u32);
        recs.extend(
            [
                n(g, "xadvance") / size,
                quad[0],
                quad[1],
                quad[2],
                quad[3],
                x / sw,
                y / sh,
                (x + w) / sw,
                (y + h) / sh,
            ]
            .map(|v| (v as f32).to_bits()),
        );
    }
    [
        sw as u32,
        sh as u32,
        chars.len() as u32,
        ((n(c, "lineHeight") / size) as f32).to_bits(),
    ]
    .into_iter()
    .chain(recs)
    .flat_map(u32::to_le_bytes)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_to_blob_to_quads() {
        // 10 px font on a 64x32 atlas: a space (advance 5) and an 'A' of 6x8 px at (16, 8), 2 px below the line top
        let json = r#"{"pages":["f.png"],"info":{"size":10},
            "common":{"lineHeight":12,"base":8,"scaleW":64,"scaleH":32},
            "chars":[
              {"id":32,"x":0,"y":0,"width":0,"height":0,"xoffset":0,"yoffset":0,"xadvance":5,"page":0},
              {"id":65,"x":16,"y":8,"width":6,"height":8,"xoffset":1,"yoffset":2,"xadvance":7,"page":0}]}"#;
        let blob = convert(json);
        let font = crate::Font::parse(&blob);
        assert_eq!((font.w, font.h, font.rgb.len()), (64, 32, 0));
        assert!((font.line - 1.2).abs() < 1e-5);

        let v = font.layout(" A", 10.0); // the space moves the pen 5 px, then 'A' at x = 5 + 1
        let near = |a: [f32; 5], b: [f32; 5]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-4);
        assert_eq!(v.len(), 6);
        assert!(near(v[0], [6.0, -2.0, 0.0, 0.25, 0.5])); // bottom left: 2 px under the baseline, cell bottom
        assert!(near(v[2], [12.0, 6.0, 0.0, 0.34375, 0.25])); // top right: 6 px over the baseline, cell top
    }
}
