//! Feature `text`: `Atlas` (the MSDF texture), `Material::msdf`, and the one descriptor set that binds the atlas.
//! `text_off.rs` is the same interface without it.
use crate::{assets::Material, gpu::*};
use ash::vk;
use core::{ptr, slice};
use ecs::App;
use errors::prelude::*;
use platform::die;
use shader::Align4;
use vulkan_sys::*;

static MSDF: &Align4<[u8]> = shader::include_shader!("msdf");

/// Resource: the glyph atlas of the `font` crate (`Font::{w, h, rgb}`, or `font::alpha_atlas` for a bitmap font),
/// RGB8, top row first. Insert it before the
/// first frame that draws a `Material::msdf()`; it is uploaded once and later changes are ignored.
pub struct Atlas {
    pub w: u32,
    pub h: u32,
    pub rgb: &'static [u8],
}

impl Material {
    /// Text: the mesh is `font::Font::layout`'s `[x, y, z, u, v]` vertices, filled with the entity's `Color`. Same
    /// transform rules as `Material::flat` (so it pairs with `Camera2d`; z is the layer). Needs the `Atlas` resource.
    /// Edges are one pixel wide in alpha (`fwidth`), which feature `aa` turns into MSAA coverage; without `aa` they
    /// are cut at the 0.5 contour, crisp but jagged.
    pub fn msdf() -> Self {
        Self {
            shader: MSDF,
            stride: size_of::<[f32; 5]>() as u32,
            attrs: &[vk::Format::R32G32B32_SFLOAT, vk::Format::R32G32_SFLOAT],
        }
    }
}

pub(crate) struct Text {
    layout: vk::DescriptorSetLayout,
    sampler: vk::Sampler,
    set: vk::DescriptorSet,
    ready: bool, // the atlas is uploaded and written into `set`
}

impl Text {
    pub unsafe fn new(c: &Ctx) -> Self {
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
        let layout: vk::DescriptorSetLayout = mk(CREATE_DESCRIPTOR_SET_LAYOUT_FAILED, |p| {
            vkCreateDescriptorSetLayout(
                c.device,
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&binds),
                NA,
                p,
            )
        });
        let sampler: vk::Sampler = mk(CREATE_SAMPLER_FAILED, |p| {
            vkCreateSampler(
                c.device,
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
        let pool: vk::DescriptorPool = mk(CREATE_DESCRIPTOR_POOL_FAILED, |p| {
            vkCreateDescriptorPool(
                c.device,
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
                c.device,
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(pool)
                    .set_layouts(slice::from_ref(&layout)),
                &mut set,
            ),
            ALLOCATE_DESCRIPTOR_SETS_FAILED,
        );
        Self {
            layout,
            sampler,
            set,
            ready: false,
        }
    }

    pub fn layouts(&self) -> &[vk::DescriptorSetLayout] {
        slice::from_ref(&self.layout)
    }

    /// Uploads the `Atlas` the first time it exists. Records into `cmd`, outside any render pass.
    pub unsafe fn record(&mut self, c: &Ctx, cmd: vk::CommandBuffer, app: &App) {
        if self.ready {
            return;
        }
        let Some(a) = app.world.resource::<Atlas>() else {
            return;
        };
        let n = (a.w * a.h) as usize;
        if a.rgb.len() < n * 3 {
            die(FONT_ATLAS_MISSING_OR_TOO_SMALL);
        }
        // RGB8 is not a format every GPU can sample, so pad to RGBA. The staging buffer is never freed, like
        // every buffer here; it is atlas-sized, once.
        let stage = make_buffer(c, n as u64 * 4, vk::BufferUsageFlags::TRANSFER_SRC, true);
        for i in 0..n {
            let px = stage.ptr.add(i * 4);
            ptr::copy_nonoverlapping(a.rgb.as_ptr().add(i * 3), px, 3);
            px.add(3).write(255);
        }
        let extent = vk::Extent2D {
            width: a.w,
            height: a.h,
        };
        // UNORM, not SRGB: distances are data, not colors
        let img = make_target(
            c,
            extent,
            vk::Format::R8G8B8A8_UNORM,
            vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
            vk::ImageAspectFlags::COLOR,
        );
        let color = vk::ImageAspectFlags::COLOR;
        transition(
            cmd,
            img.image,
            color,
            (
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            ),
            (
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
            ),
            (vk::AccessFlags::empty(), vk::AccessFlags::TRANSFER_WRITE),
        );
        let copy = vk::BufferImageCopy::default()
            .image_subresource(
                vk::ImageSubresourceLayers::default()
                    .aspect_mask(color)
                    .layer_count(1),
            )
            .image_extent(vk::Extent3D {
                width: a.w,
                height: a.h,
                depth: 1,
            });
        vkCmdCopyBufferToImage(
            cmd,
            stage.buf,
            img.image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            1,
            &copy,
        );
        transition(
            cmd,
            img.image,
            color,
            (
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            ),
            (
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
            ),
            (
                vk::AccessFlags::TRANSFER_WRITE,
                vk::AccessFlags::SHADER_READ,
            ),
        );
        let image = [vk::DescriptorImageInfo::default()
            .image_view(img.view)
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
        self.ready = true;
    }

    /// Binds the atlas for a `Material::msdf()` draw (recognized by its shader); other materials are untouched.
    pub unsafe fn bind(
        &self,
        cmd: vk::CommandBuffer,
        layout: vk::PipelineLayout,
        shader: &'static Align4<[u8]>,
    ) {
        if !ptr::eq(shader, MSDF) {
            return;
        }
        if !self.ready {
            die(FONT_ATLAS_MISSING_OR_TOO_SMALL);
        }
        vkCmdBindDescriptorSets(
            cmd,
            vk::PipelineBindPoint::GRAPHICS,
            layout,
            0,
            1,
            &self.set,
            0,
            ptr::null(),
        );
    }
}
