//! `multicam` is off: one fullscreen camera draws straight into the output image. No per-camera targets, no
//! blit, no clear pass; `Camera*::viewport` and `scale` are ignored. Same interface as `multicam.rs`.
use crate::{
    gpu::*,
    post,
    scene::{Pass, Scene},
};
use ash::vk;

pub(crate) struct Cams;

impl Cams {
    pub fn new() -> Self {
        Self
    }

    /// Draws the view and leaves the output image ready for whatever comes next (post pass or present).
    pub unsafe fn render(&mut self, scene: &mut Scene, p: &Pass) {
        let cmd = p.cmd;
        let (out, out_view) = p.sc.out(p.idx);
        transition(
            cmd,
            out,
            vk::ImageAspectFlags::COLOR,
            (
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            ),
            (
                post::out_wait(),
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            ),
            (
                vk::AccessFlags::empty(),
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            ),
        );
        transition(
            cmd,
            p.sc.depth.image,
            vk::ImageAspectFlags::DEPTH,
            (
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL,
            ),
            (
                vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
                vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
            ),
            (
                vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
                vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_READ
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
            ),
        );
        for (k, v) in p.views.iter().enumerate() {
            scene.draw(p, out_view, p.sc.depth.view, p.sc.extent, k, v);
        }
        post::finish_out(
            cmd,
            out,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
        );
    }
}
