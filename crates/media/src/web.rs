//! What both webview backends share: the page script, the request paths and the reply parser.
//!
//! One hidden page stays open and decodes every file. No bytes travel as text:
//! - Host -> page: a short message `go(id, kind, name, head)` (name and head percent-escaped). The page `fetch`es
//!   `BASE + "a/" + name`, which the backend answers from the assets directory (404 = no such file).
//! - Page -> host: it `POST`s the raw result bytes to `BASE + "r/<id>/<tag>/<a>/<b>"` (`img`: w h, rgba;
//!   `audio`: channels rate, planar f32 LE), which the backend hands to `Inbox::push_reply`.
//! - Only `ready`, `<id>:error` and `<id>:nofile` are plain messages (`post`).
//! `BASE` is a custom scheme on GTK and a made-up https host on WebView2 (the backend intercepts the requests,
//! nothing goes to the network). Everything the page sends is re-checked here.
#[cfg(feature = "audio")]
use crate::{Clip, MAX_SAMPLES};
#[cfg(feature = "image")]
use crate::{Image, MAX};
use alloc::{string::String, vec::Vec};

/// Where the page's requests go; the backend strips it to get the `a/<name>` or `r/...` path.
#[cfg(unix)]
pub const BASE: &str = "app://m/";
#[cfg(windows)]
pub const BASE: &str = "https://media.localhost/";

/// Backend-specific: how the page sends a string to the host.
#[cfg(unix)]
pub const POST: &str = "<script>const post = m => webkit.messageHandlers.ipc.postMessage(m);";
#[cfg(windows)]
pub const POST: &str = "<script>const post = m => chrome.webview.postMessage(m);";

// 4096 below is `MAX`: an oversize image is refused before its pixels are shipped. Failures post `error`, a missing
// file `nofile`. The POST says text/plain so the browser sends it without a CORS preflight.
const COMMON: &str = r#"
const T = {}; // job kind -> decoder (id, name, head), filled by the parts below
const get = async (id, name) => { const r = await fetch(B + "a/" + name); if (!r.ok) throw "nofile"; const b = await r.blob(); post(id + ":s2"); return b; };
const reply = (id, tag, a, b, bytes) => (post(id + ":s3"), fetch(B + "r/" + id + "/" + tag + "/" + a + "/" + b,
  { method: "POST", body: bytes, headers: { "Content-Type": "text/plain" } }).catch(() => post(id + ":error")));
const fail = (id, e) => post(id + (e == "nofile" ? ":nofile" : ":error"));
"#;

#[cfg(feature = "image")]
const IMG: &str = r#"
T.i = async (id, name) => { try {
  const i = new Image(), u = URL.createObjectURL(await get(id, name));
  await new Promise((ok, no) => { i.onload = ok; i.onerror = no; i.src = u; });
  const w = i.naturalWidth, h = i.naturalHeight;
  if (w > 4096 || h > 4096) throw 0;
  const x = new OffscreenCanvas(w, h).getContext("2d");
  x.drawImage(i, 0, 0);
  reply(id, "img", w, h, x.getImageData(0, 0, w, h).data);
} catch (e) { fail(id, e); } };
"#;

#[cfg(feature = "audio")]
const AUD: &str = r#"
// Offline context: no audio device. decodeAudioData resamples to the context rate (48 kHz).
T.a = async (id, name) => { try {
  const a = await new OfflineAudioContext(2, 1, 48000).decodeAudioData(await (await get(id, name)).arrayBuffer());
  const n = a.length, ch = a.numberOfChannels, f = new Float32Array(n * ch);
  for (let c = 0; c < ch; c++) f.set(a.getChannelData(c), c * n);
  reply(id, "audio", ch, a.sampleRate, new Uint8Array(f.buffer));
} catch (e) { fail(id, e); } };
"#;

#[cfg(feature = "font")]
const FNT: &str = r#"
// Font file -> glyph grid image (reply `img`). `head` is `<px>\n<chars>\n`; the grid has 16 cells per row
// (font.rs `COLS`), each as wide as the widest glyph and as tall as the font's ascent + descent.
T.f = async (id, name, head) => { try {
  const [px, chars] = decodeURIComponent(head).split("\n"), cs = [...chars];
  const f = new FontFace("f", await (await get(id, name)).arrayBuffer()); await f.load(); document.fonts.add(f);
  const c = new OffscreenCanvas(1, 1), x = c.getContext("2d", { willReadFrequently: true });
  x.font = px + "px f";
  const m = x.measureText("Mg"), a = m.fontBoundingBoxAscent;
  const cw = Math.ceil(Math.max(...cs.map(g => x.measureText(g).width))), ch = Math.ceil(a + m.fontBoundingBoxDescent);
  const w = cw * 16, h = ch * Math.ceil(cs.length / 16);
  if (w > 4096 || h > 4096) throw 0;
  c.width = w; c.height = h; x.font = px + "px f"; x.fillStyle = "white";
  cs.forEach((g, k) => x.fillText(g, k % 16 * cw, Math.floor(k / 16) * ch + a));
  reply(id, "img", w, h, x.getImageData(0, 0, w, h).data);
} catch (e) { fail(id, e); } };
"#;

const GO: &str = r#"const go = (id, k, name, head) => (post(id + ":s1"), T[k] ? T[k](id, name, head) : post(id + ":error"));"#;

/// The page script: `BASE`, the shared helpers plus the decoders of the enabled features.
pub fn page() -> String {
    let parts = [
        COMMON,
        #[cfg(feature = "image")]
        IMG,
        #[cfg(feature = "audio")]
        AUD,
        #[cfg(feature = "font")]
        FNT,
        GO,
    ];
    let mut s = alloc::format!("const B = \"{BASE}\";");
    s.extend(parts);
    s
}

/// Closes the page's script and, on WebView2 (where the host sends jobs as messages), listens for them.
#[cfg(unix)]
pub const END: &str = "post(\"ready\");</script>";
#[cfg(windows)]
pub const END: &str = r#"chrome.webview.addEventListener("message", e => {
  const m = e.data.split(":"); go(+m[0], m[1], m[2], m[3]);
});
post("ready");</script>"#;

pub enum Decoded {
    #[cfg(feature = "image")]
    Image(Image),
    #[cfg(feature = "audio")]
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

/// Filled by the backend's callbacks, drained by `Loader::poll`. `None` = that job failed.
pub struct Inbox {
    pub state: State,
    pub done: Vec<(usize, Option<Decoded>)>,
    /// Jobs whose file the host could not read.
    pub missing: Vec<usize>,
    /// `(job, n)`: how far the page got (1 job received, 2 file fetched, 3 result being posted). For timeout errors.
    pub stages: Vec<(usize, u8)>,
}

impl Inbox {
    pub fn new() -> Self {
        Self {
            state: State::Starting,
            done: Vec::new(),
            missing: Vec::new(),
            stages: Vec::new(),
        }
    }

    /// One plain message from the page: `ready`, `<id>:error`, `<id>:nofile` or `<id>:s<n>` (stage).
    pub fn push(&mut self, mut m: &[u8]) {
        let id = field(&mut m, b':');
        if id == b"ready" {
            self.state = State::Ready;
        } else if let Some(id) = num(id) {
            if let [b's', n] = m {
                self.stages.push((id as usize, n - b'0'));
            } else if m == b"nofile" {
                self.missing.push(id as usize);
            } else {
                self.done.push((id as usize, None));
            }
        }
    }

    /// A result POSTed by the page: `path` is what follows `BASE` (`r/<id>/<tag>/<a>/<b>`), `body` the raw bytes.
    /// Anything that does not parse is dropped (the job then stays `Loading`; only our own page posts here).
    pub fn push_reply(&mut self, mut path: &[u8], body: &[u8]) {
        let (r, id) = (field(&mut path, b'/'), field(&mut path, b'/'));
        let (tag, a) = (field(&mut path, b'/'), field(&mut path, b'/'));
        if let (b"r", Some(id), Some(a), Some(b)) = (r, num(id), num(a), num(path)) {
            self.done.push((id as usize, decode(tag, a, b, body)));
        }
    }
}

fn field<'a>(s: &mut &'a [u8], sep: u8) -> &'a [u8] {
    let i = s.iter().position(|&c| c == sep).unwrap_or(s.len());
    let f = &s[..i];
    *s = &s[(i + 1).min(s.len())..];
    f
}

fn num(f: &[u8]) -> Option<u32> {
    core::str::from_utf8(f).ok()?.parse().ok()
}

/// `None` for an unknown tag, or a body that does not fit what the numbers claim.
fn decode(tag: &[u8], a: u32, b: u32, data: &[u8]) -> Option<Decoded> {
    match tag {
        #[cfg(feature = "image")]
        b"img" => ((1..=MAX).contains(&a)
            && (1..=MAX).contains(&b)
            && data.len() == a as usize * b as usize * 4)
            .then(|| {
                Decoded::Image(Image {
                    w: a,
                    h: b,
                    rgba: data.into(),
                })
            }),
        #[cfg(feature = "audio")]
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

/// Percent-escape everything but `A-Za-z0-9-._~`: safe inside a URL path, a JS string and a `:`-split message.
pub fn escape(s: &str) -> String {
    let mut out = String::new();
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&alloc::format!("%{b:02X}"));
        }
    }
    out
}

/// The inverse of `escape` (for the host side of `a/<name>`); a stray `%` stays as it is.
pub fn unescape(s: &[u8]) -> Vec<u8> {
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let (mut out, mut i) = (Vec::with_capacity(s.len()), 0);
    while i < s.len() {
        match (
            s[i],
            s.get(i + 1).and_then(|&c| hex(c)),
            s.get(i + 2).and_then(|&c| hex(c)),
        ) {
            (b'%', Some(h), Some(l)) => {
                out.push(h << 4 | l);
                i += 3;
            }
            (c, ..) => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

#[cfg(all(test, feature = "image", feature = "audio"))]
mod tests {
    extern crate std;
    use super::*;
    use alloc::vec;

    #[test]
    fn escape_roundtrip() {
        let s = "px\nA b:ü/%é\"\\<=.png";
        let e = escape(s);
        assert!(
            e.bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-._~%".contains(&c))
        );
        assert_eq!(unescape(e.as_bytes()), s.as_bytes());
        assert_eq!(unescape(b"100%"), b"100%"); // stray percent
    }

    #[test]
    fn replies() {
        let mut ib = Inbox::new();
        ib.push(b"ready");
        assert!(ib.state == State::Ready && ib.done.is_empty());

        ib.push_reply(b"r/3/img/2/1", &[1, 2, 3, 4, 5, 6, 7, 8]);
        ib.push_reply(b"r/4/img/2/1", &[1, 2, 3]); // short pixels
        ib.push_reply(b"r/5/img/5000/1", &[]); // over MAX
        ib.push(b"6:error");
        ib.push(b"junk"); // no id: ignored
        ib.push(b"10:nofile");
        ib.push_reply(b"r/11/img/2", &[]); // path too short: dropped
        ib.push_reply(b"x/12/img/2/1", &[0; 8]); // not a reply path: dropped
        let mut f = vec![];
        for s in [0.5f32, -0.5, 1.0, 1.0] {
            f.extend_from_slice(&s.to_le_bytes()); // planar: ch0 = [0.5, -0.5], ch1 = [1, 1]
        }
        ib.push_reply(b"r/7/audio/2/48000", &f);
        ib.push_reply(b"r/8/audio/0/48000", &f); // zero channels
        ib.push_reply(b"r/9/audio/3/48000", &f); // 4 samples do not split into 3 channels

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
        assert_eq!(ib.missing, [10]);
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
