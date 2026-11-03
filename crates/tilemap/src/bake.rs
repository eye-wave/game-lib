//! Feature `build`: level JSON -> level blob (layout in lib.rs), for a build.rs. The source is read straight into
//! the types below with serde, so a typo in a field name is an error with its position.
use crate::{Action, Choice, END, Event, Node};
use serde::Deserialize;
use std::{collections::HashMap, fs, path::Path};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    ground: Layer<Tile>,
    objects: Option<Layer<Thing>>,
    events: Option<Layer<Spot>>,
}

/// One layer: `rows` of characters, each looked up in `legend`. `empty` (default "." in the optional layers) is a
/// character that means nothing there; the ground has none, every cell needs a tile.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Layer<T> {
    legend: HashMap<char, T>,
    rows: Vec<String>,
    empty: Option<char>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Tile {
    kind: u8, // what the tileset shader reads: an RPG Maker A2 kind 0..31
    #[serde(default)]
    solid: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Thing {
    tile: [u32; 2], // [column, row] in the objects sheet (16 x 16 cells; the last cell is unusable)
    #[serde(default)]
    solid: bool,
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

/// The cells of `l` in reading order: `None` for `empty`, else the character and its legend entry.
fn grid<'a, T>(
    l: &'a Layer<T>,
    name: &str,
    (w, h): (usize, usize),
    empty: Option<char>,
) -> Result<Vec<Option<(char, &'a T)>>, String> {
    if l.rows.len() != h {
        return Err(format!("{name}: {} rows, the ground has {h}", l.rows.len()));
    }
    let empty = l.empty.or(empty);
    let mut out = Vec::new();
    for (y, row) in l.rows.iter().enumerate() {
        if row.chars().count() != w {
            return Err(format!(
                "{name}: row {y} is {} wide, the ground is {w}",
                row.chars().count()
            ));
        }
        for (x, c) in row.chars().enumerate() {
            out.push(if Some(c) == empty {
                None
            } else {
                let t = l.legend.get(&c).ok_or(format!(
                    "{name}: row {y}, column {x}: `{c}` is not in the legend"
                ))?;
                Some((c, t))
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
    let h = src.ground.rows.len();
    let w = src
        .ground
        .rows
        .first()
        .ok_or("ground: no rows")?
        .chars()
        .count();
    let size = (w, h);

    let (mut cells, mut solid) = (Vec::new(), Vec::new());
    for t in grid(&src.ground, "ground", size, None)?
        .into_iter()
        .flatten()
    {
        cells.push(t.1.kind);
        solid.push(t.1.solid as u8);
    }
    let mut objects = vec![0; w * h];
    if let Some(l) = &src.objects {
        for (i, g) in grid(l, "objects", size, Some('.'))?.into_iter().enumerate() {
            let Some((c, t)) = g else { continue };
            let [col, row] = t.tile;
            if col >= 16 || row >= 16 || row * 16 + col > 254 {
                return Err(format!(
                    "objects: `{c}` is cell [{col}, {row}], the sheet is 16 x 16 and the last cell is unusable"
                ));
            }
            objects[i] = (row * 16 + col + 1) as u8;
            solid[i] |= t.solid as u8;
        }
    }

    let (mut nodes, mut events) = (Vec::new(), Vec::new());
    let mut scripts = HashMap::new(); // a legend character's script is baked once, however many cells use it
    if let Some(l) = &src.events {
        for (i, g) in grid(l, "events", size, Some('.'))?.into_iter().enumerate() {
            let Some((c, spot)) = g else { continue };
            let action = match spot {
                Spot::Script(steps) => {
                    let start = match scripts.get(&c) {
                        Some(&s) => s,
                        None => {
                            let s = sequence(steps, END, &mut nodes, &format!("events: `{c}`"))?;
                            scripts.insert(c, s);
                            s
                        }
                    };
                    Action::Script { start }
                }
                Spot::Goto { map, x, y } => Action::Goto {
                    map: map.clone(),
                    x: *x,
                    y: *y,
                },
            };
            events.push(Event {
                x: (i % w) as u32,
                y: (i / w) as u32,
                action,
            });
        }
    }

    let mut out = Vec::new();
    for v in [w as u32, h as u32, events.len() as u32, nodes.len() as u32] {
        put(&mut out, v);
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
    out.extend(cells);
    out.extend(objects);
    out.extend(solid);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Level;

    const SRC: &str = r#"{
      "ground": { "legend": { ".": { "kind": 0 }, "b": { "kind": 4, "solid": true } }, "rows": [".b.", "bb."] },
      "objects": { "legend": { "w": { "tile": [3, 9], "solid": true } }, "rows": ["w..", "..."] },
      "events": { "legend": {
          "1": { "script": [
              { "say": "hi" },
              { "ask": { "text": "q?", "options": [ { "text": "a", "then": [ { "say": "A" } ] }, { "text": "b" } ] } },
              { "say": "bye" } ] },
          "D": { "goto": { "map": "other.level", "x": 2, "y": 3 } } },
        "rows": [".1D", "..."] } }"#;

    #[test]
    fn bakes_and_parses_a_level() {
        let lv = Level::parse(&bake(SRC).unwrap()).unwrap();
        assert_eq!((lv.w, lv.h), (3, 2));
        assert_eq!(lv.cells, [0, 4, 0, 4, 4, 0]); // row 0 is the top
        assert_eq!(lv.objects, [3 + 9 * 16 + 1, 0, 0, 0, 0, 0]);
        assert_eq!(lv.solid, [1, 1, 0, 1, 1, 0]); // the bushes and the object
        assert!(lv.solid_at(0, 0) && !lv.solid_at(2, 0) && lv.solid_at(-1, 0) && lv.solid_at(3, 0));
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
        let ground = r#""ground": { "legend": { ".": { "kind": 0 } }, "rows": [".."] }"#;
        assert!(bake(&format!("{{ {ground} }}")).is_ok());
        assert!(
            bake(r#"{ "ground": { "legend": { ".": { "kind": 0 } }, "rows": [".x"] } }"#).is_err()
        ); // not in legend
        assert!(bake(&format!("{{ {ground}, \"objects\": {{ \"legend\": {{ \"a\": {{ \"tile\": [16, 0] }} }}, \"rows\": [\"a.\"] }} }}")).is_err());
        assert!(bake(&format!("{{ {ground}, \"typo\": 1 }}")).is_err()); // unknown field
    }
}
