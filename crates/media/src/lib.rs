//! Image and sound loading through one spawned `ffmpeg` process per file, so no decoder lives in the binary and any
//! format ffmpeg knows works. ffmpeg must be on PATH (`FFMPEG_MISSING` otherwise). Errors are `die`-ready codes.
//!
//! `Loader` is the asset system: queue files, call `poll` once per frame, read `progress`/`status`, `take` what is
//! ready. An asset is `Pending` (queued, waiting for a free slot: at most `MAX_RUNNING` ffmpegs at a time),
//! `Loading` (ffmpeg running, its output being collected), then `Ready` or `Failed(code)`. Nothing blocks except
//! `Loader::wait` and `load_image`, which are for tools and tests.
//!
//! Images come out as 8-bit RGBA (straight alpha, top row first, ffmpeg's PAM output), sounds as mono 16-bit PCM at
//! 48 kHz (the rate `audio` plays at, and it mixes everything down to mono anyway).
#![no_std]
extern crate alloc;

use alloc::vec::Vec;
use core::ffi::CStr;
use errors::assets::*;
use platform::{Child, Read};

/// Larger images are rejected as soon as the header is read, before the pixels are collected.
const MAX: u32 = 4096;
/// Clips are decoded whole: 32M samples is about 11 minutes at 48 kHz mono. Stream longer music instead.
const MAX_SAMPLES: usize = 1 << 25;
/// ffmpeg processes alive at once; the rest of the queue stays `Pending`.
const MAX_RUNNING: usize = 4;
/// Bytes read from one ffmpeg per `poll` at most, so a big asset cannot stall a frame.
const READ_PER_POLL: usize = 1 << 20;
const RATE: u32 = 48_000;

pub struct Image {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

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
pub struct Grid {
    pub cw: u32,
    pub ch: u32,
    pub cols: u32,
}

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
    /// `buf` is everything ffmpeg has written so far; `want` is the size it must reach (0 = not known yet).
    Loading {
        child: Child,
        buf: Vec<u8>,
        want: usize,
    },
    Image(Image),
    Clip(Clip),
    Taken,
    Failed(&'static CStr),
}

struct Job {
    path: Vec<u8>, // NUL-terminated
    sound: bool,
    slot: Slot,
}

#[derive(Default)]
pub struct Loader(Vec<Job>);

impl Loader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue `name` from the assets directory as an image (any format ffmpeg reads; only the first frame).
    pub fn image(&mut self, name: &str) -> Id {
        self.queue(name, false)
    }

    /// Queue `name` from the assets directory as a sound (any format ffmpeg reads; a video's audio track works too).
    pub fn sound(&mut self, name: &str) -> Id {
        self.queue(name, true)
    }

    fn queue(&mut self, name: &str, sound: bool) -> Id {
        self.0.push(Job {
            path: platform::asset_path(name),
            sound,
            slot: Slot::Pending,
        });
        Id(self.0.len() - 1)
    }

    /// Call once per frame: starts pending assets while there is room, and collects what the running ones wrote.
    pub fn poll(&mut self) {
        let mut running = self
            .0
            .iter()
            .filter(|j| matches!(j.slot, Slot::Loading { .. }))
            .count();
        for job in &mut self.0 {
            if matches!(job.slot, Slot::Pending) && running < MAX_RUNNING {
                job.start();
                running += matches!(job.slot, Slot::Loading { .. }) as usize;
            }
            if matches!(job.slot, Slot::Loading { .. }) {
                job.pump();
            }
        }
    }

    pub fn status(&self, id: Id) -> Status {
        match self.0[id.0].slot {
            Slot::Pending => Status::Pending,
            Slot::Loading { .. } => Status::Loading,
            Slot::Failed(e) => Status::Failed(e),
            _ => Status::Ready,
        }
    }

    /// Nothing is `Pending` or `Loading` any more (everything is `Ready` or `Failed`). True for an empty loader.
    pub fn done(&self) -> bool {
        self.0
            .iter()
            .all(|j| !matches!(j.slot, Slot::Pending | Slot::Loading { .. }))
    }

    /// First failure, if any asset failed.
    pub fn failed(&self) -> Option<&'static CStr> {
        self.0.iter().find_map(|j| match j.slot {
            Slot::Failed(e) => Some(e),
            _ => None,
        })
    }

    /// 0..=1 over all queued assets: finished ones count fully, a running image counts by the pixels received so
    /// far (a sound only counts once it is done: its length is not known up front). 1 for an empty loader.
    pub fn progress(&self) -> f32 {
        if self.0.is_empty() {
            return 1.0;
        }
        let sum: f32 = self
            .0
            .iter()
            .map(|j| match &j.slot {
                Slot::Pending => 0.0,
                Slot::Loading { buf, want, .. } if *want > 0 => {
                    buf.len() as f32 / *want as f32 * 0.99
                }
                Slot::Loading { .. } => 0.0,
                _ => 1.0,
            })
            .sum();
        sum / self.0.len() as f32
    }

    /// The image, once it is `Ready`; it can be taken once. `None` for anything else, including a sound's id.
    pub fn take_image(&mut self, id: Id) -> Option<Image> {
        match core::mem::replace(&mut self.0[id.0].slot, Slot::Taken) {
            Slot::Image(i) => Some(i),
            other => {
                self.0[id.0].slot = other;
                None
            }
        }
    }

    /// The sound, once it is `Ready`; it can be taken once. `None` for anything else, including an image's id.
    pub fn take_clip(&mut self, id: Id) -> Option<Clip> {
        match core::mem::replace(&mut self.0[id.0].slot, Slot::Taken) {
            Slot::Clip(c) => Some(c),
            other => {
                self.0[id.0].slot = other;
                None
            }
        }
    }

    /// Blocks until everything is done. For tools and tests; a game polls from its frame loop instead.
    pub fn wait(&mut self) {
        while !self.done() {
            self.poll();
            platform::sleep_ms(1);
        }
    }
}

/// Blocking one-off: queue, wait, take. For programs with nothing to show while they load.
pub fn load_image(name: &str) -> Result<Image, &'static CStr> {
    let mut l = Loader::new();
    let id = l.image(name);
    l.wait();
    l.take_image(id)
        .ok_or(l.failed().unwrap_or(IMAGE_DECODE_FAILED))
}

impl Job {
    fn start(&mut self) {
        let Ok(path) = CStr::from_bytes_with_nul(&self.path) else {
            self.slot = Slot::Failed(READ_FILE_FAILED); // a NUL inside the name
            return;
        };
        let head = [c"ffmpeg", c"-nostdin", c"-v", c"error", c"-i", path];
        let tail: &[&CStr] = if self.sound {
            &[
                c"-vn", c"-ac", c"1", c"-ar", c"48000", c"-f", c"s16le", c"-",
            ]
        } else {
            &[
                c"-frames:v",
                c"1",
                c"-an",
                c"-pix_fmt",
                c"rgba",
                c"-c:v",
                c"pam",
                c"-f",
                c"image2pipe",
                c"-",
            ]
        };
        let args: Vec<&CStr> = head.iter().chain(tail).copied().collect();
        self.slot = match platform::spawn(&args) {
            Some(child) => Slot::Loading {
                child,
                buf: Vec::new(),
                want: 0,
            },
            None => Slot::Failed(FFMPEG_MISSING),
        };
    }

    /// Read what ffmpeg has written (without blocking); finish the asset once its output ends.
    fn pump(&mut self) {
        let Slot::Loading { child, buf, want } = &mut self.slot else {
            return;
        };
        let mut chunk = [0u8; 1 << 14];
        let mut eof = false;
        for _ in 0..READ_PER_POLL / chunk.len() {
            match child.read(&mut chunk) {
                Read::Data(n) => buf.extend_from_slice(&chunk[..n]),
                Read::Empty => break,
                Read::Eof => {
                    eof = true;
                    break;
                }
            }
        }
        let bad = if self.sound {
            AUDIO_DECODE_FAILED
        } else {
            IMAGE_DECODE_FAILED
        };
        if !self.sound && *want == 0 {
            match pam_header(buf) {
                Ok(Some((hdr, w, h))) => *want = hdr + w as usize * h as usize * 4,
                Ok(None) if !eof && buf.len() < 256 => {} // header not complete yet
                _ => return self.slot = Slot::Failed(bad), // garbage, oversize, or no header at all
            }
        }
        if (self.sound && buf.len() / 2 > MAX_SAMPLES) || (*want > 0 && buf.len() > *want) {
            return self.slot = Slot::Failed(bad);
        }
        if !eof {
            return;
        }
        let ok = child.wait();
        let buf = core::mem::take(buf);
        self.slot = match (ok, self.sound) {
            (true, true) if buf.len() >= 2 => Slot::Clip(Clip {
                rate: RATE,
                channels: 1,
                pcm: buf
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| i16::from_le_bytes(*b))
                    .collect(),
            }),
            (true, false) if buf.len() == *want => {
                let (hdr, w, h) = pam_header(&buf).ok().flatten().unwrap();
                let mut rgba = buf;
                drop(rgba.drain(..hdr));
                Slot::Image(Image { w, h, rgba })
            }
            _ => Slot::Failed(bad),
        };
    }
}

/// `Ok(Some((header bytes, w, h)))` once the whole PAM header is in `b`, `Ok(None)` while it is incomplete, `Err`
/// for anything that is not RGBA PAM within `MAX`.
fn pam_header(b: &[u8]) -> Result<Option<(usize, u32, u32)>, ()> {
    const END: &[u8] = b"ENDHDR\n";
    let Some(end) = b.windows(END.len()).position(|w| w == END) else {
        return Ok(None);
    };
    let hdr = end + END.len();
    let text = core::str::from_utf8(&b[..end]).map_err(|_| ())?;
    let mut lines = text.lines();
    if lines.next() != Some("P7") {
        return Err(());
    }
    let (mut w, mut h, mut depth) = (0, 0, 0);
    for l in lines {
        let (key, v) = l.split_once(' ').ok_or(())?;
        let v = v.parse::<u32>().unwrap_or(0);
        match key {
            "WIDTH" => w = v,
            "HEIGHT" => h = v,
            "DEPTH" => depth = v,
            _ => {}
        }
    }
    if depth == 4 && (1..=MAX).contains(&w) && (1..=MAX).contains(&h) {
        Ok(Some((hdr, w, h)))
    } else {
        Err(())
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

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
        let missing = l.image("nope.png");
        let not_an_image = l.image("blip.wav");
        assert!(l.status(missing) == Status::Pending && l.progress() == 0.0 && !l.done());

        l.poll(); // 4 slots: the first four start, the rest wait their turn
        assert!(l.status(imgs[0]) == Status::Loading && l.status(snds[1]) == Status::Pending);
        assert!(l.status(not_an_image) == Status::Pending);

        l.wait();
        assert!(l.done() && l.progress() == 1.0);
        assert!(l.status(missing) == Status::Failed(IMAGE_DECODE_FAILED));
        assert!(l.status(not_an_image) == Status::Failed(IMAGE_DECODE_FAILED));

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

    #[test]
    fn pam_header_cases() {
        let ok = b"P7\nWIDTH 3\nHEIGHT 2\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n";
        assert!(pam_header(ok) == Ok(Some((ok.len(), 3, 2))));
        assert!(pam_header(&ok[..20]) == Ok(None));
        assert!(pam_header(b"P7\nWIDTH 5000\nHEIGHT 2\nDEPTH 4\nENDHDR\n").is_err());
        assert!(pam_header(b"P7\nWIDTH 3\nHEIGHT 2\nDEPTH 3\nENDHDR\n").is_err());
        assert!(pam_header(b"nope\nENDHDR\n").is_err());
    }
}
