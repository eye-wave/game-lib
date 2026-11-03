//! Tile maps for `gfx`: layers drawn bottom to top, y-sorted sprites, invisible events (signs, doors...) that carry
//! their own dialogue. One fragment shader per kind of tileset reads a layer's cells from a small luma texture, so a
//! tile layer is one quad however big the map is.
//!
//! A level is an asset: the game's build.rs bakes its JSON source into a `.level` file in the workspace `assets/`
//! directory (which a bundle ships next to the executable), and the game loads it at runtime like an image.
//!
//! Level source (`town.json`, not shipped). `textures` lists image files with the kind of tileset each is used as;
//! a file may be listed twice with two kinds. `layers` go bottom to top, each drawing one entry of `textures`; every
//! kind works on every layer. Layers are visual only: they have two coordinates and no height. `events` are a plain
//! list, one cell each.
//! ```json
//! { "textures": [
//!     { "name": "Outside_A2.png", "kind": "autotile" },    // RPG Maker A2 floor autotiles: legend `kind` 0..31
//!     { "name": "Outside_B.png",  "kind": "tiles" },       // 48 px cells of a 16 x 16 sheet: legend `tile`
//!     { "name": "Outside_B.png",  "kind": "sprites" } ],   // y-sorted things: legend `tile` and `size` in cells
//!   "layers": [
//!     { "texture": 0, "legend": { "g": { "kind": 0 }, "b": { "kind": 4, "solid": true } },
//!       "rows": [ "gggg", "gbgg" ] },                      // top row first
//!     { "texture": 1, "legend": { "f": { "tile": [0, 12] } },
//!       "rows": [ "....", "..f." ] },                      // "." means empty unless the legend has it
//!     { "texture": 2, "legend": { "T": { "tile": [5, 11], "size": [1, 2], "solid": true } },
//!       "rows": [ "....", "...T" ] },                      // sprites: the character marks the bottom left cell
//!     { "texture": 1, "legend": { "r": { "tile": [0, 4] } },
//!       "rows": [ "....", "r..." ] } ],                    // a tile layer after a sprites layer draws over it
//!   "events": [                                            // column, row from the top
//!     { "x": 1, "y": 1, "script": [            // Z next to it: lines, and questions with answers
//!         { "say": "Hello!" },
//!         { "ask": { "text": "Like it?", "options": [
//!             { "text": "Yes", "then": [ { "say": "Me too." } ] },
//!             { "text": "No" } ] } },          // no `then`: carry on with the steps after the ask
//!         { "say": "Bye." } ] },
//!     { "x": 3, "y": 0, "goto": { "map": "garden.level", "x": 2, "y": 6 } } ] }  // stepping on it: another map
//! ```
//! `solid` on a tile blocks its cell; on a sprite it blocks the bottom row of its footprint, so the hero can walk
//! behind a tree and be hidden by its crown. A sprite is a structure: trees, houses, towers are sprites of their
//! size, and `tilemap::see_through` thins out any that hides the hero.
//! ```ignore
//! tilemap::build("assets/town.json", "../../assets/town.level"); // build.rs, with features = ["build"]
//! w.insert_resource(tilemap::Sheets::new(load_sheet));           // game: how to load an image file by name
//! let level = tilemap::load("town.level");                       // dies if missing or corrupt
//! let drawn = tilemap::prepare(w, &level);                       // GPU side, once per level
//! let entities = tilemap::spawn(w, &drawn);                      // cheap; despawn them to leave the level
//! // anything that walks among the sprites uses `tilemap::depth(feet_y)` for its z
//! tilemap::see_through(w, &entities, feet, size, 0.4);       // every frame: sprites hiding the hero thin out
//! ```
//!
//! The scripts are baked into a graph: `Action::Script::start` is a node index, and each node says where to go
//! next, so a game only follows links: show the node, wait for Z (or a pick), move to `next` / `then`, stop at `END`.
#![cfg_attr(not(feature = "build"), no_std)]

extern crate alloc;

use alloc::{string::String, vec::Vec};

#[cfg(feature = "build")]
mod bake;
#[cfg(feature = "build")]
pub use bake::{bake, build};
#[cfg(feature = "runtime")]
mod draw;
#[cfg(feature = "runtime")]
pub use draw::{Prepared, Sheet, Sheets, depth, load, prepare, see_through, spawn};

/// "No node": the script ends here.
pub const END: u32 = u32::MAX;

/// How a texture's layers are drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// RPG Maker A2 floor autotiles: cell value = kind 0..31, edges picked from the 8 neighbours.
    Autotile,
    /// Plain 48 px cells of a sheet that is 16 cells wide.
    Tiles,
    /// Things of any size in cells, each its own quad, depth-sorted by y.
    Sprites,
}

/// An image file a level draws with, and how.
pub struct Tileset {
    pub name: String, // in the assets directory
    pub kind: Kind,
}

/// A tile layer (`Autotile` or `Tiles`).
pub struct Layer {
    pub tileset: u32,
    /// Listed after a sprites layer: draws over the sprites and whatever walks among them.
    pub over: bool,
    /// `w * h` bytes, row 0 = the top: 0 = empty, else kind + 1 (`Autotile`) or row * 16 + column + 1 (`Tiles`).
    pub cells: Vec<u8>,
}

/// One object of a sprites layer: cell (`col`, `row`) of the sheet, `w` x `h` cells, standing with its bottom left
/// cell at (`x`, `y`) of the map, so it covers rows `y + 1 - h ..= y`.
pub struct Sprite {
    pub tileset: u32,
    pub col: u32,
    pub row: u32,
    pub w: u32,
    pub h: u32,
    pub x: u32,
    pub y: u32,
}

/// An invisible spot on the map the game reacts to.
pub struct Event {
    pub x: u32, // column
    pub y: u32, // row, from the top
    pub action: Action,
}

pub enum Action {
    /// Talk to it (Z): the script starting at this node.
    Script { start: u32 },
    /// Step on it: go to cell (`x`, `y`) of the level asset `map`.
    Goto { map: String, x: u32, y: u32 },
}

/// One step of a script.
pub enum Node {
    /// Show `text`, wait for the player, go to `next`.
    Say { text: String, next: u32 },
    /// Show `text` and `options` (1 to 4), wait for a pick, go to that option's `then`.
    Ask { text: String, options: Vec<Choice> },
}

pub struct Choice {
    pub label: String,
    pub then: u32,
}

/// A baked level.
pub struct Level {
    pub w: u32,
    pub h: u32,
    pub tilesets: Vec<Tileset>,
    /// Tile layers, bottom to top.
    pub layers: Vec<Layer>,
    pub sprites: Vec<Sprite>,
    /// `w * h` bytes: 1 where a tile or a sprite's footprint blocks.
    pub solid: Vec<u8>,
    pub events: Vec<Event>,
    /// All scripts' steps; indices are what `Action::Script::start`, `Node::Say::next` and `Choice::then` point at.
    pub nodes: Vec<Node>,
}

// Blob, all little-endian, strings are a u32 byte length then UTF-8:
//   w, h, tileset count, layer count, sprite count, event count, node count (u32 each)
//   tilesets: name (string), kind u32 (0 autotile, 1 tiles, 2 sprites)
//   layers: tileset u32, over u32, then w * h cell bytes
//   sprites: tileset, col, row, w, h, x, y (u32 each)
//   events: x, y, tag u32 (0 = script: start u32; 1 = goto: x u32, y u32, map string)
//   nodes: tag u32 (0 = say: next u32, text; 1 = ask: text, option count u32, options: then u32, label)
//   w * h solid flags; row 0 = the top of the map
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (a, b) = self.0.split_at_checked(n)?;
        self.0 = b;
        Some(a)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn str(&mut self) -> Option<String> {
        let n = self.u32()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }
}

impl Level {
    /// `None` if `b` is not a blob that `tilemap::build` wrote (the file comes from disk, so it is checked).
    pub fn parse(b: &[u8]) -> Option<Self> {
        let mut r = Reader(b);
        let (w, h) = (r.u32()?, r.u32()?);
        let n = (w as usize).checked_mul(h as usize)?;
        let (nt, nl, ns, ne, nn) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?);
        let mut tilesets = Vec::new();
        for _ in 0..nt {
            let name = r.str()?;
            let kind = match r.u32()? {
                0 => Kind::Autotile,
                1 => Kind::Tiles,
                2 => Kind::Sprites,
                _ => return None,
            };
            tilesets.push(Tileset { name, kind });
        }
        let mut layers = Vec::new();
        for _ in 0..nl {
            layers.push(Layer {
                tileset: r.u32()?,
                over: r.u32()? != 0,
                cells: r.take(n)?.to_vec(),
            });
        }
        let mut sprites = Vec::new();
        for _ in 0..ns {
            sprites.push(Sprite {
                tileset: r.u32()?,
                col: r.u32()?,
                row: r.u32()?,
                w: r.u32()?,
                h: r.u32()?,
                x: r.u32()?,
                y: r.u32()?,
            });
        }
        let mut events = Vec::new();
        for _ in 0..ne {
            events.push(Event {
                x: r.u32()?,
                y: r.u32()?,
                action: match r.u32()? {
                    0 => Action::Script { start: r.u32()? },
                    1 => Action::Goto {
                        x: r.u32()?,
                        y: r.u32()?,
                        map: r.str()?,
                    },
                    _ => return None,
                },
            });
        }
        let mut nodes = Vec::new();
        for _ in 0..nn {
            nodes.push(match r.u32()? {
                0 => Node::Say {
                    next: r.u32()?,
                    text: r.str()?,
                },
                1 => {
                    let text = r.str()?;
                    let mut options = Vec::new();
                    for _ in 0..r.u32()? {
                        options.push(Choice {
                            then: r.u32()?,
                            label: r.str()?,
                        });
                    }
                    if options.is_empty() {
                        return None;
                    }
                    Node::Ask { text, options }
                }
                _ => return None,
            });
        }
        let solid = r.take(n)?.to_vec();
        // everything that points somewhere must point at something, so using a level never panics
        let kind_of = |t: u32| tilesets.get(t as usize).map(|t| t.kind);
        let ok = |i: u32| i == END || (i as usize) < nodes.len();
        let valid = w > 0
            && h > 0
            && layers
                .iter()
                .all(|l| matches!(kind_of(l.tileset), Some(Kind::Autotile | Kind::Tiles)))
            && sprites.iter().all(|s| {
                kind_of(s.tileset) == Some(Kind::Sprites)
                    && s.w > 0
                    && s.h > 0
                    && s.x + s.w <= w
                    && s.y < h
                    && s.y + 1 >= s.h
            })
            && events.iter().all(|e| {
                e.x < w && e.y < h && !matches!(e.action, Action::Script { start } if !ok(start))
            })
            && nodes.iter().all(|n| match n {
                Node::Say { next, .. } => ok(*next),
                Node::Ask { options, .. } => options.iter().all(|o| ok(o.then)),
            });
        valid.then_some(Self {
            w,
            h,
            tilesets,
            layers,
            sprites,
            solid,
            events,
            nodes,
        })
    }

    /// Does a tile or a sprite's footprint block column `x`, row `y` (from the top)? Off the map does.
    pub fn solid_at(&self, x: i32, y: i32) -> bool {
        x < 0
            || y < 0
            || x >= self.w as i32
            || y >= self.h as i32
            || self.solid[y as usize * self.w as usize + x as usize] != 0
    }
}
