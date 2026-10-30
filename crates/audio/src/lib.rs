//! Audio on its own thread. The game side only pushes commands into a lock-free queue; the thread owns the
//! mixer and the device, and blocks in the device write (ALSA) or sleeps (winmm) to pace itself.
//! `add(app)` once, then `world.resource_mut::<Audio>().unwrap().play(..)` / `.tone(..)`.
//! Sounds are procedural (`fn(seconds) -> sample in -1..1`), mixed on the CPU into i16 stereo.
//! No audio device (container, no libasound)? No thread is started and everything silently no-ops.
//! Debug builds: the audio thread aborts (`ALLOC_IN_AUDIO_THREAD`) if it ever touches the heap.
#![no_std]
#![allow(unsafe_op_in_unsafe_fn)]
extern crate alloc;

use alloc::boxed::Box;
use core::{
    cell::UnsafeCell,
    f32::consts::TAU,
    ffi::c_void,
    sync::atomic::{
        AtomicUsize,
        Ordering::{Acquire, Relaxed, Release},
    },
};
use ecs::App;
use math::FloatExt;

#[cfg_attr(unix, path = "linux.rs")]
#[cfg_attr(windows, path = "windows.rs")]
mod sys;

pub const RATE: u32 = 48_000;
const VOICES: usize = 16;
pub const OSCS: usize = 16;
const RAMP: f32 = 1.0 / (RATE as f32 * 0.01); // 10 ms attack/release, no clicks
const OSC_VOL: f32 = 0.2;

#[derive(Clone, Copy)]
pub struct Sound {
    pub secs: f32,
    pub f: fn(f32) -> f32,
}

/// Example sound: 880 Hz blip with linear decay (the decay also avoids end clicks).
pub const BLIP: Sound = Sound {
    secs: 0.15,
    f: |t| (t * 880.0 * TAU).sin() * (1.0 - t / 0.15),
};

#[derive(Clone, Copy)]
struct Voice {
    s: Sound,
    pos: u32,
    vol: f32,
}

/// Held sine oscillator: sounds while `on`, fades out after.
#[derive(Clone, Copy, Default)]
struct Osc {
    freq: f32,
    on: bool,
    phase: f32, // 0..1
    amp: f32,
}

#[derive(Default)]
pub struct Mixer {
    voices: [Option<Voice>; VOICES],
    oscs: [Osc; OSCS],
}

impl Mixer {
    // all voices busy => new sound is dropped; add stealing if that is audible.
    fn play(&mut self, s: Sound, vol: f32) {
        if let Some(v) = self.voices.iter_mut().find(|v| v.is_none()) {
            *v = Some(Voice { s, pos: 0, vol });
        }
    }

    fn tone(&mut self, slot: usize, freq: f32, on: bool) {
        if let Some(o) = self.oscs.get_mut(slot) {
            o.freq = freq;
            o.on = on;
        }
    }

    fn apply(&mut self, c: Cmd) {
        match c {
            Cmd::Play(s, vol) => self.play(s, vol),
            Cmd::Tone(slot, freq, on) => self.tone(slot as usize, freq, on),
        }
    }

    /// Overwrites `out` (interleaved stereo i16) with the sum of all voices.
    pub fn mix(&mut self, out: &mut [i16]) {
        for fr in out.as_chunks_mut::<2>().0 {
            let mut sum = 0.0;
            for slot in &mut self.voices {
                let Some(v) = slot else { continue };
                sum += (v.s.f)(v.pos as f32 / RATE as f32) * v.vol;
                v.pos += 1;
                if v.pos as f32 >= v.s.secs * RATE as f32 {
                    *slot = None;
                }
            }
            for o in &mut self.oscs {
                let target = if o.on { 1.0 } else { 0.0 };
                o.amp += (target - o.amp).clamp(-RAMP, RAMP);
                if o.amp == 0.0 {
                    continue;
                }
                sum += (o.phase * TAU).sin() * o.amp * OSC_VOL;
                o.phase += o.freq / RATE as f32;
                if o.phase >= 1.0 {
                    o.phase -= 1.0;
                }
            }
            fr[0] = (sum.clamp(-1.0, 1.0) * 32767.0) as i16;
            fr[1] = fr[0];
        }
    }
}

// ---------------------------------------------------------------- game thread -> audio thread

#[derive(Clone, Copy)]
enum Cmd {
    Play(Sound, f32),
    Tone(u8, f32, bool),
}

const QN: usize = 64; // power of two: the wrapping counters stay consistent with `% QN`

/// Single producer (game thread), single consumer (audio thread).
struct Queue {
    buf: [UnsafeCell<Cmd>; QN],
    head: AtomicUsize, // next slot to write
    tail: AtomicUsize, // next slot to read
}
unsafe impl Sync for Queue {}

impl Queue {
    /// false = full, command dropped.
    fn push(&self, c: Cmd) -> bool {
        let h = self.head.load(Relaxed);
        if h.wrapping_sub(self.tail.load(Acquire)) == QN {
            return false;
        }
        unsafe { *self.buf[h % QN].get() = c };
        self.head.store(h.wrapping_add(1), Release);
        true
    }

    fn pop(&self) -> Option<Cmd> {
        let t = self.tail.load(Relaxed);
        if t == self.head.load(Acquire) {
            return None;
        }
        let c = unsafe { *self.buf[t % QN].get() };
        self.tail.store(t.wrapping_add(1), Release);
        Some(c)
    }
}

/// Everything the audio thread owns. Leaked on purpose: it lives until the process exits.
struct Thread {
    dev: sys::Dev,
    mixer: Mixer,
    q: &'static Queue,
}

/// Entry point of the audio thread, called by the backend's thread trampoline.
unsafe fn thread_main(p: *mut c_void) {
    let t = &mut *p.cast::<Thread>();
    #[cfg(debug_assertions)]
    platform::NO_ALLOC.store(platform::thread_id(), Relaxed); // from here on, any heap use aborts
    loop {
        while let Some(c) = t.q.pop() {
            t.mixer.apply(c);
        }
        t.dev.pump(|b| t.mixer.mix(b));
    }
}

/// ECS resource: the game-side handle.
pub struct Audio {
    q: Option<&'static Queue>,
    last: [(f32, bool); OSCS], // what the thread was last told, so `tone` can be called every frame
}

impl Audio {
    pub fn new() -> Self {
        let q = unsafe { sys::open() }.and_then(|dev| {
            let q: &'static Queue = Box::leak(Box::new(Queue {
                buf: [const { UnsafeCell::new(Cmd::Tone(0, 0.0, false)) }; QN],
                head: AtomicUsize::new(0),
                tail: AtomicUsize::new(0),
            }));
            let t = Box::leak(Box::new(Thread {
                dev,
                mixer: Mixer::default(),
                q,
            }));
            unsafe { sys::spawn((t as *mut Thread).cast()) }.then_some(q)
        });
        Audio {
            q,
            last: [(0.0, false); OSCS],
        }
    }

    // a full queue (64 pending commands) drops a play; tone retries next frame.
    pub fn play(&mut self, s: Sound, vol: f32) {
        if let Some(q) = self.q {
            q.push(Cmd::Play(s, vol));
        }
    }

    /// Start/stop the sine oscillator in `slot` (< OSCS) at `freq` Hz. Cheap to call every frame.
    pub fn tone(&mut self, slot: usize, freq: f32, on: bool) {
        let Some(q) = self.q else { return };
        if slot < OSCS && self.last[slot] != (freq, on) && q.push(Cmd::Tone(slot as u8, freq, on)) {
            self.last[slot] = (freq, on);
        }
    }
}

impl Default for Audio {
    fn default() -> Self {
        Self::new()
    }
}

/// Inserts the `Audio` resource (and starts the thread).
pub fn add(app: &mut App) {
    app.world.insert_resource(Audio::new());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn voice_plays_then_frees() {
        let mut m = Mixer::default();
        m.play(BLIP, 1.0);
        let mut b = [0i16; 2 * 4800]; // 100 ms
        m.mix(&mut b);
        assert!(b.iter().any(|&s| s.abs() > 1000));
        let mut b = [1i16; 2 * 4800]; // blip is 150 ms: ends inside this block
        m.mix(&mut b);
        assert!(m.voices.iter().all(|v| v.is_none()));
        m.mix(&mut b);
        assert!(b.iter().all(|&s| s == 0)); // overwritten with silence
    }

    #[test]
    fn tone_holds_then_fades() {
        let mut m = Mixer::default();
        m.tone(0, 440.0, true);
        let mut b = [0i16; 2 * 4800];
        m.mix(&mut b);
        assert!(b[2 * 2400..].iter().any(|&s| s.abs() > 3000)); // full level after the ramp
        m.tone(0, 440.0, false);
        m.mix(&mut b);
        m.mix(&mut b);
        assert!(b.iter().all(|&s| s == 0));
    }

    #[test]
    fn queue_is_fifo_and_bounded() {
        let q = Queue {
            buf: [const { UnsafeCell::new(Cmd::Tone(0, 0.0, false)) }; QN],
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        };
        for i in 0..QN {
            assert!(q.push(Cmd::Tone(i as u8, 0.0, true)));
        }
        assert!(!q.push(Cmd::Tone(0, 0.0, true))); // full
        for i in 0..QN {
            assert!(matches!(q.pop(), Some(Cmd::Tone(n, ..)) if n as usize == i));
        }
        assert!(q.pop().is_none());
        assert!(q.push(Cmd::Tone(7, 0.0, true))); // space again
    }
}
