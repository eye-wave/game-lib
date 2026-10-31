//! Image and audio decoding by the OS, so no decoder lives in the binary. Errors are `die`-ready codes from `errors`.
//! The caller names the format (`png`, `jpeg`, `webp`, `wav`, `ogg`, `mp3`), so the linker drops the ones a game never calls.
//! Images come out as 8-bit RGBA (straight alpha, top row first), sounds as interleaved 16-bit PCM at the file's own rate.
//! Linux dlopens libpng, libturbojpeg, libwebp, libvorbisfile and libmpg123; each is only needed for its own format.
//! Windows uses GDI+ for PNG/JPEG, WIC for WebP (needs Microsoft's "WebP Image Extensions") and Media Foundation for
//! MP3/Ogg (Ogg needs the "Web Media Extensions"). WAV is parsed here: smaller than any binding.
#![no_std]
extern crate alloc;

use alloc::vec::Vec;
use core::ffi::CStr;
use errors::assets::AUDIO_DECODE_FAILED;

#[cfg_attr(unix, path = "linux.rs")]
#[cfg_attr(windows, path = "windows.rs")]
mod sys;

/// Larger images are rejected before any allocation.
const MAX: u32 = 4096;
/// Clips are decoded whole: 32M samples is about 6 minutes of 48 kHz stereo. Stream longer music instead.
const MAX_SAMPLES: usize = 1 << 25;

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

pub use sys::{jpeg, mp3, ogg, png, webp};

/// Interleaved 16-bit PCM: frame `i` is `pcm[i * channels..][..channels]`.
pub struct Clip {
    pub rate: u32,
    pub channels: u32,
    pub pcm: Vec<i16>,
}

/// Appends little-endian s16 bytes; false once the clip would pass `MAX_SAMPLES`.
fn push(pcm: &mut Vec<i16>, bytes: &[u8]) -> bool {
    pcm.extend(
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes([b[0], b[1]])),
    );
    pcm.len() <= MAX_SAMPLES
}

/// RIFF/WAVE with 16-bit PCM. Add 8/24-bit and float when an asset needs them.
pub fn wav(file: &[u8]) -> Result<Clip, &'static CStr> {
    let bad = AUDIO_DECODE_FAILED;
    let [
        b'R',
        b'I',
        b'F',
        b'F',
        _,
        _,
        _,
        _,
        b'W',
        b'A',
        b'V',
        b'E',
        rest @ ..,
    ] = file
    else {
        return Err(bad);
    };
    let mut p = rest; // `mut p @ ..` would bind the unsized [u8] by value, not the &[u8]
    let (mut rate, mut channels) = (0, 0);
    while p.len() >= 8 {
        let n = u32::from_le_bytes([p[4], p[5], p[6], p[7]]) as usize;
        let Some(body) = p.get(8..8 + n) else {
            return Err(bad); // truncated
        };
        match &p[..4] {
            b"fmt " if n >= 16 => {
                let u16le = |o: usize| u16::from_le_bytes([body[o], body[o + 1]]);
                // format tag 1 = PCM, 0xFFFE = extensible (the sub-format is not checked)
                if !matches!(u16le(0), 1 | 0xFFFE) || u16le(14) != 16 || u16le(2) == 0 {
                    return Err(bad);
                }
                channels = u16le(2) as u32;
                rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
            }
            b"data" => {
                let mut pcm = Vec::with_capacity(body.len() / 2);
                if rate == 0 || body.is_empty() || !push(&mut pcm, body) {
                    return Err(bad);
                }
                return Ok(Clip {
                    rate,
                    channels,
                    pcm,
                });
            }
            _ => {}
        }
        p = p.get(8 + n + (n & 1)..).unwrap_or(&[]); // chunks are padded to even sizes
    }
    Err(bad)
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use errors::assets::{AUDIO_LIBRARY_MISSING, IMAGE_LIBRARY_MISSING};

    type Img = fn(&[u8]) -> Result<Image, &'static CStr>;
    type Snd = fn(&[u8]) -> Result<Clip, &'static CStr>;

    const PNG: &[u8] = include_bytes!("../../../assets/walk.png");
    const JPG: &[u8] = include_bytes!("../../../assets/walk.jpg");
    const WEBP: &[u8] = include_bytes!("../../../assets/walk.webp");
    const WAV: &[u8] = include_bytes!("../../../assets/blip.wav"); // 0.2 s sine, 22.05 kHz mono
    const OGG: &[u8] = include_bytes!("../../../assets/blip.ogg"); // 0.2 s sine, 44.1 kHz stereo
    const MP3: &[u8] = include_bytes!("../../../assets/blip.mp3"); // same

    #[test]
    fn walk_sheet() {
        let img = png(PNG).unwrap();
        assert_eq!((img.w, img.h), (168, 24));
        let g = Grid::new(&img, 24, 24);
        assert_eq!(g.cols, 7);
        for i in 0..7 {
            let opaque = (0..24 * 24)
                .filter(|&k| g.px(&img, i, k % 24, k / 24)[3] != 0)
                .count();
            assert!(opaque > 20, "frame {i} is empty");
        }
    }

    #[test]
    fn jpeg_and_webp() {
        let png = png(PNG).unwrap();
        let jpg = jpeg(JPG).unwrap();
        assert_eq!((jpg.w, jpg.h), (168, 24));
        assert!(jpg.rgba.chunks(4).all(|p| p[3] == 255));
        // the codec may legitimately be absent (no libwebp / no Store extension); if present it must agree with the PNG
        match webp(WEBP) {
            Ok(w) => {
                assert_eq!((w.w, w.h), (png.w, png.h));
                for (a, b) in w.rgba.chunks(4).zip(png.rgba.chunks(4)) {
                    assert!(a[3] == b[3] && (b[3] == 0 || a == b)); // lossless; colour under alpha 0 is don't-care
                }
            }
            Err(e) => assert!(e == IMAGE_LIBRARY_MISSING),
        }
    }

    #[test]
    fn sounds() {
        let w = wav(WAV).unwrap();
        assert_eq!((w.rate, w.channels, w.pcm.len()), (22050, 1, 4410));
        assert!(w.pcm.iter().any(|s| s.abs() > 1000));
        for (dec, file) in [(ogg as Snd, OGG), (mp3, MP3)] {
            // lossy codecs add or trim a little padding; a missing codec is fine
            match dec(file) {
                Ok(c) => {
                    assert_eq!((c.rate, c.channels), (44100, 2));
                    assert!((8000..10500).contains(&(c.pcm.len() / 2)));
                    assert!(c.pcm.iter().any(|s| s.abs() > 1000));
                }
                Err(e) => assert!(e == AUDIO_LIBRARY_MISSING),
            }
        }
    }

    #[test]
    fn garbage_is_err() {
        for (dec, file) in [(png as Img, PNG), (jpeg, JPG), (webp, WEBP)] {
            assert!(dec(b"").is_err());
            assert!(dec(&file[..4]).is_err());
            assert!(dec(&file[..file.len() / 2]).is_err()); // truncated
        }
        for (dec, file) in [(wav as Snd, WAV), (ogg, OGG), (mp3, MP3)] {
            assert!(dec(b"").is_err());
            assert!(dec(&file[..4]).is_err());
        }
        assert!(wav(&WAV[..WAV.len() / 2]).is_err()); // an Ogg or MP3 stream just ends early, which is not an error
    }
}
