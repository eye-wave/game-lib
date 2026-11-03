//! Tile maps for `gfx`. The level is a luma texture, one byte per cell holding the tile kind; one fragment shader
//! per tileset style reads it and the tileset image, so a whole map is one quad. Events (signs, people...) carry
//! their own dialogue: lines to say and questions with options to pick from.
//!
//! A level is an asset: the game's build.rs bakes its JSON source into a `.level` file in the workspace `assets/`
//! directory (which a bundle ships next to the executable), and the game loads it at runtime like an image.
//!
//! Level source (`town.json`, not shipped): three layers of ASCII rows, each with a legend. The ground and the
//! objects are what the shader draws; the events are invisible spots the game reacts to.
//! ```json
//! { "ground": {
//!     "legend": { ".": { "kind": 0 }, "b": { "kind": 4, "solid": true } },   // kind = what the tileset shader reads
//!     "rows": [ "....", ".b.." ] },                                           // top row first
//!   "objects": {                                                               // optional; "." is empty
//!     "legend": { "w": { "tile": [3, 9], "solid": true } },                    // [column, row] in the objects sheet
//!     "rows": [ "....", "..w." ] },
//!   "events": {                                                                // optional; "." is empty
//!     "legend": {
//!       "1": { "script": [                       // Z next to it: lines, and questions with answers
//!         { "say": "Hello!" },
//!         { "ask": { "text": "Like it?", "options": [
//!             { "text": "Yes", "then": [ { "say": "Me too." } ] },
//!             { "text": "No" } ] } },            // no `then`: carry on with the steps after the ask
//!         { "say": "Bye." } ] },
//!       "D": { "goto": { "map": "garden.level", "x": 2, "y": 6 } } },            // stepping on it: another map
//!     "rows": [ "....", "1.D." ] } }
//! ```
//! ```ignore
//! tilemap::build("assets/town.json", "../../assets/town.level"); // build.rs, with features = ["build"]
//! let level = tilemap::load("town.level");                        // game: dies if missing or corrupt
//! let sets = tilemap::rpgmaker::tilesets(w, ground_sheet, object_sheet); // once: the images are shared by levels
//! let material = tilemap::rpgmaker::material(w, &level, sets);           // one tileset style, one per level
//! tilemap::spawn(w, material, (level.w, level.h));                       // later `set` swaps the level
//! ```
//! New tileset styles are one module each, next to `rpgmaker`: a shader, and a `material` function that adds the
//! textures it wants. `Level` and the baked file stay the same.
//!
//! The scripts are baked into a graph: `Event::start` is a node index, and each node says where to go next, so a
//! game only follows links: show the node, wait for Z (or a pick), move to `next` / `then`, stop at `END`.
#![cfg_attr(not(feature = "build"), no_std)]

extern crate alloc;

use alloc::{string::String, vec::Vec};

#[cfg(feature = "build")]
mod bake;
#[cfg(feature = "build")]
pub use bake::{bake, build};
#[cfg(feature = "runtime")]
pub mod rpgmaker;
#[cfg(feature = "runtime")]
mod spawn;
#[cfg(feature = "runtime")]
pub use spawn::{load, set, spawn};

/// "No node": the script ends here.
pub const END: u32 = u32::MAX;

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
    /// The ground kind of each cell, `w * h` bytes, row 0 = the top. This is a texture's pixels.
    pub cells: Vec<u8>,
    /// The object of each cell: 0 = none, else its cell in the objects sheet + 1 (row * 16 + column + 1).
    pub objects: Vec<u8>,
    /// 1 where the ground or the object is `solid`.
    pub solid: Vec<u8>,
    pub events: Vec<Event>,
    /// All scripts' steps; indices are what `Action::Script::start`, `Node::Say::next` and `Choice::then` point at.
    pub nodes: Vec<Node>,
}

// Blob, all little-endian, strings are a u32 byte length then UTF-8:
//   w, h, event count, node count (u32 each)
//   events: x, y, tag u32 (0 = script: start u32; 1 = goto: x u32, y u32, map string)
//   nodes: tag u32 (0 = say: next u32, text; 1 = ask: text, option count u32, options: then u32, label)
//   w * h ground kinds, w * h objects, w * h solid flags; row 0 = the top of the map
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
        let (ne, nn) = (r.u32()?, r.u32()?);
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
        let n = (w as usize).checked_mul(h as usize)?;
        let (cells, objects, solid) = (
            r.take(n)?.to_vec(),
            r.take(n)?.to_vec(),
            r.take(n)?.to_vec(),
        );
        // every link must land on a node, so following them never panics
        let ok = |i: u32| i == END || (i as usize) < nodes.len();
        let linked = events.iter().all(|e| {
            e.x < w && e.y < h && !matches!(e.action, Action::Script { start } if !ok(start))
        }) && nodes.iter().all(|n| match n {
            Node::Say { next, .. } => ok(*next),
            Node::Ask { options, .. } => options.iter().all(|o| ok(o.then)),
        });
        (w > 0 && h > 0 && linked).then_some(Self {
            w,
            h,
            cells,
            objects,
            solid,
            events,
            nodes,
        })
    }

    /// Does the ground or an object block column `x`, row `y` (from the top)? Off the map does.
    pub fn solid_at(&self, x: i32, y: i32) -> bool {
        x < 0
            || y < 0
            || x >= self.w as i32
            || y >= self.h as i32
            || self.solid[y as usize * self.w as usize + x as usize] != 0
    }

    /// The kind at column `x`, row `y` (from the top); the nearest edge cell outside the map, like the shaders.
    pub fn kind(&self, x: i32, y: i32) -> u8 {
        let x = x.clamp(0, self.w as i32 - 1) as usize;
        let y = y.clamp(0, self.h as i32 - 1) as usize;
        self.cells[y * self.w as usize + x]
    }
}
