// Images: GDI+ flat C API (gdiplus.dll, on every Windows). WebP goes through WIC, which only has a WebP codec if the
// user installed Microsoft's "WebP Image Extensions". Sounds: Media Foundation (MP3 built in; Ogg Vorbis only with
// the "Web Media Extensions"). Without the codec: IMAGE_LIBRARY_MISSING / AUDIO_LIBRARY_MISSING.
// Note: mfplat.dll is imported at load time, so a Windows N edition without the Media Feature Pack cannot start
// an exe that calls `mp3` or `ogg`; programs that never call them do not import it.
use crate::{Clip, Image, MAX, push};
use alloc::{vec, vec::Vec};
use core::{
    ffi::{CStr, c_void},
    mem::{transmute_copy, zeroed},
    ptr::{null, null_mut},
    slice,
};
use errors::assets::*;

#[repr(C)]
struct Startup {
    version: u32,
    callback: *const c_void,
    no_thread: i32,
    no_codecs: i32,
}
#[repr(C)]
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}
#[repr(C)]
struct Data {
    w: u32,
    h: u32,
    stride: i32,
    format: i32,
    scan0: *mut u8,
    reserved: usize,
}
#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);

const LOCK_READ: u32 = 1; // ImageLockModeRead
const ARGB32: i32 = 0x0026_200A; // PixelFormat32bppARGB: straight alpha, bytes in memory are B G R A

const CLSID_WIC_FACTORY: Guid = Guid(
    0xcacaf262,
    0x9370,
    0x4615,
    [0xa1, 0x3b, 0x9f, 0x55, 0x39, 0xda, 0x4c, 0x0a],
);
const IID_WIC_FACTORY: Guid = Guid(
    0xec5ec8a9,
    0xc395,
    0x4314,
    [0x9c, 0x77, 0x54, 0xd7, 0xa9, 0x35, 0xff, 0x70],
);
const WIC_BGRA: Guid = Guid(
    0x6fddc324,
    0x4e03,
    0x4bfe,
    [0xb1, 0x85, 0x3d, 0x77, 0x76, 0x8d, 0xc9, 0x0f],
);
const MF_MT_MAJOR_TYPE: Guid = Guid(
    0x48eba18e,
    0xf8c9,
    0x4687,
    [0xbf, 0x11, 0x0a, 0x74, 0xc9, 0xf9, 0x6a, 0x8f],
);
const MF_MT_SUBTYPE: Guid = Guid(
    0xf7e34c9a,
    0x42e8,
    0x4714,
    [0xb7, 0x4b, 0xcb, 0x29, 0xd7, 0x2c, 0x35, 0xe5],
);
const MF_MT_BITS: Guid = Guid(
    0xf2deb57f,
    0x40fa,
    0x4764,
    [0xaa, 0x33, 0xed, 0x4f, 0x2d, 0x1f, 0xf6, 0x69],
); // AUDIO_BITS_PER_SAMPLE
const MF_MT_CHANNELS: Guid = Guid(
    0x37e48bf5,
    0x645e,
    0x4c5b,
    [0x89, 0xde, 0xad, 0xa9, 0xe2, 0x9b, 0x69, 0x6a],
); // AUDIO_NUM_CHANNELS
const MF_MT_RATE: Guid = Guid(
    0x5faeeae7,
    0x0290,
    0x4c31,
    [0x9e, 0x8a, 0xc5, 0x34, 0xf6, 0x8d, 0x9d, 0xba],
); // AUDIO_SAMPLES_PER_SECOND
const MF_AUDIO: Guid = Guid(
    0x73647561,
    0x0000,
    0x0010,
    [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
); // MFMediaType_Audio
const MF_PCM: Guid = Guid(
    0x0000_0001,
    0x0000,
    0x0010,
    [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
); // MFAudioFormat_PCM
const MF_FIRST_AUDIO_STREAM: u32 = 0xFFFF_FFFD;
const MF_VERSION: u32 = 0x0002_0070;
const MF_UNSUPPORTED_BYTESTREAM_TYPE: u32 = 0xC00D_36C4;
const MF_TOPO_CODEC_NOT_FOUND: u32 = 0xC00D_5212;
const WIC_COMPONENT_NOT_FOUND: u32 = 0x8898_2F50;
const WIC_UNKNOWN_FORMAT: u32 = 0x8898_2F07;

const MF_BYTESTREAM_ORIGIN_NAME: Guid = Guid(
    0xfc358288,
    0x3cb6,
    0x460c,
    [0xa4, 0x24, 0xb6, 0x68, 0x12, 0x60, 0x37, 0x5a],
);
const MF_BYTESTREAM_CONTENT_TYPE: Guid = Guid(
    0xfc358289,
    0x3cb6,
    0x460c,
    [0xa4, 0x24, 0xb6, 0x68, 0x12, 0x60, 0x37, 0x5a],
);
const IID_ATTRIBUTES: Guid = Guid(
    0x2cd2d921,
    0xc447,
    0x44a7,
    [0xa1, 0x3c, 0x4a, 0xda, 0xbf, 0xc2, 0x47, 0xe3],
); // IID_IMFAttributes

#[link(name = "gdiplus", kind = "raw-dylib")]
unsafe extern "system" {
    fn GdiplusStartup(token: *mut usize, input: *const Startup, output: *mut c_void) -> i32;
    fn GdiplusShutdown(token: usize);
    fn GdipCreateBitmapFromStream(stream: *mut c_void, bmp: *mut *mut c_void) -> i32;
    fn GdipGetImageWidth(img: *mut c_void, w: *mut u32) -> i32;
    fn GdipGetImageHeight(img: *mut c_void, h: *mut u32) -> i32;
    fn GdipBitmapLockBits(
        bmp: *mut c_void,
        r: *const Rect,
        mode: u32,
        fmt: i32,
        d: *mut Data,
    ) -> i32;
    fn GdipBitmapUnlockBits(bmp: *mut c_void, d: *mut Data) -> i32;
    fn GdipDisposeImage(img: *mut c_void) -> i32;
}

#[link(name = "shlwapi", kind = "raw-dylib")]
unsafe extern "system" {
    /// IStream over a copy of the bytes.
    fn SHCreateMemStream(p: *const u8, n: u32) -> *mut c_void;
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *const c_void, mode: u32) -> i32;
    fn CoUninitialize();
    fn CoCreateInstance(
        clsid: *const Guid,
        outer: *mut c_void,
        ctx: u32,
        iid: *const Guid,
        out: *mut *mut c_void,
    ) -> i32;
}

type R = Result<Image, &'static CStr>;

// GDI+ sniffs the format itself, so `png` and `jpeg` are the same call here (and also accept GIF/BMP/TIFF).
#[link(name = "mfplat", kind = "raw-dylib")]
unsafe extern "system" {
    fn MFStartup(version: u32, flags: u32) -> i32;
    fn MFShutdown() -> i32;
    fn MFCreateMediaType(out: *mut *mut c_void) -> i32;
    fn MFCreateMFByteStreamOnStream(stream: *mut c_void, out: *mut *mut c_void) -> i32;
}

#[link(name = "mfreadwrite", kind = "raw-dylib")]
unsafe extern "system" {
    fn MFCreateSourceReaderFromByteStream(
        bytes: *mut c_void,
        attrs: *mut c_void,
        out: *mut *mut c_void,
    ) -> i32;
}

type A = Result<Clip, &'static CStr>;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetStdHandle(n: u32) -> *mut c_void;
    fn WriteFile(h: *mut c_void, buf: *const u8, n: u32, written: *mut u32, ov: *mut c_void)
    -> i32;
    fn OutputDebugStringA(s: *const u8);
}

/// DEBUG: prints `tag 0xHHHHHHHH` to stdout and to the debugger (DebugView / VS output). Remove when done.
fn dbg(tag: &str, v: u32) {
    let mut b = [0u8; 96];
    let mut n = 0;
    for &c in tag.as_bytes().iter().take(80) {
        b[n] = c;
        n += 1;
    }
    for &c in b" 0x" {
        b[n] = c;
        n += 1;
    }
    for i in (0..8).rev() {
        b[n] = b"0123456789ABCDEF"[(v >> (i * 4)) as usize & 15];
        n += 1;
    }
    b[n] = b'\n';
    n += 1;
    unsafe {
        let mut w = 0;
        WriteFile(
            GetStdHandle(0xFFFF_FFF5),
            b.as_ptr(),
            n as u32,
            &mut w,
            null_mut(),
        ); // STD_OUTPUT_HANDLE
        b[n] = 0;
        OutputDebugStringA(b.as_ptr());
    }
}

pub fn png(file: &[u8]) -> R {
    with_stream(file, IMAGE_DECODE_FAILED, gdi)
}

pub fn jpeg(file: &[u8]) -> R {
    with_stream(file, IMAGE_DECODE_FAILED, gdi)
}

pub fn webp(file: &[u8]) -> R {
    with_stream(file, IMAGE_DECODE_FAILED, wic)
}

// Media Foundation sniffs MP3 by itself, but Ogg needs a content-type hint on the byte stream.
pub fn mp3(file: &[u8]) -> A {
    with_stream(file, AUDIO_DECODE_FAILED, mf_mp3)
}

pub fn ogg(file: &[u8]) -> A {
    with_stream(file, AUDIO_DECODE_FAILED, mf_ogg)
}

unsafe fn mf_mp3(stream: *mut c_void) -> A {
    unsafe { mf(stream, None) }
}

unsafe fn mf_ogg(stream: *mut c_void) -> A {
    unsafe { mf(stream, Some("audio/ogg")) }
}

/// Puts the bytes in an IStream for the decoder. `fail` is the error if that stream cannot be made.
fn with_stream<T>(
    file: &[u8],
    fail: &'static CStr,
    decode: unsafe fn(*mut c_void) -> Result<T, &'static CStr>,
) -> Result<T, &'static CStr> {
    unsafe {
        let stream = SHCreateMemStream(file.as_ptr(), file.len() as u32);
        if stream.is_null() {
            return Err(fail);
        }
        let out = decode(stream);
        // the decoder is done with the stream (GDI+ needs it alive until its bitmap is disposed, done in `load`)
        release(stream);
        out
    }
}

// ---------------------------------------------------------------- COM by hand: object -> vtable -> slot

/// Method `i` of COM object `o`; slots 0..3 are QueryInterface, AddRef, Release.
unsafe fn com<F: Copy>(o: *mut c_void, i: usize) -> F {
    unsafe { transmute_copy(&*(*(o as *const *const usize)).add(i)) }
}

unsafe fn release(o: *mut c_void) {
    unsafe { com::<unsafe extern "system" fn(*mut c_void) -> u32>(o, 2)(o) };
}

// ---------------------------------------------------------------- WIC (WebP)

type CreateDecoder =
    unsafe extern "system" fn(*mut c_void, *mut c_void, *const Guid, u32, *mut *mut c_void) -> i32;
type NewObject = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32;
type GetFrame = unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32;
type ConvInit = unsafe extern "system" fn(
    *mut c_void,
    *mut c_void,
    *const Guid,
    u32,
    *mut c_void,
    f64,
    u32,
) -> i32;
type GetSize = unsafe extern "system" fn(*mut c_void, *mut u32, *mut u32) -> i32;
type CopyPixels = unsafe extern "system" fn(*mut c_void, *const Rect, u32, u32, *mut u8) -> i32;

/// Runs `f` with COM initialised on this thread.
unsafe fn with_com<T>(f: impl FnOnce() -> T) -> T {
    // S_OK and S_FALSE both need a CoUninitialize; a negative result (COM already up in another mode) is fine to use as is.
    let up = unsafe { CoInitializeEx(null(), 0) } >= 0; // COINIT_MULTITHREADED
    let out = f();
    if up {
        unsafe { CoUninitialize() };
    }
    out
}

unsafe fn wic(stream: *mut c_void) -> R {
    unsafe {
        with_com(|| {
            let mut factory = null_mut();
            if CoCreateInstance(
                &CLSID_WIC_FACTORY,
                null_mut(),
                1,
                &IID_WIC_FACTORY,
                &mut factory,
            ) < 0
            {
                return Err(IMAGE_LIBRARY_MISSING);
            }
            let (mut dec, mut frame, mut conv) = (null_mut(), null_mut(), null_mut());
            let out = wic_read(factory, stream, &mut dec, &mut frame, &mut conv);
            for o in [conv, frame, dec, factory] {
                if !o.is_null() {
                    release(o);
                }
            }
            out
        })
    }
}

/// Decoder -> first frame -> format converter to BGRA -> pixels. The caller releases whatever got created.
unsafe fn wic_read(
    factory: *mut c_void,
    stream: *mut c_void,
    dec: &mut *mut c_void,
    frame: &mut *mut c_void,
    conv: &mut *mut c_void,
) -> Result<Image, &'static CStr> {
    unsafe {
        // vtable slots: factory 4 CreateDecoderFromStream, 10 CreateFormatConverter; decoder 13 GetFrame;
        // converter 8 Initialize, 3 GetSize, 7 CopyPixels (the last two come from IWICBitmapSource)
        let hr = com::<CreateDecoder>(factory, 4)(factory, stream, null(), 0, dec); // 0 = metadata on demand
        if hr < 0 {
            // the file says WebP, so "no codec for this" means the extension is not installed
            let hr = hr as u32;
            return Err(
                if hr == WIC_COMPONENT_NOT_FOUND || hr == WIC_UNKNOWN_FORMAT {
                    IMAGE_LIBRARY_MISSING
                } else {
                    IMAGE_DECODE_FAILED
                },
            );
        }
        // source frame, BGRA, no dither (0), no palette, alpha threshold 0, palette type custom (0)
        if com::<GetFrame>(*dec, 13)(*dec, 0, frame) < 0
            || com::<NewObject>(factory, 10)(factory, conv) < 0
            || com::<ConvInit>(*conv, 8)(*conv, *frame, &WIC_BGRA, 0, null_mut(), 0.0, 0) < 0
        {
            return Err(IMAGE_DECODE_FAILED);
        }
        let (mut w, mut h) = (0, 0);
        if com::<GetSize>(*conv, 3)(*conv, &mut w, &mut h) < 0
            || !(1..=MAX).contains(&w)
            || !(1..=MAX).contains(&h)
        {
            return Err(IMAGE_DECODE_FAILED);
        }
        let mut rgba = vec![0u8; w as usize * h as usize * 4];
        // null rect = whole image, stride = tightly packed
        if com::<CopyPixels>(*conv, 7)(*conv, null(), w * 4, rgba.len() as u32, rgba.as_mut_ptr())
            < 0
        {
            return Err(IMAGE_DECODE_FAILED);
        }
        for p in rgba.chunks_exact_mut(4) {
            p.swap(0, 2); // BGRA -> RGBA
        }
        Ok(Image { w, h, rgba })
    }
}

// ---------------------------------------------------------------- Media Foundation (MP3, Ogg)

type SetGuid = unsafe extern "system" fn(*mut c_void, *const Guid, *const Guid) -> i32;
type SetU32 = unsafe extern "system" fn(*mut c_void, *const Guid, u32) -> i32;
type GetU32 = unsafe extern "system" fn(*mut c_void, *const Guid, *mut u32) -> i32;
type SetType = unsafe extern "system" fn(*mut c_void, u32, *mut u32, *mut c_void) -> i32;
type GetType = unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32;
type ReadSample = unsafe extern "system" fn(
    *mut c_void,
    u32,
    u32,
    *mut u32,
    *mut u32,
    *mut i64,
    *mut *mut c_void,
) -> i32;
type Lock = unsafe extern "system" fn(*mut c_void, *mut *mut u8, *mut u32, *mut u32) -> i32;
type Unlock = unsafe extern "system" fn(*mut c_void) -> i32;
type Qi = unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32;
type SetStr = unsafe extern "system" fn(*mut c_void, *const Guid, *const u16) -> i32;

unsafe fn mf(stream: *mut c_void, hint: Option<&str>) -> A {
    unsafe {
        with_com(|| {
            let hr = MFStartup(MF_VERSION, 0);
            if hr < 0 {
                dbg("MFStartup failed", hr as u32);
                return Err(AUDIO_LIBRARY_MISSING);
            }
            // byte stream, source reader, requested type, actual type: released below whatever happens
            let mut objs = [null_mut(); 4];
            let out = mf_read(stream, hint, &mut objs);
            for o in objs {
                if !o.is_null() {
                    release(o);
                }
            }
            MFShutdown();
            out
        })
    }
}

/// Source reader over the stream, asked for 16-bit PCM (it inserts the decoder), then read sample by sample.
unsafe fn mf_read(stream: *mut c_void, hint: Option<&str>, objs: &mut [*mut c_void; 4]) -> A {
    unsafe {
        let bad = AUDIO_DECODE_FAILED;
        // no demuxer or decoder for this file = the codec/extension is not installed
        let why = |at: &str, hr: i32| {
            let hr = hr as u32;
            dbg(at, hr);
            if hr == MF_UNSUPPORTED_BYTESTREAM_TYPE || hr == MF_TOPO_CODEC_NOT_FOUND {
                AUDIO_LIBRARY_MISSING
            } else {
                AUDIO_DECODE_FAILED
            }
        };
        if MFCreateMFByteStreamOnStream(stream, &mut objs[0]) < 0 {
            return Err(bad);
        }
        // hint the container so the Ogg byte-stream handler is picked (IMFAttributes slot 25 = SetString)
        if let Some(hint) = hint {
            let mut attrs = null_mut();
            let hr = com::<Qi>(objs[0], 0)(objs[0], &IID_ATTRIBUTES, &mut attrs);
            dbg("QueryInterface(IMFAttributes)", hr as u32);
            if hr >= 0 {
                let mime: Vec<u16> = hint.encode_utf16().chain([0]).collect();
                let hr =
                    com::<SetStr>(attrs, 25)(attrs, &MF_BYTESTREAM_CONTENT_TYPE, mime.as_ptr());
                dbg("SetString(CONTENT_TYPE)", hr as u32);
                let name: Vec<u16> = "file.ogg\0".encode_utf16().collect();
                let hr = com::<SetStr>(attrs, 25)(attrs, &MF_BYTESTREAM_ORIGIN_NAME, name.as_ptr());
                dbg("SetString(ORIGIN_NAME)", hr as u32);
                release(attrs);
            }
        } else {
            dbg("no hint (mp3 path)", 0);
        }
        let hr = MFCreateSourceReaderFromByteStream(objs[0], null_mut(), &mut objs[1]);
        if hr < 0 {
            return Err(why("MFCreateSourceReaderFromByteStream", hr));
        }
        if MFCreateMediaType(&mut objs[2]) < 0 {
            return Err(bad);
        }
        // IMFAttributes slots: 7 GetUINT32, 21 SetUINT32, 24 SetGUID
        let (reader, want) = (objs[1], objs[2]);
        if com::<SetGuid>(want, 24)(want, &MF_MT_MAJOR_TYPE, &MF_AUDIO) < 0
            || com::<SetGuid>(want, 24)(want, &MF_MT_SUBTYPE, &MF_PCM) < 0
            || com::<SetU32>(want, 21)(want, &MF_MT_BITS, 16) < 0
        {
            return Err(bad);
        }
        // IMFSourceReader slots: 6 GetCurrentMediaType, 7 SetCurrentMediaType, 9 ReadSample
        let hr = com::<SetType>(reader, 7)(reader, MF_FIRST_AUDIO_STREAM, null_mut(), want);
        if hr < 0 {
            return Err(why("SetCurrentMediaType(PCM)", hr));
        }
        if com::<GetType>(reader, 6)(reader, MF_FIRST_AUDIO_STREAM, &mut objs[3]) < 0 {
            return Err(bad);
        }
        let got = objs[3];
        let (mut rate, mut channels) = (0, 0);
        if com::<GetU32>(got, 7)(got, &MF_MT_RATE, &mut rate) < 0
            || com::<GetU32>(got, 7)(got, &MF_MT_CHANNELS, &mut channels) < 0
        {
            return Err(bad);
        }
        let mut pcm = Vec::new();
        loop {
            let (mut flags, mut sample) = (0, null_mut());
            let hr = com::<ReadSample>(reader, 9)(
                reader,
                MF_FIRST_AUDIO_STREAM,
                0,
                null_mut(),
                &mut flags,
                null_mut(),
                &mut sample,
            );
            if hr < 0 || flags & 1 != 0 {
                return Err(bad); // 1 = MF_SOURCE_READERF_ERROR
            }
            if !sample.is_null() {
                let ok = sample_pcm(sample, &mut pcm);
                release(sample);
                if !ok {
                    return Err(bad);
                }
            }
            if flags & 2 != 0 {
                break; // MF_SOURCE_READERF_ENDOFSTREAM
            }
        }
        if rate == 0 || channels == 0 || pcm.is_empty() {
            return Err(bad);
        }
        Ok(Clip {
            rate,
            channels,
            pcm,
        })
    }
}

/// Appends one IMFSample's audio. IMFSample slot 41 is ConvertToContiguousBuffer; IMFMediaBuffer slots 3 Lock, 4 Unlock.
unsafe fn sample_pcm(sample: *mut c_void, pcm: &mut Vec<i16>) -> bool {
    unsafe {
        let mut buf = null_mut();
        if com::<NewObject>(sample, 41)(sample, &mut buf) < 0 {
            return false;
        }
        let (mut p, mut n) = (null_mut(), 0);
        let ok = com::<Lock>(buf, 3)(buf, &mut p, null_mut(), &mut n) >= 0 && {
            let ok = push(pcm, slice::from_raw_parts(p, n as usize));
            com::<Unlock>(buf, 4)(buf);
            ok
        };
        release(buf);
        ok
    }
}

// ---------------------------------------------------------------- GDI+ (PNG, JPEG)

unsafe fn gdi(stream: *mut c_void) -> R {
    unsafe {
        let mut token = 0;
        let start = Startup {
            version: 1,
            callback: null(),
            no_thread: 0,
            no_codecs: 0,
        };
        if GdiplusStartup(&mut token, &start, null_mut()) != 0 {
            return Err(IMAGE_LIBRARY_MISSING);
        }
        let out = load(stream);
        GdiplusShutdown(token);
        out
    }
}

unsafe fn load(stream: *mut c_void) -> Result<Image, &'static CStr> {
    unsafe {
        let mut bmp = null_mut();
        if GdipCreateBitmapFromStream(stream, &mut bmp) != 0 {
            return Err(IMAGE_DECODE_FAILED);
        }
        let (mut w, mut h) = (0, 0);
        GdipGetImageWidth(bmp, &mut w);
        GdipGetImageHeight(bmp, &mut h);
        let mut out = Err(IMAGE_DECODE_FAILED);
        if (1..=MAX).contains(&w) && (1..=MAX).contains(&h) {
            let r = Rect {
                x: 0,
                y: 0,
                w: w as i32,
                h: h as i32,
            };
            let mut d: Data = zeroed();
            if GdipBitmapLockBits(bmp, &r, LOCK_READ, ARGB32, &mut d) == 0 {
                let mut rgba = vec![0u8; w as usize * h as usize * 4];
                for y in 0..h as usize {
                    // stride can be negative; scan0 is always the first row
                    let row = d.scan0.offset(y as isize * d.stride as isize);
                    for x in 0..w as usize {
                        let [b, g, r, a] = *(row.add(x * 4) as *const [u8; 4]);
                        rgba[(y * w as usize + x) * 4..][..4].copy_from_slice(&[r, g, b, a]);
                    }
                }
                GdipBitmapUnlockBits(bmp, &mut d);
                out = Ok(Image { w, h, rgba });
            }
        }
        GdipDisposeImage(bmp);
        out
    }
}
