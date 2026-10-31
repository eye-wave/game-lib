//! `cull` is off: no compute pass, no `visible` buffer. Each group gets its own contiguous slice of the input
//! buffer and every instance is drawn. Same interface as `cull.rs`.
use crate::{
    cams::View,
    gpu::*,
    scene::{CAP, Frame, Inst},
};
use ash::vk;

pub(crate) struct Cull;

impl Cull {
    pub unsafe fn new(_: &Ctx, _: &[Frame; FRAMES]) -> Self {
        Self
    }

    pub fn dst(&self, _n: usize, group: usize, in_group: usize) -> usize {
        group * CAP + in_group - 1
    }

    pub fn instance_count(&self, in_group: usize) -> u32 {
        in_group as u32
    }

    pub fn instances(
        &self,
        _f: usize,
        fr: &Frame,
        _slice: usize,
        group: usize,
    ) -> (vk::Buffer, u64) {
        (fr.input.buf, (group * CAP * size_of::<Inst>()) as u64)
    }

    pub unsafe fn dispatch(&self, _: vk::CommandBuffer, _: usize, _: &[View], _: usize) {}
}
