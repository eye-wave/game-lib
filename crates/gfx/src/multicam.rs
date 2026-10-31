//! Feature `multicam`: every active camera (up to `MAX_CAMERAS`) renders into its own target, `scale` times its
//! viewport's pixel size, which is then blitted into the viewport. `single.rs` is the same interface for one
//! fullscreen camera drawing straight into the output.
use crate::{
    cams::MAX_CAMERAS,
    gpu::*,
    post,
    scene::{Pass, Scene},
};
use ash::vk;
use core::ptr;
use errors::prelude::*;
use vulkan_sys::*;

/// What a camera renders into.
struct CamTarget {
    extent: vk::Extent2D,
    color: Target,
    depth: Target,
}

pub(crate) struct Cams {
    targets: [Option<CamTarget>; MAX_CAMERAS],
}

impl Cams {
    pub fn new() -> Self {
        Self {
            targets: core::array::from_fn(|_| None),
        }
    }

    /// Draws all views and leaves the output image ready for whatever comes next (post pass or present).
    /// The output is only copied into: clear it, then each camera renders to its own target and is blitted in.
    pub unsafe fn render(&mut self, scene: &mut Scene, p: &Pass) {
        let (c, cmd) = (p.c, p.cmd);
        let (ew, eh) = (p.sc.extent.width as f32, p.sc.extent.height as f32);
        let (out, _) = p.sc.out(p.idx);
        transition(
            cmd,
            out,
            vk::ImageAspectFlags::COLOR,
            (
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            ),
            (post::out_wait(), vk::PipelineStageFlags::TRANSFER),
            (vk::AccessFlags::empty(), vk::AccessFlags::TRANSFER_WRITE),
        );
        let black = vk::ClearColorValue {
            float32: [0.0, 0.0, 0.0, 1.0],
        };
        let whole = color_range();
        vkCmdClearColorImage(
            cmd,
            out,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &black,
            1,
            &whole,
        );

        for (k, v) in p.views.iter().enumerate() {
            let ext = vk::Extent2D {
                width: ((v.rect[2] * ew * v.scale) as u32).max(1),
                height: ((v.rect[3] * eh * v.scale) as u32).max(1),
            };
            if self.targets[k].as_ref().is_some_and(|t| t.extent != ext) {
                ck(vkDeviceWaitIdle(c.device), WAIT_FOR_DEVICE_IDLE_FAILED);
                let old = self.targets[k].take().unwrap();
                destroy_target(c, &old.color);
                destroy_target(c, &old.depth);
            }
            let t = self.targets[k].get_or_insert_with(|| CamTarget {
                extent: ext,
                color: make_target(
                    c,
                    ext,
                    SCENE_FORMAT,
                    vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC,
                    vk::ImageAspectFlags::COLOR,
                ),
                depth: make_target(
                    c,
                    ext,
                    DEPTH_FORMAT,
                    vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
                    vk::ImageAspectFlags::DEPTH,
                ),
            });
            transition(
                cmd,
                t.color.image,
                vk::ImageAspectFlags::COLOR,
                (
                    vk::ImageLayout::UNDEFINED,
                    vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                ),
                // waits for the previous frame's blit to finish reading this image
                (
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                ),
                (
                    vk::AccessFlags::empty(),
                    vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
                ),
            );
            transition(
                cmd,
                t.depth.image,
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
            scene.draw(p, t.color.view, t.depth.view, ext, k, v);
            transition(
                cmd,
                t.color.image,
                vk::ImageAspectFlags::COLOR,
                (
                    vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                ),
                (
                    vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                    vk::PipelineStageFlags::TRANSFER,
                ),
                (
                    vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
                    vk::AccessFlags::TRANSFER_READ,
                ),
            );
            // the clear and earlier blits must land before this one (later cameras draw over earlier ones)
            let order = [vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)];
            vkCmdPipelineBarrier(
                cmd,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                1,
                order.as_ptr(),
                0,
                ptr::null(),
                0,
                ptr::null(),
            );
            let layers = vk::ImageSubresourceLayers::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .layer_count(1);
            let at = |x: f32, y: f32, z: i32| vk::Offset3D {
                x: x as i32,
                y: y as i32,
                z,
            };
            let region = [vk::ImageBlit::default()
                .src_subresource(layers)
                .src_offsets([at(0.0, 0.0, 0), at(ext.width as f32, ext.height as f32, 1)])
                .dst_subresource(layers)
                .dst_offsets([
                    at(v.rect[0] * ew, v.rect[1] * eh, 0),
                    at(
                        (v.rect[0] + v.rect[2]) * ew,
                        (v.rect[1] + v.rect[3]) * eh,
                        1,
                    ),
                ])];
            let filter = if v.scale < 1.0 {
                vk::Filter::NEAREST // big, crisp pixels
            } else {
                vk::Filter::LINEAR
            };
            vkCmdBlitImage(
                cmd,
                t.color.image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                out,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                1,
                region.as_ptr(),
                filter,
            );
        }
        post::finish_out(
            cmd,
            out,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::PipelineStageFlags::TRANSFER,
            vk::AccessFlags::TRANSFER_WRITE,
        );
    }
}
