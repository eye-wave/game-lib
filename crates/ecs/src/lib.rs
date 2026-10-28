//! Tiny ECS. All component values live in ONE byte arena; per-type `Vec`s are only indexes.
//!
//! entity ──(comps)──> (TypeId, Handle) ──(slots)──> offset/size/align/type/generation ──> arena bytes
//! type   ──(index)──> Vec<(Entity, Handle)>         (used to drive queries)
#![no_std]
extern crate alloc;

use alloc::{boxed::Box, vec, vec::Vec};
use core::any::TypeId;
use core::marker::PhantomData;
use core::{mem, ptr};

const ALIGN: usize = 16; // max supported component alignment == arena base alignment

#[derive(Clone, Copy)]
#[repr(align(16))]
#[allow(dead_code)] // only used as aligned backing storage
struct Chunk([u8; ALIGN]); // backing unit, guarantees a 16-aligned base pointer

/// Stable allocation id + generation. Survives compaction; stale after the slot is freed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Handle {
    id: u32,
    generation: u32,
}

/// Stable entity id + generation. A despawned entity never matches a reused index.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Entity {
    idx: u32,
    generation: u32,
}

/// Allocation metadata for one component value.
#[derive(Clone, Copy)]
struct Slot {
    offset: usize,
    size: usize,
    align: usize,
    generation: u32,
    ty: TypeId,
    drop: unsafe fn(*mut u8),
    live: bool,
}

unsafe fn drop_t<T>(p: *mut u8) {
    // SAFETY: caller guarantees `p` points at a live, aligned T.
    unsafe { ptr::drop_in_place(p.cast::<T>()) }
}

// ---------------------------------------------------------------- arena

#[derive(Default)]
struct Arena {
    buf: Vec<Chunk>,
    used: usize, // bump pointer in bytes
    slots: Vec<Slot>,
    free: Vec<u32>, // free slot ids
    // Arena holds arbitrary T (maybe !Send, e.g. Rc) as raw bytes: opt out of auto Send/Sync.
    _not_send: PhantomData<*const ()>,
}

impl Arena {
    fn slot(&self, h: Handle) -> Option<&Slot> {
        self.slots
            .get(h.id as usize)
            .filter(|s| s.live && s.generation == h.generation)
    }

    fn base(&mut self) -> *mut u8 {
        self.buf.as_mut_ptr().cast()
    }

    // pure bump allocation; freed holes are only reclaimed by compact().
    // Non-generic on purpose: one copy of this code instead of one per component type.
    #[inline(never)]
    fn alloc(
        &mut self,
        size: usize,
        align: usize,
        ty: TypeId,
        drop: unsafe fn(*mut u8),
    ) -> (Handle, usize) {
        assert!(
            align <= ALIGN,
            "component alignment above {ALIGN} unsupported"
        );
        let offset = self.used.next_multiple_of(align);
        self.used = offset + size;
        let need = self.used.div_ceil(ALIGN);
        if need > self.buf.len() {
            self.buf.resize(need, Chunk([0; ALIGN])); // may move the buffer: fine, we hold &mut self
        }
        let slot = Slot {
            offset,
            size,
            align,
            generation: 0,
            ty,
            drop,
            live: true,
        };
        let h = match self.free.pop() {
            Some(id) => {
                let generation = self.slots[id as usize].generation; // bumped when freed
                self.slots[id as usize] = Slot { generation, ..slot };
                Handle { id, generation }
            }
            None => {
                self.slots.push(slot);
                Handle {
                    id: self.slots.len() as u32 - 1,
                    generation: 0,
                }
            }
        };
        (h, offset)
    }

    fn insert<T: 'static>(&mut self, v: T) -> Handle {
        let (h, offset) = self.alloc(
            mem::size_of::<T>(),
            mem::align_of::<T>(),
            TypeId::of::<T>(),
            drop_t::<T>,
        );
        // SAFETY: offset is aligned for T and offset+size <= buf bytes; nothing live overlaps it.
        unsafe { ptr::write(self.base().add(offset).cast::<T>(), v) };
        h
    }

    /// Byte offset of a live slot of type `ty`.
    fn offset(&self, h: Handle, ty: TypeId) -> Option<usize> {
        Some(self.slot(h).filter(|s| s.ty == ty)?.offset)
    }

    fn get<T: 'static>(&self, h: Handle) -> Option<&T> {
        let off = self.offset(h, TypeId::of::<T>())?;
        // SAFETY: live slot of type T at an aligned in-bounds offset; &self forbids mutation.
        Some(unsafe { &*self.buf.as_ptr().cast::<u8>().add(off).cast::<T>() })
    }

    fn get_mut<T: 'static>(&mut self, h: Handle) -> Option<&mut T> {
        let off = self.offset(h, TypeId::of::<T>())?;
        // SAFETY: as above; &mut self makes this the only access.
        Some(unsafe { &mut *self.base().add(off).cast::<T>() })
    }

    fn release(&mut self, id: u32) {
        let s = &mut self.slots[id as usize];
        s.live = false;
        s.generation = s.generation.wrapping_add(1); // invalidates outstanding handles
        self.free.push(id);
    }

    /// Move the value out and free its slot.
    fn take<T: 'static>(&mut self, h: Handle) -> Option<T> {
        let off = self.offset(h, TypeId::of::<T>())?;
        self.release(h.id);
        // SAFETY: slot was live and held a T; it is now marked dead so it won't be read/dropped again.
        Some(unsafe { ptr::read(self.base().add(off).cast::<T>()) })
    }

    /// Drop the value in place (type-erased) and free its slot.
    fn drop_slot(&mut self, h: Handle) {
        let Some(s) = self.slot(h) else { return };
        let (off, f) = (s.offset, s.drop);
        self.release(h.id); // mark dead first so a panicking Drop can't cause a double drop
        // SAFETY: slot was live; `f` is drop_t::<T> for the stored T.
        unsafe { f(self.base().add(off)) }
    }

    /// Slide live values down, closing holes. Handles stay valid (they name slots, not offsets).
    /// Rust moves are bitwise, so memmove-ing a value and forgetting the old bytes is a legal move.
    fn compact(&mut self) {
        let mut ids: Vec<u32> = (0..self.slots.len() as u32)
            .filter(|&i| self.slots[i as usize].live)
            .collect();
        ids.sort_by_key(|&i| self.slots[i as usize].offset);
        let base = self.base();
        let mut cur = 0usize;
        for i in ids {
            let s = &mut self.slots[i as usize];
            let off = cur.next_multiple_of(s.align);
            // off <= s.offset: s.offset is aligned and >= end of previous value (>= cur).
            // SAFETY: both ranges in bounds; ptr::copy handles overlap; old bytes are never used again.
            unsafe { ptr::copy(base.add(s.offset), base.add(off), s.size) };
            s.offset = off;
            cur = off + s.size;
        }
        self.used = cur;
        self.buf.truncate(cur.div_ceil(ALIGN));
        self.buf.shrink_to_fit();
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        let base = self.base();
        for s in self.slots.iter().filter(|s| s.live) {
            // SAFETY: live slot holds an initialised value of the type `drop` was built for.
            unsafe { (s.drop)(base.add(s.offset)) }
        }
    }
}

// ---------------------------------------------------------------- world

struct EntitySlot {
    generation: u32,
    comps: Vec<(TypeId, Handle)>, // entity -> component index
}

#[derive(Default)]
pub struct World {
    ents: Vec<EntitySlot>,
    free: Vec<u32>,
    arena: Arena,
    // Dynamic registry: TypeId -> index of (owner, handle). No component types hardcoded.
    // linear scan over a Vec of pairs (no_std has no HashMap, BTreeMap costs ~6KB of code);
    // swap in a map if you ever have hundreds of component types.
    index: Vec<(TypeId, Vec<(Entity, Handle)>)>,
    res: Option<Entity>, // hidden entity that owns all resources
}

pub fn id<T: 'static>() -> TypeId {
    TypeId::of::<T>()
}

/// A group of components, written as a tuple: `(Pos, Vel)`. Used both to insert several
/// components at once (a value tuple) and to name a query (the type tuple).
pub trait Bundle {
    fn insert_into(self, w: &mut World, e: Entity);
    fn type_ids() -> Vec<TypeId>;
}

macro_rules! bundle {
    ($($T:ident),+) => {
        impl<$($T: 'static),+> Bundle for ($($T,)+) {
            #[allow(non_snake_case)]
            fn insert_into(self, w: &mut World, e: Entity) {
                let ($($T,)+) = self;
                $(w.insert(e, $T);)+
            }
            fn type_ids() -> Vec<TypeId> {
                vec![$(id::<$T>()),+]
            }
        }
    };
}
// tuples up to 8; extend the list if you need more.
bundle!(A);
bundle!(A, B);
bundle!(A, B, C);
bundle!(A, B, C, D);
bundle!(A, B, C, D, E);
bundle!(A, B, C, D, E, F);
bundle!(A, B, C, D, E, F, G);
bundle!(A, B, C, D, E, F, G, H);

impl World {
    pub fn spawn(&mut self) -> Entity {
        match self.free.pop() {
            Some(idx) => Entity {
                idx,
                generation: self.ents[idx as usize].generation,
            },
            None => {
                self.ents.push(EntitySlot {
                    generation: 0,
                    comps: Vec::new(),
                });
                Entity {
                    idx: self.ents.len() as u32 - 1,
                    generation: 0,
                }
            }
        }
    }

    /// Spawn an entity with a group of components: `w.spawn_with((Pos{..}, Vel{..}))`.
    pub fn spawn_with<B: Bundle>(&mut self, b: B) -> Entity {
        let e = self.spawn();
        b.insert_into(self, e);
        e
    }

    /// Add a group of components to an existing entity (overwrites ones it already has).
    pub fn insert_bundle<B: Bundle>(&mut self, e: Entity, b: B) -> bool {
        let ok = self.alive(e);
        if ok {
            b.insert_into(self, e);
        }
        ok
    }

    /// Entities having every component in the tuple: `w.query_of::<(Pos, Vel)>()`.
    pub fn query_of<B: Bundle>(&self) -> Vec<Entity> {
        self.query(&B::type_ids())
    }

    /// Singleton values (one per type), stored as components of a hidden entity.
    pub fn insert_resource<T: 'static>(&mut self, v: T) {
        let e = *self.res.get_or_insert_with(|| {
            self.ents.push(EntitySlot {
                generation: 0,
                comps: Vec::new(),
            });
            Entity {
                idx: self.ents.len() as u32 - 1,
                generation: 0,
            }
        });
        self.insert(e, v);
    }

    pub fn resource<T: 'static>(&self) -> Option<&T> {
        self.get(self.res?)
    }

    pub fn resource_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.get_mut(self.res?)
    }

    pub fn alive(&self, e: Entity) -> bool {
        self.ents
            .get(e.idx as usize)
            .is_some_and(|s| s.generation == e.generation)
    }

    /// Drops all of the entity's components. Returns false if it was already dead.
    pub fn despawn(&mut self, e: Entity) -> bool {
        if !self.alive(e) {
            return false;
        }
        for (ty, h) in mem::take(&mut self.ents[e.idx as usize].comps) {
            self.arena.drop_slot(h);
            self.unindex(e, ty);
        }
        let s = &mut self.ents[e.idx as usize];
        s.generation = s.generation.wrapping_add(1);
        self.free.push(e.idx);
        true
    }

    /// Adds T, or overwrites the existing T. Registers T's type on first use.
    /// Returns false (dropping `v`) if the entity is dead.
    pub fn insert<T: 'static>(&mut self, e: Entity, v: T) -> bool {
        if !self.alive(e) {
            return false;
        }
        if let Some(h) = self.handle(e, id::<T>()) {
            *self.arena.get_mut::<T>(h).unwrap() = v; // drops the old value
            return true;
        }
        let h = self.arena.insert(v);
        self.attach(e, id::<T>(), h);
        true
    }

    #[inline(never)]
    fn attach(&mut self, e: Entity, ty: TypeId, h: Handle) {
        let i = match self.index.iter().position(|x| x.0 == ty) {
            Some(i) => i,
            None => {
                self.index.push((ty, Vec::new()));
                self.index.len() - 1
            }
        };
        self.index[i].1.push((e, h));
        self.ents[e.idx as usize].comps.push((ty, h));
    }

    pub fn remove<T: 'static>(&mut self, e: Entity) -> Option<T> {
        let v = self.arena.take::<T>(self.handle(e, id::<T>())?)?;
        self.detach(e, id::<T>());
        Some(v)
    }

    pub fn get<T: 'static>(&self, e: Entity) -> Option<&T> {
        self.arena.get(self.handle(e, id::<T>())?)
    }

    pub fn get_mut<T: 'static>(&mut self, e: Entity) -> Option<&mut T> {
        let h = self.handle(e, id::<T>())?;
        self.arena.get_mut(h)
    }

    /// Entities having ALL of `types`. Walks the smallest type index, checks the rest via the
    /// entity's own component list. Returns a Vec so callers may mutate the world while looping.
    /// empty `types` yields nothing; make it a lazy iterator if allocation shows up.
    pub fn query(&self, types: &[TypeId]) -> Vec<Entity> {
        let mut lists = Vec::with_capacity(types.len());
        for t in types {
            match self.index.iter().find(|x| x.0 == *t) {
                Some(l) => lists.push(&l.1),
                None => return Vec::new(), // type never registered / no owners
            }
        }
        let Some(driver) = lists.into_iter().min_by_key(|l| l.len()) else {
            return Vec::new();
        };
        driver
            .iter()
            .map(|&(e, _)| e)
            .filter(|e| {
                let c = &self.ents[e.idx as usize].comps;
                types.iter().all(|t| c.iter().any(|x| x.0 == *t))
            })
            .collect()
    }

    /// Safe point to call between frames: closes arena holes. All Entity/Handle ids stay valid.
    pub fn compact(&mut self) {
        self.arena.compact();
    }

    pub fn arena_bytes(&self) -> usize {
        self.arena.used
    }

    #[inline(never)]
    fn detach(&mut self, e: Entity, ty: TypeId) {
        self.ents[e.idx as usize].comps.retain(|c| c.0 != ty);
        self.unindex(e, ty);
    }

    #[inline(never)]
    fn handle(&self, e: Entity, ty: TypeId) -> Option<Handle> {
        let s = self
            .ents
            .get(e.idx as usize)
            .filter(|s| s.generation == e.generation)?;
        s.comps.iter().find(|c| c.0 == ty).map(|c| c.1)
    }

    // O(n) position scan per removal; store the index position if removal gets hot.
    fn unindex(&mut self, e: Entity, ty: TypeId) {
        let v = &mut self
            .index
            .iter_mut()
            .find(|x| x.0 == ty)
            .expect("registered")
            .1;
        let i = v.iter().position(|x| x.0 == e).expect("indexed");
        v.swap_remove(i);
    }
}

// ---------------------------------------------------------------- schedules

/// A system is any closure over the world plus a delta time in seconds.
/// dt is 0 for startup, the fixed step for fixed systems, the frame time for update systems.
type System = Box<dyn FnMut(&mut World, f32)>;

const MAX_CATCHUP: u32 = 8; // fixed steps per frame, so a long stall can't spiral

/// Owns the World and three schedules. Systems run in the order they were added.
///   startup: once, before the first frame    fixed: 0..n times per frame at `tick_hz`    update: once per frame
pub struct App {
    pub world: World,
    startup: Vec<System>,
    fixed: Vec<System>,
    update: Vec<System>,
    step: f32, // seconds per fixed tick
    acc: f32,
    started: bool,
}

impl App {
    pub fn new(tick_hz: f32) -> Self {
        let mut app = App {
            world: World::default(),
            startup: vec![],
            fixed: vec![],
            update: vec![],
            step: 0.0,
            acc: 0.0,
            started: false,
        };
        app.set_tick_rate(tick_hz);
        app
    }

    /// Change the fixed tick rate at any time; leftover accumulated time carries over.
    pub fn set_tick_rate(&mut self, hz: f32) {
        assert!(hz > 0.0, "tick rate must be positive");
        self.step = 1.0 / hz;
    }

    pub fn add_startup(&mut self, f: impl FnMut(&mut World, f32) + 'static) -> &mut Self {
        self.startup.push(Box::new(f));
        self
    }

    pub fn add_fixed(&mut self, f: impl FnMut(&mut World, f32) + 'static) -> &mut Self {
        self.fixed.push(Box::new(f));
        self
    }

    pub fn add_update(&mut self, f: impl FnMut(&mut World, f32) + 'static) -> &mut Self {
        self.update.push(Box::new(f));
        self
    }

    /// Advance one frame of `dt` seconds. You own the loop (and the clock); call this from it.
    /// no built-in real-time loop; wrap this in `Instant`/sleep when you need one.
    pub fn run_frame(&mut self, dt: f32) {
        if !self.started {
            self.started = true;
            for s in &mut self.startup {
                s(&mut self.world, 0.0);
            }
        }
        self.acc += dt;
        let mut n = 0;
        while self.acc >= self.step && n < MAX_CATCHUP {
            for s in &mut self.fixed {
                s(&mut self.world, self.step);
            }
            self.acc -= self.step;
            n += 1;
        }
        if self.acc >= self.step {
            self.acc = 0.0; // hit the cap: drop the backlog instead of chasing it
        }
        for s in &mut self.update {
            s(&mut self.world, dt);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_query_compact_despawn() {
        let mut w = World::default();
        let a = w.spawn_with((1u32, 2.0f32));
        let b = w.spawn_with((3u32,));
        assert_eq!(w.query_of::<(u32, f32)>(), vec![a]);
        assert_eq!(w.query_of::<(u32,)>().len(), 2);
        struct Marker; // zero-sized components must work too
        let m = w.spawn_with((Marker, 5u32));
        assert_eq!(w.query_of::<(Marker, u32)>(), vec![m]);
        w.insert_resource(7i64);
        *w.resource_mut::<i64>().unwrap() += 1;
        assert_eq!(w.resource::<i64>(), Some(&8));
        assert_eq!(w.query_of::<(i64,)>().len(), 1);
        w.despawn(a);
        w.compact();
        assert_eq!(w.get::<u32>(b), Some(&3));
        assert!(!w.alive(a) && w.get::<u32>(a).is_none());
    }
}
