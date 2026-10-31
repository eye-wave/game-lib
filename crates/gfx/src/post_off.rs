//! `post` is off: no offscreen scene image, no sampler, no post shader. The cameras write the swapchain image
//! itself, which `finish_out` hands to the presentation engine. Same interface as `post.rs`.
use crate::{gpu::*, scene::Pass, swapchain::Swapchain};
use ash::vk;

pub(crate) struct Post;

/// The swapchain image is first written once the acquire semaphore lets it.
pub(crate) fn out_wait() -> vk::PipelineStageFlags {
    acquire_stages()
}

/// Hands the finished swapchain image to the presentation engine.
pub(crate) unsafe fn finish_out(
    cmd: vk::CommandBuffer,
    out: vk::Image,
    old: vk::ImageLayout,
    src_stage: vk::PipelineStageFlags,
    src_access: vk::AccessFlags,
) {
    transition(
        cmd,
        out,
        vk::ImageAspectFlags::COLOR,
        (old, vk::ImageLayout::PRESENT_SRC_KHR),
        (src_stage, vk::PipelineStageFlags::BOTTOM_OF_PIPE),
        (src_access, vk::AccessFlags::empty()),
    );
}

impl Post {
    pub unsafe fn new(_: &Ctx, _: &Swapchain) -> Self {
        Self
    }
    pub unsafe fn resize(&self, _: &Ctx, _: &Swapchain) {}
    pub unsafe fn record(&mut self, _: &Pass, _: f32) {}
}
