//! Feature `fixed_input`: `FixedInput`, input that advances once per fixed step instead of once per frame.
use core::ops::Deref;
use ecs::{App, Schedule};
use platform::input::{MAX_KEYS, Pending, View as PlatformView};

/// Resource: keyboard (and, with `mouse`, mouse motion) for `Fixed` systems, same methods as `Input`, advanced once
/// per fixed step. Input is still sampled every frame; the changes wait here until a step takes them. So a physics
/// step never misses a tap, never sees a press twice, and input stays as precise as the frame rate even at a slow
/// tick. A step takes at most one up/down change per key (a tap shows as held for one step, then released), and
/// `mouse_delta` is all motion since the previous step. Runs only if you added a `Fixed` system.
#[derive(Clone, Copy, Default)]
pub struct FixedInput {
    view: PlatformView,
    pending: Pending,
    #[cfg(feature = "mouse")]
    delta: [f32; 2],
    #[cfg(feature = "mouse")]
    pending_delta: [f32; 2],
}
impl Deref for FixedInput {
    type Target = PlatformView;
    fn deref(&self) -> &PlatformView {
        &self.view
    }
}
impl FixedInput {
    #[cfg(feature = "mouse")]
    pub fn mouse_delta(&self) -> [f32; 2] {
        self.delta
    }
    pub(crate) fn feed(&mut self, changes: &[u8; MAX_KEYS], delta: [f32; 2]) {
        self.pending.add(changes);
        #[cfg(feature = "mouse")]
        {
            self.pending_delta = [
                self.pending_delta[0] + delta[0],
                self.pending_delta[1] + delta[1],
            ];
        }
        #[cfg(not(feature = "mouse"))]
        let _ = delta;
    }
    fn step(&mut self) {
        self.pending.step(&mut self.view);
        #[cfg(feature = "mouse")]
        {
            self.delta = core::mem::take(&mut self.pending_delta);
        }
    }
}

/// Adds the resource and the system that advances it before each fixed step.
pub(crate) fn init(app: &mut App) {
    app.world.insert_resource(FixedInput::default());
    app.add_system(Schedule::PreFixed, |w, _| {
        w.resource_mut::<FixedInput>().unwrap().step()
    });
}
