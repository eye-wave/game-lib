//! What both webview backends share: the page script, the yEnc transport and the reply parser.
//!
//! One hidden page stays open and decodes every file. The host sends `go(id, kind, payload)`; the page answers with
//! a string message `<id>:img:<w>:<h>:<rgba>`, `<id>:audio:<channels>:<rate>:<planar f32 LE>` or `<id>:error`
//! (`ready` once, when its script has run). Payloads are yEnc: byte+42, with NUL LF CR `=` `"` `<` `\` escaped as
//! `=` + (byte+64). Every JS char is one byte value 0..=255; each backend maps that to its transport's text
//! encoding (UTF-8 on GTK, UTF-16 on WebView2). Everything the page sends is re-checked here.
use crate::{Clip, Image, MAX, MAX_SAMPLES};
use alloc::vec::Vec;

/// Backend-specific: how the page sends a string to the host.
#[cfg(unix)]
pub const POST: &str = "<script>const post = m => webkit.messageHandlers.ipc.postMessage(m);";
#[cfg(windows)]
pub const POST: &str = "<script>const post = m => chrome.webview.postMessage(m);";

// 4096 below is `MAX`: an oversize image is refused before its pixels are shipped. Decode failures post `error`.
pub const PAGE: &str = r#"
const yenc = u => { let s = "", a = [];
  for (let k = 0; k < u.length; k++) {
    let c = (u[k] + 42) & 255;
    if (c == 0 || c == 10 || c == 13 || c == 34 || c == 60 || c == 61 || c == 92) { a.push(61); c = (c + 64) & 255; }
    a.push(c);
    if (a.length >= 32768) { s += String.fromCharCode.apply(null, a); a = []; }
  }
  return s + String.fromCharCode.apply(null, a); };
const un = s => { const o = new Uint8Array(s.length); let j = 0;
  for (let i = 0; i < s.length; i++) {
    let c = s.charCodeAt(i);
    if (c == 61) c = s.charCodeAt(++i) - 64;
    o[j++] = c - 42;
  }
  return o.slice(0, j); };
const img = (id, D) => {
  const i = new Image();
  i.onerror = () => post(id + ":error");
  i.onload = () => {
    const w = i.naturalWidth, h = i.naturalHeight;
    if (w > 4096 || h > 4096) return post(id + ":error");
    const c = new OffscreenCanvas(w, h), x = c.getContext("2d");
    x.drawImage(i, 0, 0);
    post(id + ":img:" + w + ":" + h + ":" + yenc(x.getImageData(0, 0, w, h).data));
  };
  i.src = URL.createObjectURL(new Blob([un(D)]));
};
// Offline context: no audio device. decodeAudioData resamples to the context rate (48 kHz).
const aud = (id, D) => new OfflineAudioContext(2, 1, 48000).decodeAudioData(un(D).buffer)
  .then(a => {
    const n = a.length, ch = a.numberOfChannels, f = new Float32Array(n * ch);
    for (let c = 0; c < ch; c++) f.set(a.getChannelData(c), c * n);
    post(id + ":audio:" + ch + ":" + a.sampleRate + ":" + yenc(new Uint8Array(f.buffer)));
  }).catch(() => post(id + ":error"));
"#;

/// Closes the page's script and, on WebView2 (where the host sends jobs as messages), listens for them.
#[cfg(unix)]
pub const END: &str = "post(\"ready\");</script>";
#[cfg(windows)]
pub const END: &str = r#"chrome.webview.addEventListener("message", e => {
  const m = e.data, i = m.indexOf(":"), j = m.indexOf(":", i + 1);
  go(+m.slice(0, i), m.slice(i + 1, j), m.slice(j + 1));
});
post("ready");</script>"#;

pub enum Decoded {
    Image(Image),
    Clip(Clip),
}

#[derive(PartialEq, Eq)]
pub enum State {
    Starting,
    Ready,
    /// The webview could not start (no WebView2 Runtime).
    #[cfg_attr(unix, allow(dead_code))]
    Dead,
}

/// Filled by the backend's callback, drained by `Loader::poll`. `None` = that job failed.
pub struct Inbox {
    pub state: State,
    pub done: Vec<(usize, Option<Decoded>)>,
}

impl Inbox {
    pub fn new() -> Self {
        Self {
            state: State::Starting,
            done: Vec::new(),
        }
    }

    /// One message from the page, as byte values 0..=255.
    pub fn push(&mut self, mut m: &[u8]) {
        let id = field(&mut m);
        if id == b"ready" {
            self.state = State::Ready;
        } else if let Some(id) = num(id) {
            self.done.push((id as usize, parse(m)));
        }
    }
}

fn field<'a>(s: &mut &'a [u8]) -> &'a [u8] {
    let i = s.iter().position(|&c| c == b':').unwrap_or(s.len());
    let f = &s[..i];
    *s = &s[(i + 1).min(s.len())..];
    f
}

fn num(f: &[u8]) -> Option<u32> {
    core::str::from_utf8(f).ok()?.parse().ok()
}

/// `<tag>:<a>:<b>:<payload>`; `None` for `error`, a bad number, or a payload that does not fit what it claims.
fn parse(mut s: &[u8]) -> Option<Decoded> {
    let tag = field(&mut s);
    let a = num(field(&mut s))?;
    let b = num(field(&mut s))?;
    let data = yenc_decode(s);
    match tag {
        b"img" => ((1..=MAX).contains(&a)
            && (1..=MAX).contains(&b)
            && data.len() == a as usize * b as usize * 4)
            .then_some(Decoded::Image(Image {
                w: a,
                h: b,
                rgba: data,
            })),
        b"audio" => {
            let f: Vec<f32> = data
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| f32::from_le_bytes(*c))
                .collect();
            let ch = a as usize;
            let n = f.len().checked_div(ch)?; // channels = 0 -> None
            if n == 0 || n > MAX_SAMPLES || f.len() != n * ch {
                return None;
            }
            // planar -> mono 16-bit: `audio` mixes everything down to mono anyway
            let pcm = (0..n)
                .map(|i| {
                    let m = (0..ch).map(|c| f[c * n + i]).sum::<f32>() / ch as f32;
                    (m.clamp(-1.0, 1.0) * 32767.0) as i16
                })
                .collect();
            Some(Decoded::Clip(Clip {
                rate: b,
                channels: 1,
                pcm,
            }))
        }
        _ => None,
    }
}

/// Bytes -> yEnc byte values (see the module docs for the escapes).
pub fn yenc_encode(d: &[u8], out: &mut Vec<u8>) {
    for &b in d {
        let mut c = b.wrapping_add(42);
        if matches!(c, 0 | 10 | 13 | 34 | 60 | 61 | 92) {
            out.push(b'=');
            c = c.wrapping_add(64);
        }
        out.push(c);
    }
}

pub fn yenc_decode(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut esc = false;
    for &c in s {
        let mut c = c;
        if esc {
            c = c.wrapping_sub(64);
            esc = false;
        } else if c == b'=' {
            esc = true;
            continue;
        }
        out.push(c.wrapping_sub(42));
    }
    out
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use alloc::vec;

    #[test]
    fn yenc_roundtrip() {
        let data: Vec<u8> = (0..1024u32).map(|i| i as u8).collect(); // every byte value, 4x
        for n in 0..data.len() {
            let mut e = Vec::new();
            yenc_encode(&data[..n], &mut e);
            assert!(
                ![0u8, b'\n', b'\r', b'"', b'<', b'\\']
                    .iter()
                    .any(|c| e.contains(c))
            );
            assert_eq!(yenc_decode(&e), &data[..n]);
        }
    }

    fn msg(head: &str, payload: &[u8]) -> Vec<u8> {
        let mut m = head.as_bytes().to_vec();
        yenc_encode(payload, &mut m);
        m
    }

    #[test]
    fn replies() {
        let mut ib = Inbox::new();
        ib.push(b"ready");
        assert!(ib.state == State::Ready && ib.done.is_empty());

        ib.push(&msg("3:img:2:1:", &[1, 2, 3, 4, 5, 6, 7, 8]));
        ib.push(&msg("4:img:2:1:", &[1, 2, 3])); // short pixels
        ib.push(&msg("5:img:5000:1:", &[])); // over MAX
        ib.push(b"6:error");
        ib.push(b"junk"); // no id: ignored
        let mut f = vec![];
        for s in [0.5f32, -0.5, 1.0, 1.0] {
            f.extend_from_slice(&s.to_le_bytes()); // planar: ch0 = [0.5, -0.5], ch1 = [1, 1]
        }
        ib.push(&msg("7:audio:2:48000:", &f));
        ib.push(&msg("8:audio:0:48000:", &f)); // zero channels
        ib.push(&msg("9:audio:3:48000:", &f)); // 4 samples do not split into 3 channels

        let got: Vec<_> = ib.done.iter().map(|(id, d)| (*id, d.is_some())).collect();
        assert_eq!(
            got,
            [
                (3, true),
                (4, false),
                (5, false),
                (6, false),
                (7, true),
                (8, false),
                (9, false)
            ]
        );
        match &ib.done[0].1 {
            Some(Decoded::Image(i)) => assert_eq!(
                (i.w, i.h, &i.rgba[..]),
                (2, 1, &[1, 2, 3, 4, 5, 6, 7, 8][..])
            ),
            _ => panic!(),
        }
        match &ib.done[4].1 {
            Some(Decoded::Clip(c)) => {
                assert_eq!((c.rate, c.channels), (48000, 1));
                assert_eq!(c.pcm, [(0.75 * 32767.0) as i16, (0.25 * 32767.0) as i16]);
            }
            _ => panic!(),
        }
    }
}
