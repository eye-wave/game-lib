// ALSA via dlopen: no libasound => `open` returns None and the game runs silent.
// Works on PulseAudio/PipeWire too (both ship an ALSA "default" device).
use super::RATE;
use core::{
    ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void},
    mem::transmute,
    ptr::{null, null_mut},
};

// Old glibc keeps dlopen/pthread out of libc, new glibc ships stubs with these sonames.
#[link(name = "libdl.so.2", kind = "dylib", modifiers = "+verbatim")]
unsafe extern "C" {
    fn dlopen(file: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(lib: *mut c_void, name: *const c_char) -> *mut c_void;
}

#[link(name = "libpthread.so.0", kind = "dylib", modifiers = "+verbatim")]
unsafe extern "C" {
    fn pthread_create(
        t: *mut c_ulong,
        attr: *const c_void,
        f: extern "C" fn(*mut c_void) -> *mut c_void,
        arg: *mut c_void,
    ) -> c_int;
}

#[link(name = "c")]
unsafe extern "C" {
    fn usleep(us: c_uint) -> c_int;
}

type Open = unsafe extern "C" fn(*mut *mut c_void, *const c_char, c_int, c_int) -> c_int;
type SetParams =
    unsafe extern "C" fn(*mut c_void, c_int, c_int, c_uint, c_uint, c_int, c_uint) -> c_int;
type Writei = unsafe extern "C" fn(*mut c_void, *const c_void, c_ulong) -> c_long;
type Recover = unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_int;

pub struct Dev {
    pcm: *mut c_void,
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
    let mut pcm: *mut c_void = null_mut();
    // stream 0 = playback, mode 0 = blocking: writei paces the audio thread
    if open(&mut pcm, c"default".as_ptr(), 0, 0) < 0 {
        return None;
    }
    // S16_LE (2), interleaved (3), stereo, allow resampling, 80 ms buffer
    if set_params(pcm, 2, 3, 2, RATE, 1, 80_000) < 0 {
        return None;
    }
    Some(Dev {
        pcm,
        writei: sym!(c"snd_pcm_writei", Writei),
        recover: sym!(c"snd_pcm_recover", Recover),
    })
}

impl Dev {
    /// Mix and write 10 ms; blocks while the device buffer is full. Underruns click, then recover.
    pub unsafe fn pump(&mut self, mut mix: impl FnMut(&mut [i16])) {
        let mut buf = [0i16; 960];
        mix(&mut buf);
        let n = (self.writei)(self.pcm, buf.as_ptr().cast(), 480);
        // a dead device sleeps instead of spinning; no reopen logic.
        if n < 0 && (self.recover)(self.pcm, n as c_int, 1) < 0 {
            usleep(10_000);
        }
    }
}

extern "C" fn tramp(a: *mut c_void) -> *mut c_void {
    unsafe { crate::thread_main(a) };
    null_mut()
}

/// Runs `thread_main(arg)` on a new thread. The thread is never joined.
pub unsafe fn spawn(arg: *mut c_void) -> bool {
    let mut t: c_ulong = 0;
    pthread_create(&mut t, null(), tramp, arg) == 0
}
