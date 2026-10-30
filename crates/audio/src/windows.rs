// winmm waveOut: plain C, no COM, no callback; the audio thread polls it. Ring of 4 x 20 ms buffers, refilled when WHDR_DONE.
use super::RATE;
use alloc::boxed::Box;
use core::{
    ffi::c_void,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
};

#[repr(C)]
struct Fmt {
    tag: u16,
    channels: u16,
    rate: u32,
    bytes_per_sec: u32,
    align: u16,
    bits: u16,
    extra: u16,
}

#[repr(C)]
struct Hdr {
    data: *mut i16,
    len: u32,
    recorded: u32,
    user: usize,
    flags: u32,
    loops: u32,
    next: *mut Hdr,
    reserved: usize,
}

#[link(name = "winmm", kind = "raw-dylib")]
unsafe extern "system" {
    fn waveOutOpen(
        h: *mut isize,
        dev: u32,
        f: *const Fmt,
        cb: usize,
        inst: usize,
        flags: u32,
    ) -> u32;
    fn waveOutPrepareHeader(h: isize, p: *mut Hdr, n: u32) -> u32;
    fn waveOutWrite(h: isize, p: *mut Hdr, n: u32) -> u32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateThread(
        attr: *const c_void,
        stack: usize,
        f: unsafe extern "system" fn(*mut c_void) -> u32,
        arg: *mut c_void,
        flags: u32,
        id: *mut u32,
    ) -> isize;
    fn Sleep(ms: u32);
}

const N: usize = 4;
const SAMPLES: usize = (RATE as usize / 50) * 2; // 20 ms stereo
const WHDR_DONE: u32 = 1;
const HDR: u32 = size_of::<Hdr>() as u32;

struct Inner {
    h: isize,
    hdr: [Hdr; N],
    buf: [[i16; SAMPLES]; N],
    next: usize, // buffers play in write order, so refill strictly in ring order
}

pub struct Dev(Box<Inner>); // boxed: headers point into `buf`, so it must not move

pub unsafe fn open() -> Option<Dev> {
    let fmt = Fmt {
        tag: 1,
        channels: 2,
        rate: RATE,
        bytes_per_sec: RATE * 4,
        align: 4,
        bits: 16,
        extra: 0,
    };
    let mut s: Box<Inner> = Box::new(zeroed());
    // WAVE_MAPPER = u32::MAX, CALLBACK_NULL = 0
    if waveOutOpen(&mut s.h, u32::MAX, &fmt, 0, 0, 0) != 0 {
        return None;
    }
    for i in 0..N {
        s.hdr[i].data = s.buf[i].as_mut_ptr();
        s.hdr[i].len = (SAMPLES * 2) as u32;
        if waveOutPrepareHeader(s.h, &mut s.hdr[i], HDR) != 0 {
            return None;
        }
        s.hdr[i].flags |= WHDR_DONE; // so the first pump fills it
    }
    Some(Dev(s))
}

impl Dev {
    pub unsafe fn pump(&mut self, mut mix: impl FnMut(&mut [i16])) {
        let s = &mut *self.0;
        while s.hdr[s.next].flags & WHDR_DONE != 0 {
            let i = s.next;
            mix(&mut s.buf[i]);
            waveOutWrite(s.h, &mut s.hdr[i], HDR);
            s.next = (i + 1) % N;
        }
        Sleep(5); // buffers are 20 ms, so this never starves them
    }
}

unsafe extern "system" fn tramp(a: *mut c_void) -> u32 {
    unsafe { crate::thread_main(a) };
    0
}

/// Runs `thread_main(arg)` on a new thread. The thread is never joined.
pub unsafe fn spawn(arg: *mut c_void) -> bool {
    CreateThread(null(), 0, tramp, arg, 0, null_mut()) != 0
}
