//! Feature `post`: the cameras draw into an offscreen image, and a fullscreen pass (`post.wgsl`, or the shader in the
//! `PostProcess` resource) writes it to the swapchain. `post_off.rs` is the same interface without it.
use crate::{gpu::*, scene::Pass, swapchain::Swapchain};
use ash::vk;
use core::ptr;
use errors::prelude::*;
use shader::Align4;
use vulkan_sys::*;

// the default pass: passthrough
static DEFAULT: &Align4<[u8]> = shader::include_shader!("post");

/// Resource: the fullscreen post-processing shader (contract in `crates/gfx/src/post.wgsl`).
/// `None` is the default pass: a passthrough (a custom shader replaces it, so it brings its own). It is built the first time it is seen, so set it any time before drawing.
/// `.1` reaches the shader as `params.w` (a loading bar's progress, say); it may change every frame.
#[derive(Clone, Copy, Default)]
pub struct PostProcess(pub Option<&'static Align4<[u8]>>, pub f32);

pub(crate) struct Post {
    layout: vk::PipelineLayout,
    set: vk::DescriptorSet,
    sampler: vk::Sampler,
    pipe: Option<(&'static Align4<[u8]>, vk::Pipeline)>,
}

/// The scene image is only read by the post pass; before the cameras write it again, wait for that read.
pub(crate) fn out_wait() -> vk::PipelineStageFlags {
    vk::PipelineStageFlags::FRAGMENT_SHADER
}

/// Hands the finished scene image to the post pass.
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
        (old, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL),
        (src_stage, vk::PipelineStageFlags::FRAGMENT_SHADER),
        (src_access, vk::AccessFlags::SHADER_READ),
    );
}

impl Post {
    pub unsafe fn new(c: &Ctx, sc: &Swapchain) -> Self {
        let device = c.device;
        // the scene color is sampled by a fullscreen pass into the swapchain
        let binds = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        ];
        let set_layout: vk::DescriptorSetLayout = mk(CREATE_DESCRIPTOR_SET_LAYOUT_FAILED, |p| {
            vkCreateDescriptorSetLayout(
                device,
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&binds),
                NA,
                p,
            )
        });
        let set_layouts = [set_layout];
        let push = [vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::FRAGMENT)
            .size(size_of::<[f32; 4]>() as u32)];
        let layout: vk::PipelineLayout = mk(CREATE_PIPELINE_LAYOUT_FAILED, |p| {
            vkCreatePipelineLayout(
                device,
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(&set_layouts)
                    .push_constant_ranges(&push),
                NA,
                p,
            )
        });
        let sampler: vk::Sampler = mk(CREATE_SAMPLER_FAILED, |p| {
            vkCreateSampler(
                device,
                &vk::SamplerCreateInfo::default()
                    .mag_filter(vk::Filter::LINEAR)
                    .min_filter(vk::Filter::LINEAR)
                    .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
                NA,
                p,
            )
        });
        let pool_sizes = [
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLER)
                .descriptor_count(1),
        ];
        let dpool: vk::DescriptorPool = mk(CREATE_DESCRIPTOR_POOL_FAILED, |p| {
            vkCreateDescriptorPool(
                device,
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(1)
                    .pool_sizes(&pool_sizes),
                NA,
                p,
            )
        });
        let mut set = vk::DescriptorSet::default();
        ck(
            vkAllocateDescriptorSets(
                device,
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(dpool)
                    .set_layouts(&set_layouts),
                &mut set,
            ),
            ALLOCATE_DESCRIPTOR_SETS_FAILED,
        );
        let post = Self {
            layout,
            set,
            sampler,
            pipe: None,
        };
        post.resize(c, sc);
        post
    }

    /// Point the descriptor set at the current scene image (call after every swapchain recreation).
    pub unsafe fn resize(&self, c: &Ctx, sc: &Swapchain) {
        let image = [vk::DescriptorImageInfo::default()
            .image_view(sc.scene.view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
        let samp = [vk::DescriptorImageInfo::default().sampler(self.sampler)];
        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(self.set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&image),
            vk::WriteDescriptorSet::default()
                .dst_set(self.set)
                .dst_binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .image_info(&samp),
        ];
        vkUpdateDescriptorSets(c.device, 2, writes.as_ptr(), 0, ptr::null());
    }

    /// The post pass: scene -> swapchain image, which ends in the present layout. `time` is seconds since start.
    pub unsafe fn record(&mut self, p: &Pass, time: f32) {
        let (cmd, sc) = (p.cmd, p.sc);
        let (ew, eh) = (sc.extent.width as f32, sc.extent.height as f32);
        let area = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: sc.extent,
        };
        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: ew,
            height: eh,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        transition(
            cmd,
            sc.images[p.idx],
            vk::ImageAspectFlags::COLOR,
            (
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            ),
            (
                acquire_stages(),
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            ),
            (
                vk::AccessFlags::empty(),
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            ),
        );
        let target = [vk::RenderingAttachmentInfo::default()
            .image_view(sc.views[p.idx])
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::DONT_CARE) // the fullscreen triangle covers everything
            .store_op(vk::AttachmentStoreOp::STORE)];
        vkCmdBeginRendering(
            cmd,
            &vk::RenderingInfo::default()
                .render_area(area)
                .layer_count(1)
                .color_attachments(&target),
        );
        vkCmdSetViewport(cmd, 0, 1, &viewport);
        vkCmdSetScissor(cmd, 0, 1, &area);
        let PostProcess(custom, extra) = *p.app.world.resource::<PostProcess>().unwrap();
        let shader = custom.unwrap_or(DEFAULT);
        // swapping the shader at runtime leaks the old pipeline.
        let pipe = match self.pipe {
            Some((s, pl)) if ptr::eq(s, shader) => pl,
            _ => {
                // no vertex buffers, no depth
                let pl = build_pipeline(
                    p.c,
                    self.layout,
                    shader,
                    sc.format,
                    false,
                    vk::SampleCountFlags::TYPE_1,
                    &vk::PipelineVertexInputStateCreateInfo::default(),
                );
                self.pipe = Some((shader, pl));
                pl
            }
        };
        vkCmdBindPipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pipe);
        vkCmdBindDescriptorSets(
            cmd,
            vk::PipelineBindPoint::GRAPHICS,
            self.layout,
            0,
            1,
            &self.set,
            0,
            ptr::null(),
        );
        let params = [ew, eh, time, extra];
        vkCmdPushConstants(
            cmd,
            self.layout,
            vk::ShaderStageFlags::FRAGMENT,
            0,
            size_of::<[f32; 4]>() as u32,
            params.as_ptr().cast(),
        );
        vkCmdDraw(cmd, 3, 1, 0, 0);
        vkCmdEndRendering(cmd);
        transition(
            cmd,
            sc.images[p.idx],
            vk::ImageAspectFlags::COLOR,
            (
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                vk::ImageLayout::PRESENT_SRC_KHR,
            ),
            (
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            ),
            (
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
                vk::AccessFlags::empty(),
            ),
        );
    }
}
