// One dlopen per format, so a missing libwebp only breaks WebP files (IMAGE_LIBRARY_MISSING).
use crate::{Clip, Image, MAX, push};
use alloc::{vec, vec::Vec};
use core::{
    ffi::{CStr, c_char, c_int, c_long, c_ulong, c_void},
    mem::{transmute, zeroed},
    ptr::{null, null_mut},
    slice,
};
use errors::assets::*;

type R = Result<Image, &'static CStr>;
type A = Result<Clip, &'static CStr>;

// Same dlopen shim as audio: old glibc keeps it out of libc, new glibc ships a stub with this soname.
#[link(name = "libdl.so.2", kind = "dylib", modifiers = "+verbatim")]
unsafe extern "C" {
    fn dlopen(file: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(lib: *mut c_void, name: *const c_char) -> *mut c_void;
}

#[link(name = "c")]
unsafe extern "C" {
    fn free(p: *mut c_void);
}

/// `miss` is the error for a library that is not installed.
fn open(name: &CStr, miss: &'static CStr) -> Result<*mut c_void, &'static CStr> {
    let lib = unsafe { dlopen(name.as_ptr(), 2) }; // RTLD_NOW
    if lib.is_null() { Err(miss) } else { Ok(lib) }
}

/// Looks `$n` up in `$lib` as fn type `$t`; a missing symbol (library too old) returns `$miss`. Use inside `unsafe`.
macro_rules! sym {
    ($lib:expr, $miss:expr, $n:literal, $t:ty) => {{
        let p = dlsym($lib, $n.as_ptr());
        if p.is_null() {
            return Err($miss);
        }
        transmute::<*mut c_void, $t>(p)
    }};
}

fn dim_ok(v: c_int) -> bool {
    (1..=MAX as c_int).contains(&v)
}

// ---------------------------------------------------------------- PNG: libpng's simplified API (1.6+)
// No setjmp, and it converts palette/gray/16-bit to 8-bit RGBA for us.

/// `png_image` from png.h.
#[repr(C)]
struct PngImage {
    opaque: *mut c_void,
    version: u32,
    width: u32,
    height: u32,
    format: u32,
    flags: u32,
    colormap_entries: u32,
    warning_or_error: u32,
    message: [u8; 64],
}

type Begin = unsafe extern "C" fn(*mut PngImage, *const c_void, usize) -> c_int;
type Finish =
    unsafe extern "C" fn(*mut PngImage, *const c_void, *mut c_void, i32, *const c_void) -> c_int;
type Release = unsafe extern "C" fn(*mut PngImage);

pub fn png(file: &[u8]) -> R {
    unsafe {
        let miss = IMAGE_LIBRARY_MISSING;
        let lib = open(c"libpng16.so.16", miss)?;
        let begin = sym!(lib, miss, c"png_image_begin_read_from_memory", Begin);
        let finish = sym!(lib, miss, c"png_image_finish_read", Finish);
        let release = sym!(lib, miss, c"png_image_free", Release);

        let mut im: PngImage = zeroed();
        im.version = 1; // PNG_IMAGE_VERSION
        let mut out = Err(IMAGE_DECODE_FAILED);
        if begin(&mut im, file.as_ptr().cast(), file.len()) != 0
            && dim_ok(im.width as c_int)
            && dim_ok(im.height as c_int)
        {
            im.format = 3; // PNG_FORMAT_RGBA
            let mut rgba = vec![0u8; im.width as usize * im.height as usize * 4];
            // no background, row stride 0 = tightly packed, no colormap
            if finish(&mut im, null(), rgba.as_mut_ptr().cast(), 0, null()) != 0 {
                out = Ok(Image {
                    w: im.width,
                    h: im.height,
                    rgba,
                });
            }
        }
        release(&mut im); // no-op after finish_read; needed when we bail between begin and finish
        out
    }
}

// ---------------------------------------------------------------- JPEG: TurboJPEG's flat API
// The 2.x names, still exported by 3.x. libjpeg itself needs setjmp and a version-specific struct.

type Init = unsafe extern "C" fn() -> *mut c_void;
type Header = unsafe extern "C" fn(
    *mut c_void,
    *const u8,
    c_ulong,
    *mut c_int,
    *mut c_int,
    *mut c_int,
    *mut c_int,
) -> c_int;
type Decompress = unsafe extern "C" fn(
    *mut c_void,
    *const u8,
    c_ulong,
    *mut u8,
    c_int,
    c_int,
    c_int,
    c_int,
    c_int,
) -> c_int;
type Destroy = unsafe extern "C" fn(*mut c_void) -> c_int;

pub fn jpeg(file: &[u8]) -> R {
    unsafe {
        let miss = IMAGE_LIBRARY_MISSING;
        let lib = open(c"libturbojpeg.so.0", miss)?;
        let init = sym!(lib, miss, c"tjInitDecompress", Init);
        let header = sym!(lib, miss, c"tjDecompressHeader3", Header);
        let decompress = sym!(lib, miss, c"tjDecompress2", Decompress);
        let destroy = sym!(lib, miss, c"tjDestroy", Destroy);

        let tj = init();
        if tj.is_null() {
            return Err(IMAGE_DECODE_FAILED);
        }
        let (mut w, mut h, mut subsamp, mut space) = (0, 0, 0, 0);
        let n = file.len() as c_ulong;
        let mut out = Err(IMAGE_DECODE_FAILED);
        if header(
            tj,
            file.as_ptr(),
            n,
            &mut w,
            &mut h,
            &mut subsamp,
            &mut space,
        ) == 0
            && dim_ok(w)
            && dim_ok(h)
        {
            let mut rgba = vec![0u8; w as usize * h as usize * 4];
            // pitch 0 = tightly packed, pixel format 7 = TJPF_RGBA (alpha 255), flags 0.
            // Any warning (truncated data) is a failure too.
            if decompress(tj, file.as_ptr(), n, rgba.as_mut_ptr(), w, 0, h, 7, 0) == 0 {
                out = Ok(Image {
                    w: w as u32,
                    h: h as u32,
                    rgba,
                });
            }
        }
        destroy(tj);
        out
    }
}

// ---------------------------------------------------------------- WebP: libwebp's one-shot decoder

type Info = unsafe extern "C" fn(*const u8, usize, *mut c_int, *mut c_int) -> c_int;
type Webp = unsafe extern "C" fn(*const u8, usize, *mut c_int, *mut c_int) -> *mut u8;

pub fn webp(file: &[u8]) -> R {
    unsafe {
        let miss = IMAGE_LIBRARY_MISSING;
        let lib = open(c"libwebp.so.7", miss)?;
        let info = sym!(lib, miss, c"WebPGetInfo", Info);
        let decode = sym!(lib, miss, c"WebPDecodeRGBA", Webp);

        let (mut w, mut h) = (0, 0);
        // header only: reject oversize before libwebp allocates
        if info(file.as_ptr(), file.len(), &mut w, &mut h) == 0 || !dim_ok(w) || !dim_ok(h) {
            return Err(IMAGE_DECODE_FAILED);
        }
        let p = decode(file.as_ptr(), file.len(), &mut w, &mut h);
        if p.is_null() {
            return Err(IMAGE_DECODE_FAILED);
        }
        let rgba = slice::from_raw_parts(p, w as usize * h as usize * 4).to_vec();
        free(p.cast()); // WebPFree is only exported since 1.3.1; it is plain malloc underneath
        Ok(Image {
            w: w as u32,
            h: h as u32,
            rgba,
        })
    }
}

// ---------------------------------------------------------------- Ogg Vorbis: libvorbisfile

#[repr(C)]
struct OvCallbacks {
    read: unsafe extern "C" fn(*mut c_void, usize, usize, *mut c_void) -> usize,
    seek: *const c_void,
    close: *const c_void,
    tell: *const c_void,
}
#[repr(C)]
struct VorbisInfo {
    version: c_int,
    channels: c_int,
    rate: c_long,
}

type OvOpen =
    unsafe extern "C" fn(*mut c_void, *mut c_void, *const u8, c_long, OvCallbacks) -> c_int;
type OvInfo = unsafe extern "C" fn(*mut c_void, c_int) -> *const VorbisInfo;
type OvRead =
    unsafe extern "C" fn(*mut c_void, *mut u8, c_int, c_int, c_int, c_int, *mut c_int) -> c_long;
type OvClear = unsafe extern "C" fn(*mut c_void) -> c_int;

unsafe extern "C" fn eof(_: *mut c_void, _: usize, _: usize, _: *mut c_void) -> usize {
    0
}

/// Chained streams (changing rate or channels mid-file) are not handled.
pub fn ogg(file: &[u8]) -> A {
    unsafe {
        let miss = AUDIO_LIBRARY_MISSING;
        let lib = open(c"libvorbisfile.so.3", miss)?;
        let ov_open = sym!(lib, miss, c"ov_open_callbacks", OvOpen);
        let ov_info = sym!(lib, miss, c"ov_info", OvInfo);
        let ov_read = sym!(lib, miss, c"ov_read", OvRead);
        let ov_clear = sym!(lib, miss, c"ov_clear", OvClear);

        // OggVorbis_File is about 1 KB and opaque to us; this is 2 KB of zeroed, aligned room.
        let mut vf = [0u64; 256];
        let vf = vf.as_mut_ptr().cast::<c_void>();
        // The whole file goes in as "bytes already read". With no datasource, libvorbisfile treats the end of
        // those bytes as end of stream, so there is no read state to keep and the callbacks only need to exist.
        let cb = OvCallbacks {
            read: eof,
            seek: null(),
            close: null(),
            tell: null(),
        };
        if ov_open(null_mut(), vf, file.as_ptr(), file.len() as c_long, cb) != 0 {
            return Err(AUDIO_DECODE_FAILED);
        }
        // read these now: the info is freed by ov_clear
        let info = &*ov_info(vf, -1);
        let (rate, channels) = (info.rate as u32, info.channels as u32);
        let mut pcm = Vec::new();
        let mut chunk = [0u8; 4096];
        let mut ok = true;
        loop {
            // little-endian (0), 16-bit words (2), signed (1)
            let n = ov_read(vf, chunk.as_mut_ptr(), 4096, 0, 2, 1, &mut 0);
            match n {
                0 => break,
                -3 => {} // OV_HOLE: a gap in the data, keep going
                ..0 => ok = false,
                _ => ok = push(&mut pcm, &chunk[..n as usize]),
            }
            if !ok {
                break;
            }
        }
        ov_clear(vf);
        if ok && rate > 0 && channels > 0 && !pcm.is_empty() {
            Ok(Clip {
                rate,
                channels,
                pcm,
            })
        } else {
            Err(AUDIO_DECODE_FAILED)
        }
    }
}

// ---------------------------------------------------------------- MP3: libmpg123 in feed mode

type MpgInit = unsafe extern "C" fn() -> c_int;
type MpgNew = unsafe extern "C" fn(*const c_char, *mut c_int) -> *mut c_void;
type MpgOpen = unsafe extern "C" fn(*mut c_void) -> c_int;
type MpgFeed = unsafe extern "C" fn(*mut c_void, *const u8, usize) -> c_int;
type MpgRead = unsafe extern "C" fn(*mut c_void, *mut u8, usize, *mut usize) -> c_int;
type MpgFormat = unsafe extern "C" fn(*mut c_void, *mut c_long, *mut c_int, *mut c_int) -> c_int;
type MpgDelete = unsafe extern "C" fn(*mut c_void);

/// Leading ID3 tags are skipped by the library. Encoder delay/padding is not trimmed (no gapless playback).
pub fn mp3(file: &[u8]) -> A {
    unsafe {
        let miss = AUDIO_LIBRARY_MISSING;
        let lib = open(c"libmpg123.so.0", miss)?;
        let init = sym!(lib, miss, c"mpg123_init", MpgInit);
        let new = sym!(lib, miss, c"mpg123_new", MpgNew);
        let open_feed = sym!(lib, miss, c"mpg123_open_feed", MpgOpen);
        let feed = sym!(lib, miss, c"mpg123_feed", MpgFeed);
        let read = sym!(lib, miss, c"mpg123_read", MpgRead);
        let format = sym!(lib, miss, c"mpg123_getformat", MpgFormat);
        let delete = sym!(lib, miss, c"mpg123_delete", MpgDelete);

        init();
        let h = new(null(), null_mut());
        if h.is_null() {
            return Err(AUDIO_DECODE_FAILED);
        }
        let (mut rate, mut channels) = (0, 0);
        let mut pcm = Vec::new();
        let mut chunk = [0u8; 8192];
        // the whole file is fed up front; read then decodes until it runs dry
        let mut ok = open_feed(h) == 0 && feed(h, file.as_ptr(), file.len()) == 0;
        while ok {
            let mut done = 0;
            let r = read(h, chunk.as_mut_ptr(), chunk.len(), &mut done);
            ok = push(&mut pcm, &chunk[..done]);
            match r {
                0 => {}
                -11 => {
                    // MPG123_NEW_FORMAT: the stream header has been parsed
                    let (mut hz, mut ch, mut enc) = (0, 0, 0);
                    // 0xD0 = MPG123_ENC_SIGNED_16, which is the library default
                    ok &= format(h, &mut hz, &mut ch, &mut enc) == 0 && enc == 0xD0;
                    (rate, channels) = (hz as u32, ch as u32);
                }
                _ => break, // -10 NEED_MORE (everything fed is decoded), -12 DONE, or an error
            }
        }
        delete(h);
        if ok && rate > 0 && channels > 0 && !pcm.is_empty() {
            Ok(Clip {
                rate,
                channels,
                pcm,
            })
        } else {
            Err(AUDIO_DECODE_FAILED)
        }
    }
}
