//! Image and sound loading through one hidden webview (WebKitGTK on Linux, WebView2 on Windows), so no decoder lives
//! in the binary and any format the system browser engine knows works. Linux needs a display, libgtk-3 and
//! libwebkit2gtk-4.1; Windows needs the WebView2 Runtime and `WebView2Loader.dll` next to the exe (`WEBVIEW_UNAVAILABLE`
//! otherwise). Errors are `die`-ready codes.
//!
//! `Loader` is the asset system: queue files, call `poll` once per frame, read `progress`/`status`, `take` what is
//! ready. An asset is `Pending` (queued, waiting for the webview to start or for a free slot: at most `MAX_RUNNING`
//! decodes at a time), `Loading` (the webview is decoding it), then `Ready` or `Failed(code)`. Nothing blocks except
//! `Loader::wait` and `load_image`, which are for tools and tests. Call everything from one thread that owns the
//! GTK main context / has a message loop (your main thread). The webview lives only while something is queued.
//!
//! Text: `Loader::font` has the webview draw a TTF/OTF/WOFF into a glyph grid image; a pixel font is just such an image
//! already. `Font` (see `font.rs`) lays text out from the grid in plain Rust.
//!
//! Images come out as 8-bit RGBA (straight alpha, top row first), sounds as mono 16-bit PCM at 48 kHz (the rate
//! `audio` plays at, and it mixes everything down to mono anyway).
#![no_std]
extern crate alloc;

#[cfg(feature = "image")]
mod font;
#[cfg_attr(unix, path = "gtk.rs")]
#[cfg_attr(windows, path = "webview2.rs")]
mod sys;
mod web;
#[cfg(feature = "image")]
pub use font::{ASCII, COLS, Font, alpha_atlas};

use alloc::{string::String, vec::Vec};
use core::ffi::CStr;
use errors::assets::*;
use sys::View;
use web::{Decoded, State, escape};

/// Larger images are rejected before their pixels are sent over.
#[cfg(feature = "image")]
const MAX: u32 = 4096;
/// Clips are decoded whole: 32M samples is about 11 minutes at 48 kHz mono. Stream longer music instead.
#[cfg(feature = "audio")]
const MAX_SAMPLES: usize = 1 << 25;
/// Files being decoded at once; the rest of the queue stays `Pending`.
const MAX_RUNNING: usize = 4;

#[cfg(feature = "image")]
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

#[cfg(feature = "image")]
impl Image {
    pub fn px(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.w + x) * 4) as usize;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }
}

/// Equal `cw` x `ch` cells numbered left to right, top to bottom.
#[cfg(feature = "image")]
pub struct Grid {
    pub cw: u32,
    pub ch: u32,
    pub cols: u32,
}

#[cfg(feature = "image")]
impl Grid {
    pub fn new(img: &Image, cw: u32, ch: u32) -> Self {
        Self {
            cw,
            ch,
            cols: img.w / cw,
        }
    }
    /// Pixel (x, y) of cell `i`, with (0, 0) at the cell's top-left.
    pub fn px(&self, img: &Image, i: u32, x: u32, y: u32) -> [u8; 4] {
        img.px(i % self.cols * self.cw + x, i / self.cols * self.ch + y)
    }
}

/// Interleaved 16-bit PCM: frame `i` is `pcm[i * channels..][..channels]`.
#[cfg(feature = "audio")]
pub struct Clip {
    pub rate: u32,
    pub channels: u32,
    pub pcm: Vec<i16>,
}

/// Where an asset is in its life. `Ready` stays `Ready` after `take`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pending,
    Loading,
    Ready,
    Failed(&'static CStr),
}

/// Ticket for a queued asset. Use it with the `Loader` that gave it out.
#[derive(Clone, Copy)]
pub struct Id(usize);

enum Slot {
    Pending,
    Loading,
    #[cfg(feature = "image")]
    Image(Image),
    #[cfg(feature = "audio")]
    Clip(Clip),
    Taken,
    Failed(&'static CStr),
}

struct Job {
    name: String,
    kind: char,   // `i`mage, `a`udio or `f`ont: what the page's `go` takes
    stage: u8,    // how far the page got with it (web.rs `stages`), only for the timeout error
    head: String, // `f` only: `<px>\n<chars>\n`, sent before the file
    slot: Slot,
}

#[derive(Default)]
pub struct Loader {
    jobs: Vec<Job>,
    view: Option<View>,
}

impl Loader {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(feature = "image")]
    /// Queue `name` from the assets directory as an image (any format the webview reads; only the first frame).
    pub fn image(&mut self, name: &str) -> Id {
        self.queue(name, 'i')
    }

    #[cfg(feature = "audio")]
    /// Queue `name` from the assets directory as a sound (any format the webview reads).
    pub fn sound(&mut self, name: &str) -> Id {
        self.queue(name, 'a')
    }

    #[cfg(feature = "font")]
    /// Queue the font file `name` (TTF, OTF, WOFF) to be drawn by the webview, `px` tall, into a glyph grid image for
    /// `Font::new(&img, chars, 0)`: take it with `take_image`. `chars` must not contain a newline.
    pub fn font(&mut self, name: &str, chars: &str, px: u32) -> Id {
        let id = self.queue(name, 'f');
        self.jobs[id.0].head = alloc::format!("{px}\n{chars}\n");
        id
    }

    fn queue(&mut self, name: &str, kind: char) -> Id {
        self.jobs.push(Job {
            name: name.into(),
            kind,
            head: String::new(),
            stage: 0,
            slot: Slot::Pending,
        });
        Id(self.jobs.len() - 1)
    }

    /// Call once per frame: runs the webview's pending events, files away what it finished, and starts pending
    /// assets while there is room. Nothing blocks: the page fetches the files from the host as it goes.
    pub fn poll(&mut self) {
        if self.done() {
            return;
        }
        if self.view.is_none() {
            self.view = View::new();
        }
        let Some(view) = &mut self.view else {
            return self.fail_all(WEBVIEW_UNAVAILABLE);
        };
        view.pump();
        for (id, d) in core::mem::take(&mut view.inbox.done) {
            // an id the page made up, or a second answer for one job, is ignored
            if let Some(job) = self
                .jobs
                .get_mut(id)
                .filter(|j| matches!(j.slot, Slot::Loading))
            {
                job.slot = match (d, job.kind) {
                    #[cfg(feature = "image")]
                    (Some(Decoded::Image(i)), 'i' | 'f') => Slot::Image(i),
                    #[cfg(feature = "audio")]
                    (Some(Decoded::Clip(c)), 'a') => Slot::Clip(c),
                    #[cfg(feature = "audio")]
                    (_, 'a') => Slot::Failed(AUDIO_DECODE_FAILED),
                    _ => Slot::Failed(IMAGE_DECODE_FAILED),
                };
            }
        }
        for (id, n) in core::mem::take(&mut view.inbox.stages) {
            if let Some(job) = self.jobs.get_mut(id) {
                job.stage = job.stage.max(n);
            }
        }
        for id in core::mem::take(&mut view.inbox.missing) {
            if let Some(job) = self
                .jobs
                .get_mut(id)
                .filter(|j| matches!(j.slot, Slot::Loading))
            {
                job.slot = Slot::Failed(READ_FILE_FAILED);
            }
        }
        match view.inbox.state {
            State::Dead => return self.fail_all(WEBVIEW_UNAVAILABLE),
            State::Starting => return,
            State::Ready => {}
        }
        let mut running = self
            .jobs
            .iter()
            .filter(|j| matches!(j.slot, Slot::Loading))
            .count();
        for (id, job) in self.jobs.iter_mut().enumerate() {
            if running >= MAX_RUNNING {
                break;
            }
            if !matches!(job.slot, Slot::Pending) {
                continue;
            }
            view.run(id, job.kind, &escape(&job.name), &escape(&job.head)); // the page fetches the file itself
            running += 1;
            job.slot = Slot::Loading;
        }
        if self.done() {
            self.view = None; // frees the browser process; the next queued asset starts a new one
        }
    }

    fn fail_all(&mut self, e: &'static CStr) {
        for j in &mut self.jobs {
            if matches!(j.slot, Slot::Pending | Slot::Loading) {
                j.slot = Slot::Failed(e);
            }
        }
        self.view = None;
    }

    pub fn status(&self, id: Id) -> Status {
        match self.jobs[id.0].slot {
            Slot::Pending => Status::Pending,
            Slot::Loading => Status::Loading,
            Slot::Failed(e) => Status::Failed(e),
            _ => Status::Ready,
        }
    }

    /// Nothing is `Pending` or `Loading` any more (everything is `Ready` or `Failed`). True for an empty loader.
    pub fn done(&self) -> bool {
        self.jobs
            .iter()
            .all(|j| !matches!(j.slot, Slot::Pending | Slot::Loading))
    }

    /// First failure, if any asset failed.
    pub fn failed(&self) -> Option<&'static CStr> {
        self.jobs.iter().find_map(|j| match j.slot {
            Slot::Failed(e) => Some(e),
            _ => None,
        })
    }

    /// 0..=1 over all queued assets: the share that is finished (an asset is all or nothing: the webview does not
    /// report how far along a decode is). 1 for an empty loader.
    pub fn progress(&self) -> f32 {
        if self.jobs.is_empty() {
            return 1.0;
        }
        let ready = self
            .jobs
            .iter()
            .filter(|j| !matches!(j.slot, Slot::Pending | Slot::Loading))
            .count();
        ready as f32 / self.jobs.len() as f32
    }

    /// The image, once it is `Ready`; it can be taken once. `None` for anything else, including a sound's id.
    #[cfg(feature = "image")]
    pub fn take_image(&mut self, id: Id) -> Option<Image> {
        match core::mem::replace(&mut self.jobs[id.0].slot, Slot::Taken) {
            Slot::Image(i) => Some(i),
            other => {
                self.jobs[id.0].slot = other;
                None
            }
        }
    }

    /// The sound, once it is `Ready`; it can be taken once. `None` for anything else, including an image's id.
    #[cfg(feature = "audio")]
    pub fn take_clip(&mut self, id: Id) -> Option<Clip> {
        match core::mem::replace(&mut self.jobs[id.0].slot, Slot::Taken) {
            Slot::Clip(c) => Some(c),
            other => {
                self.jobs[id.0].slot = other;
                None
            }
        }
    }

    /// Blocks until everything is done. After 15 s it gives up and fails what is left: `WEBVIEW_UNAVAILABLE` if the
    /// webview never came up, else a `WEBVIEW_TIMEOUT_*` that says how far the page got. For tools and tests; a game
    /// polls from its frame loop instead.
    pub fn wait(&mut self) {
        let t0 = platform::now();
        while !self.done() {
            if platform::now() - t0 > 15.0 {
                let up = self
                    .view
                    .as_ref()
                    .is_some_and(|v| v.inbox.state == State::Ready);
                for j in &mut self.jobs {
                    if matches!(j.slot, Slot::Pending | Slot::Loading) {
                        j.slot = Slot::Failed(match (up, j.stage) {
                            (false, _) => WEBVIEW_UNAVAILABLE,
                            (_, 0) => WEBVIEW_TIMEOUT_NO_JOB,
                            (_, 1) => WEBVIEW_TIMEOUT_NO_FILE,
                            (_, 2) => WEBVIEW_TIMEOUT_NO_DECODE,
                            _ => WEBVIEW_TIMEOUT_NO_REPLY,
                        });
                    }
                }
                self.view = None;
                return;
            }
            self.poll();
            platform::sleep_ms(1);
        }
    }
}

#[cfg(feature = "image")]
/// Blocking one-off: queue, wait, take. For programs with nothing to show while they load.
pub fn load_image(name: &str) -> Result<Image, &'static CStr> {
    let mut l = Loader::new();
    let id = l.image(name);
    l.wait();
    l.take_image(id)
        .ok_or(l.failed().unwrap_or(IMAGE_DECODE_FAILED))
}

#[cfg(all(test, feature = "image", feature = "audio", feature = "font"))]
mod tests {
    extern crate std;
    use super::*;

    /// Needs the assets and a display (xvfb-run on a headless Linux box) or the WebView2 Runtime.
    #[test]
    fn loader_lifecycle() {
        let mut l = Loader::new();
        assert!(l.done() && l.progress() == 1.0);
        let imgs = [
            l.image("walk.png"),
            l.image("walk.jpg"),
            l.image("walk.webp"),
        ];
        let snds = [
            l.sound("blip.wav"),
            l.sound("blip.ogg"),
            l.sound("blip.mp3"),
        ];
        let grid = l.font("ComicMono.ttf", ASCII, 32); // drawn by the page: 16 cells a row, 6 rows
        let missing = l.image("nope.png");
        let not_an_image = l.image("blip.wav");
        assert!(l.status(missing) == Status::Pending && l.progress() == 0.0 && !l.done());

        while l.status(imgs[0]) == Status::Pending {
            l.poll(); // the webview takes a moment to start
            platform::sleep_ms(1);
        }
        // 4 slots: the first four started in the same poll, the rest wait their turn
        assert!(l.status(snds[0]) == Status::Loading && l.status(snds[1]) == Status::Pending);
        assert!(l.status(not_an_image) == Status::Pending);

        l.wait();
        assert!(l.done() && l.progress() == 1.0 && l.view.is_none());
        assert!(l.status(missing) == Status::Failed(READ_FILE_FAILED));
        assert!(l.status(not_an_image) == Status::Failed(IMAGE_DECODE_FAILED));

        let g = l.take_image(grid).unwrap();
        assert!(g.w % COLS == 0 && g.h % 6 == 0 && g.rgba.chunks(4).any(|p| p[3] > 200));
        let png = l.take_image(imgs[0]).unwrap();
        assert_eq!((png.w, png.h), (168, 24));
        let g = Grid::new(&png, 24, 24);
        assert_eq!(g.cols, 7);
        let jpg = l.take_image(imgs[1]).unwrap();
        assert_eq!((jpg.w, jpg.h), (168, 24));
        assert!(jpg.rgba.chunks(4).all(|p| p[3] == 255));
        let webp = l.take_image(imgs[2]).unwrap(); // lossless: same pixels as the PNG
        for (a, b) in webp.rgba.chunks(4).zip(png.rgba.chunks(4)) {
            assert!(a[3] == b[3] && (b[3] == 0 || a == b));
        }
        for id in snds {
            let c = l.take_clip(id).unwrap();
            assert_eq!((c.rate, c.channels), (48_000, 1));
            assert!((8000..12000).contains(&c.pcm.len())); // 0.2 s = 9600; lossy codecs add or trim padding
            assert!(c.pcm.iter().any(|s| s.abs() > 1000));
        }
        assert!(l.take_image(imgs[0]).is_none() && l.status(imgs[0]) == Status::Ready); // taken once
    }
}
