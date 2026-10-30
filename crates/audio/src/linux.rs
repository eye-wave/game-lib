// ALSA via dlopen: no libasound => `open` returns None and the game runs silent.
// Works on PulseAudio/PipeWire too (both ship an ALSA "default" device).
use super::RATE;
use core::{
    ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void},
    mem::transmute,
};

#[link(name = "libdl.so.2", kind = "dylib", modifiers = "+verbatim")]
unsafe extern "C" {
    fn dlopen(file: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(lib: *mut c_void, name: *const c_char) -> *mut c_void;
}

type Open = unsafe extern "C" fn(*mut *mut c_void, *const c_char, c_int, c_int) -> c_int;
type SetParams =
    unsafe extern "C" fn(*mut c_void, c_int, c_int, c_uint, c_uint, c_int, c_uint) -> c_int;
type Avail = unsafe extern "C" fn(*mut c_void) -> c_long;
type Writei = unsafe extern "C" fn(*mut c_void, *const c_void, c_ulong) -> c_long;
type Recover = unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_int;

pub struct Dev {
    pcm: *mut c_void,
    avail: Avail,
    writei: Writei,
    recover: Recover,
}

pub unsafe fn open() -> Option<Dev> {
    let lib = dlopen(c"libasound.so.2".as_ptr(), 2); // RTLD_NOW
    if lib.is_null() {
        return None;
    }
    macro_rules! sym {
        ($n:literal, $t:ty) => {{
            let p = dlsym(lib, $n.as_ptr());
            if p.is_null() {
                return None;
            }
            transmute::<*mut c_void, $t>(p)
        }};
    }
    let open = sym!(c"snd_pcm_open", Open);
    let set_params = sym!(c"snd_pcm_set_params", SetParams);
    let mut pcm: *mut c_void = core::ptr::null_mut();
    // stream 0 = playback, mode 1 = non-blocking so writei never stalls the frame loop
    if open(&mut pcm, c"default".as_ptr(), 0, 1) < 0 {
        return None;
    }
    // S16_LE (2), interleaved (3), stereo, allow resampling, 80 ms buffer
    if set_params(pcm, 2, 3, 2, RATE, 1, 80_000) < 0 {
        return None;
    }
    Some(Dev {
        pcm,
        avail: sym!(c"snd_pcm_avail_update", Avail),
        writei: sym!(c"snd_pcm_writei", Writei),
        recover: sym!(c"snd_pcm_recover", Recover),
    })
}

impl Dev {
    /// Once per frame: top up the device buffer. A frame hitch over ~80 ms underruns (click), then recovers.
    pub unsafe fn pump(&mut self, mut mix: impl FnMut(&mut [i16])) {
        let mut free = (self.avail)(self.pcm);
        if free < 0 {
            (self.recover)(self.pcm, free as c_int, 1);
            return;
        }
        let mut buf = [0i16; 2048];
        while free > 0 {
            let frames = (free as usize).min(1024);
            mix(&mut buf[..frames * 2]);
            let n = (self.writei)(self.pcm, buf.as_ptr().cast(), frames as c_ulong);
            if n < 0 {
                (self.recover)(self.pcm, n as c_int, 1);
            }
            if n <= 0 {
                return;
            }
            free -= n;
        }
    }
}
