//! Frame-driven audio: no thread, no callback. `add(app)` once, then `world.resource_mut::<Audio>()`.play(..).
//! Sounds are procedural (`fn(seconds) -> sample in -1..1`), mixed on the CPU into i16 stereo.
//! No audio device (container, no libasound)? Everything silently no-ops.
#![no_std]
#![allow(unsafe_op_in_unsafe_fn)]
extern crate alloc;

use core::f32::consts::TAU;
use ecs::{App, Schedule};
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
    pub fn play(&mut self, s: Sound, vol: f32) {
        if let Some(v) = self.voices.iter_mut().find(|v| v.is_none()) {
            *v = Some(Voice { s, pos: 0, vol });
        }
    }

    /// Start/stop the sine oscillator in `slot` (< OSCS) at `freq` Hz. Cheap to call every frame.
    pub fn tone(&mut self, slot: usize, freq: f32, on: bool) {
        if let Some(o) = self.oscs.get_mut(slot) {
            o.freq = freq;
            o.on = on;
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

/// ECS resource: mixer + optional device.
pub struct Audio {
    mixer: Mixer,
    dev: Option<sys::Dev>,
}

impl Audio {
    pub fn new() -> Self {
        Audio {
            mixer: Mixer::default(),
            dev: unsafe { sys::open() },
        }
    }

    pub fn play(&mut self, s: Sound, vol: f32) {
        if self.dev.is_some() {
            self.mixer.play(s, vol);
        }
    }

    pub fn tone(&mut self, slot: usize, freq: f32, on: bool) {
        if self.dev.is_some() {
            self.mixer.tone(slot, freq, on);
        }
    }

    fn pump(&mut self) {
        if let Some(d) = &mut self.dev {
            unsafe { d.pump(|b| self.mixer.mix(b)) }
        }
    }
}

impl Default for Audio {
    fn default() -> Self {
        Self::new()
    }
}

/// Inserts the `Audio` resource and a per-frame pump system.
pub fn add(app: &mut App) {
    app.world.insert_resource(Audio::new());
    app.add_system(Schedule::Update, |w, _| {
        if let Some(a) = w.resource_mut::<Audio>() {
            a.pump();
        }
    });
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
}
