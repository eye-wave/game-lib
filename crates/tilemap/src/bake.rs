//! Feature `build`: level JSON -> level blob (layout in lib.rs), for a build.rs. The source is read straight into
//! the types below with serde, so a typo in a field name is an error with its position.
use crate::{Action, Choice, END, Event, Node};
use serde::Deserialize;
use std::{collections::HashMap, fs, path::Path};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    textures: Vec<TextureSrc>,
    layers: Vec<LayerSrc>,
    #[serde(default)]
    events: Vec<EventSrc>,
}

/// An image file (by name in the assets directory) and what it is used as.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextureSrc {
    name: String,
    kind: KindSrc,
}

#[derive(Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
enum KindSrc {
    Autotile,
    Tiles,
    Sprites,
}

/// One visual layer: `rows` of characters, each looked up in `legend`; it draws `texture` (an index into
/// `textures`), and its legend entries have to suit that texture's kind. `empty` (default ".") is the character
/// that means nothing there, unless the legend defines it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LayerSrc {
    texture: usize,
    legend: HashMap<char, Entry>,
    rows: Vec<String>,
    empty: Option<char>,
}

/// `kind` for autotile layers; `tile` (and `size` for sprites) for the others.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    kind: Option<u8>,       // autotile: RPG Maker A2 kind 0..31
    tile: Option<[u32; 2]>, // tiles, sprites: [column, row] of the (top left) cell in the sheet
    size: Option<[u32; 2]>, // sprites: width and height in cells, default [1, 1]
    #[serde(default)]
    solid: bool, // sprites: the bottom row of the footprint
}

/// Where it is (two coordinates: column, row from the top) and what it does.
#[derive(Deserialize)]
struct EventSrc {
    x: u32,
    y: u32,
    #[serde(flatten)]
    spot: Spot,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Spot {
    Script(Vec<Step>),
    Goto { map: String, x: u32, y: u32 },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Step {
    Say(String),
    Ask(Question),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Question {
    text: String,
    options: Vec<Answer>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Answer {
    text: String,
    #[serde(default)]
    then: Vec<Step>, // empty: carry on after the question
}

/// For build.rs: bake the level source at `src` into the file `dst` (relative to the package, like `src`; use the
/// workspace `assets/` directory so `tilemap::load` finds it and bundles ship it). Panics with the file and the
/// mistake, so the build fails instead of shipping a bad level.
pub fn build(src: &str, dst: &str) {
    println!("cargo:rerun-if-changed={src}");
    let json = fs::read_to_string(src).unwrap_or_else(|e| panic!("{src}: {e}"));
    let blob = bake(&json).unwrap_or_else(|e| panic!("{src}: {e}"));
    if let Some(dir) = Path::new(dst).parent() {
        fs::create_dir_all(dir).unwrap();
    }
    fs::write(dst, blob).unwrap_or_else(|e| panic!("{dst}: {e}"));
}

fn put(out: &mut Vec<u8>, v: u32) {
    out.extend(v.to_le_bytes());
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put(out, s.len() as u32);
    out.extend(s.as_bytes());
}

/// The cells of `l` in reading order: `None` for empty, else the character and its legend entry.
fn grid<'a>(
    l: &'a LayerSrc,
    name: &str,
    (w, h): (usize, usize),
) -> Result<Vec<Option<(char, &'a Entry)>>, String> {
    if l.rows.len() != h {
        return Err(format!(
            "{name}: {} rows, the first layer has {h}",
            l.rows.len()
        ));
    }
    let mut out = Vec::new();
    for (y, row) in l.rows.iter().enumerate() {
        if row.chars().count() != w {
            return Err(format!(
                "{name}: row {y} is {} wide, the first layer is {w}",
                row.chars().count()
            ));
        }
        for (x, c) in row.chars().enumerate() {
            out.push(match l.legend.get(&c) {
                Some(e) => Some((c, e)),
                None if c == l.empty.unwrap_or('.') => None,
                None => {
                    return Err(format!(
                        "{name}: row {y}, column {x}: `{c}` is not in the legend"
                    ));
                }
            });
        }
    }
    Ok(out)
}

/// Appends the nodes of `steps` to `out`, last step first so each knows what follows it, and returns the first
/// one's index; `cont` is where the sequence goes when it runs out (`END`, or the step after an `ask`).
fn sequence(steps: &[Step], cont: u32, out: &mut Vec<Node>, at: &str) -> Result<u32, String> {
    let mut next = cont;
    for (i, s) in steps.iter().enumerate().rev() {
        let at = format!("{at}, step {i}");
        let node = match s {
            Step::Say(text) => Node::Say {
                text: text.clone(),
                next,
            },
            Step::Ask(q) => {
                if !(1..=4).contains(&q.options.len()) {
                    return Err(format!("{at}: an ask needs 1 to 4 options"));
                }
                let mut options = Vec::new();
                for (k, a) in q.options.iter().enumerate() {
                    // a branch that runs out continues after the ask, like a call returning
                    let then = sequence(&a.then, next, out, &format!("{at}, option {k}"))?;
                    options.push(Choice {
                        label: a.text.clone(),
                        then,
                    });
                }
                Node::Ask {
                    text: q.text.clone(),
                    options,
                }
            }
        };
        out.push(node);
        next = (out.len() - 1) as u32;
    }
    Ok(next)
}

/// The level blob for source `json`, or what is wrong with it.
pub fn bake(json: &str) -> Result<Vec<u8>, String> {
    let src: Source = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if src.layers.is_empty() || src.layers.len() > 100 {
        return Err("`layers` needs 1 to 100 layers".into());
    }
    let h = src.layers[0].rows.len();
    let w = src.layers[0]
        .rows
        .first()
        .ok_or("layers[0]: no rows")?
        .chars()
        .count();
    let size = (w, h);

    let (mut layers, mut sprites, mut solid) = (Vec::new(), Vec::new(), vec![0u8; w * h]);
    let mut over = false; // after the first sprites layer
    for (li, l) in src.layers.iter().enumerate() {
        let name = format!("layers[{li}]");
        let t = src
            .textures
            .get(l.texture)
            .ok_or(format!("{name}: there is no texture {}", l.texture))?;
        let mut cells = vec![0u8; w * h];
        for (i, g) in grid(l, &name, size)?.into_iter().enumerate() {
            let Some((c, e)) = g else { continue };
            let at = format!("{name}: `{c}`");
            let (x, y) = (i % w, i / w);
            match t.kind {
                KindSrc::Autotile => {
                    let k = e
                        .kind
                        .filter(|&k| k <= 31)
                        .ok_or(format!("{at} needs `kind` 0..31 (autotile)"))?;
                    if e.tile.is_some() || e.size.is_some() {
                        return Err(format!("{at}: `tile` and `size` are for tiles and sprites"));
                    }
                    cells[i] = k + 1;
                    solid[i] |= e.solid as u8;
                }
                KindSrc::Tiles => {
                    let [col, row] = e.tile.ok_or(format!("{at} needs `tile` (tiles)"))?;
                    if col >= 16
                        || row >= 16
                        || row * 16 + col > 254
                        || e.kind.is_some()
                        || e.size.is_some()
                    {
                        return Err(format!(
                            "{at}: needs `tile` [column, row] inside a 16 x 16 sheet (the last cell is unusable), no `kind`, no `size`"
                        ));
                    }
                    cells[i] = (row * 16 + col + 1) as u8;
                    solid[i] |= e.solid as u8;
                }
                KindSrc::Sprites => {
                    let [col, row] = e.tile.ok_or(format!("{at} needs `tile` (sprites)"))?;
                    let [sw, sh] = e.size.unwrap_or([1, 1]);
                    if e.kind.is_some() || sw == 0 || sh == 0 || col + sw > 16 || row + sh > 16 {
                        return Err(format!(
                            "{at}: needs `tile` and `size` inside the 16 x 16 sheet, no `kind`"
                        ));
                    }
                    // the character marks the bottom left cell; it has to fit on the map
                    if x + sw as usize > w || y + 1 < sh as usize {
                        return Err(format!(
                            "{at} at column {x}, row {y}: a {sw} x {sh} sprite does not fit on the map"
                        ));
                    }
                    sprites.push((l.texture as u32, [col, row, sw, sh, x as u32, y as u32]));
                    if e.solid {
                        for dx in 0..sw as usize {
                            solid[y * w + x + dx] = 1;
                        }
                    }
                }
            }
        }
        if t.kind != KindSrc::Sprites {
            layers.push((l.texture as u32, over, cells));
        } else {
            over = true;
        }
    }

    let (mut nodes, mut events) = (Vec::new(), Vec::new());
    for (i, e) in src.events.iter().enumerate() {
        if e.x as usize >= w || e.y as usize >= h {
            return Err(format!(
                "events[{i}]: ({}, {}) is outside the {w} x {h} map",
                e.x, e.y
            ));
        }
        let action = match &e.spot {
            Spot::Script(steps) => Action::Script {
                start: sequence(steps, END, &mut nodes, &format!("events[{i}]"))?,
            },
            Spot::Goto { map, x, y } => Action::Goto {
                map: map.clone(),
                x: *x,
                y: *y,
            },
        };
        events.push(Event {
            x: e.x,
            y: e.y,
            action,
        });
    }

    let mut out = Vec::new();
    for v in [
        w,
        h,
        src.textures.len(),
        layers.len(),
        sprites.len(),
        events.len(),
        nodes.len(),
    ] {
        put(&mut out, v as u32);
    }
    for t in &src.textures {
        put_str(&mut out, &t.name);
        put(&mut out, t.kind as u32);
    }
    for (tex, over, cells) in &layers {
        put(&mut out, *tex);
        put(&mut out, *over as u32);
        out.extend(cells);
    }
    for (tex, d) in &sprites {
        put(&mut out, *tex);
        for v in d {
            put(&mut out, *v);
        }
    }
    for e in &events {
        put(&mut out, e.x);
        put(&mut out, e.y);
        match &e.action {
            Action::Script { start } => {
                put(&mut out, 0);
                put(&mut out, *start);
            }
            Action::Goto { map, x, y } => {
                put(&mut out, 1);
                put(&mut out, *x);
                put(&mut out, *y);
                put_str(&mut out, map);
            }
        }
    }
    for n in &nodes {
        match n {
            Node::Say { text, next } => {
                put(&mut out, 0);
                put(&mut out, *next);
                put_str(&mut out, text);
            }
            Node::Ask { text, options } => {
                put(&mut out, 1);
                put_str(&mut out, text);
                put(&mut out, options.len() as u32);
                for o in options {
                    put(&mut out, o.then);
                    put_str(&mut out, &o.label);
                }
            }
        }
    }
    out.extend(solid);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Kind, Level};

    const SRC: &str = r#"{
      "textures": [ { "name": "a.png", "kind": "autotile" }, { "name": "b.png", "kind": "sprites" }, { "name": "b.png", "kind": "tiles" } ],
      "layers": [
        { "texture": 0, "legend": { "g": { "kind": 0 }, "b": { "kind": 4, "solid": true } }, "rows": ["gbg", "bbg"] },
        { "texture": 1, "legend": { "T": { "tile": [5, 11], "size": [1, 2], "solid": true } }, "rows": ["...", ".T."] },
        { "texture": 2, "legend": { "r": { "tile": [0, 4] } }, "rows": ["r..", "..."] } ],
      "events": [
        { "x": 1, "y": 0, "script": [
            { "say": "hi" },
            { "ask": { "text": "q?", "options": [ { "text": "a", "then": [ { "say": "A" } ] }, { "text": "b" } ] } },
            { "say": "bye" } ] },
        { "x": 2, "y": 0, "goto": { "map": "other.level", "x": 2, "y": 3 } } ] }"#;

    #[test]
    fn bakes_and_parses_a_level() {
        let lv = Level::parse(&bake(SRC).unwrap()).unwrap();
        assert_eq!((lv.w, lv.h), (3, 2));
        assert_eq!(
            (lv.tilesets[1].name.as_str(), lv.tilesets[1].kind),
            ("b.png", Kind::Sprites)
        );
        // two tile layers (the sprites are not one), the second listed after the sprites so it is `over`
        assert_eq!(lv.layers.len(), 2);
        assert_eq!(
            (lv.layers[0].tileset, lv.layers[0].over, lv.layers[1].over),
            (0, false, true)
        );
        assert_eq!(lv.layers[0].cells, [1, 5, 1, 5, 5, 1]); // kind + 1, row 0 is the top
        assert_eq!(lv.layers[1].cells, [4 * 16 + 1, 0, 0, 0, 0, 0]); // row * 16 + column + 1
        // the sprite's character marks its bottom left cell, (1, 1); 2 cells tall, it reaches up to row 0
        assert_eq!(lv.sprites.len(), 1);
        let s = &lv.sprites[0];
        assert_eq!((s.col, s.row, s.w, s.h, s.x, s.y), (5, 11, 1, 2, 1, 1));
        assert_eq!(lv.solid, [0, 1, 0, 1, 1, 0]); // the bushes; the sprite's base (1, 1) is one of those cells
        assert!(lv.solid_at(1, 0) && !lv.solid_at(0, 0) && lv.solid_at(-1, 0) && lv.solid_at(3, 0));
        assert_eq!(lv.events.len(), 2);
        let (script, door) = (&lv.events[0], &lv.events[1]);
        assert_eq!((script.x, script.y, door.x, door.y), (1, 0, 2, 0));
        assert!(matches!(&door.action, Action::Goto { map, x: 2, y: 3 } if map == "other.level"));
        // built back to front: bye = 0, A = 1, ask = 2, hi = 3
        assert!(matches!(script.action, Action::Script { start: 3 }));
        assert!(matches!(&lv.nodes[3], Node::Say { text, next: 2 } if text == "hi"));
        let Node::Ask { options, .. } = &lv.nodes[2] else {
            panic!("not an ask")
        };
        assert_eq!([options[0].then, options[1].then], [1, 0]); // A, then on to bye; `b` goes straight to bye
        assert!(matches!(&lv.nodes[1], Node::Say { next: 0, .. }));
        assert!(matches!(&lv.nodes[0], Node::Say { next: END, .. }));
    }

    #[test]
    fn rejects_mistakes() {
        let ok = r#"{ "textures": [ { "name": "a", "kind": "autotile" } ],
                      "layers": [ { "texture": 0, "legend": { "g": { "kind": 0 } }, "rows": ["gg"] } ]"#;
        assert!(bake(&format!("{ok} }}")).is_ok());
        assert!(bake(&format!("{ok}, \"typo\": 1 }}")).is_err()); // unknown field
        assert!(
            bake(&format!(
                "{ok}, \"events\": [ {{ \"x\": 5, \"y\": 0, \"script\": [] }} ] }}"
            ))
            .is_err()
        ); // off the map
        assert!(bake(&ok.replace("\"rows\": [\"gg\"]", "\"rows\": [\"gx\"]")).is_err()); // `x` is not in the legend
        assert!(bake(&ok.replace("\"kind\": 0", "\"tile\": [0, 0]")).is_err()); // a tile entry on an autotile layer
    }
}
