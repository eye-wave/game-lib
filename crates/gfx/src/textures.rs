//! Feature `textures`: uploads every `Texture` a `Material` lists, gives each such material one descriptor set
//! (textures at bindings 0..3, a linear clamp sampler at 4), and binds it when the material is drawn. With `text`
//! the `Atlas` gets a set of its own, bound for `Material::msdf()`. `textures_off.rs` is the same interface without it.
use crate::{
    assets::{Assets, Material, TexFormat, Texture},
    gpu::*,
};
use alloc::vec::Vec;
use ash::vk;
use core::{ptr, slice};
use ecs::App;
use errors::prelude::*;
use platform::die;
use shader::Align4;
use vulkan_sys::*;

const SLOTS: usize = 4; // images per material: bindings 0..3, the sampler is 4
const MAX_SETS: u32 = 256; // materials with textures, plus the atlas

pub(crate) struct Textures {
    layout: vk::DescriptorSetLayout,
    sampler: vk::Sampler,
    #[cfg(feature = "text")]
    nearest: vk::Sampler, // for bitmap-font atlases
    pool: vk::DescriptorPool,
    images: Vec<Option<Target>>, // by texture index, uploaded on first use
    sets: Vec<Option<vk::DescriptorSet>>, // by material index
    #[cfg(feature = "text")]
    atlas: Option<vk::DescriptorSet>,
}

impl Textures {
    pub unsafe fn new(c: &Ctx) -> Self {
        let binds: [_; SLOTS + 1] = core::array::from_fn(|i| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(i as u32)
                .descriptor_type(if i < SLOTS {
                    vk::DescriptorType::SAMPLED_IMAGE
                } else {
                    vk::DescriptorType::SAMPLER
                })
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT)
        });
        let layout: vk::DescriptorSetLayout = mk(CREATE_DESCRIPTOR_SET_LAYOUT_FAILED, |p| {
            vkCreateDescriptorSetLayout(
                c.device,
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&binds),
                NA,
                p,
            )
        });
        let sampler = make_sampler(c, vk::Filter::LINEAR);
        let sizes = [
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(MAX_SETS * SLOTS as u32),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLER)
                .descriptor_count(MAX_SETS),
        ];
        let pool: vk::DescriptorPool = mk(CREATE_DESCRIPTOR_POOL_FAILED, |p| {
            vkCreateDescriptorPool(
                c.device,
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(MAX_SETS)
                    .pool_sizes(&sizes),
                NA,
                p,
            )
        });
        Self {
            layout,
            sampler,
            #[cfg(feature = "text")]
            nearest: make_sampler(c, vk::Filter::NEAREST),
            pool,
            images: Vec::new(),
            sets: Vec::new(),
            #[cfg(feature = "text")]
            atlas: None,
        }
    }

    pub fn layouts(&self) -> &[vk::DescriptorSetLayout] {
        slice::from_ref(&self.layout)
    }

    /// Uploads what is new: the `Atlas`, and the textures of every material that has no set yet.
    /// Records into `cmd`, outside any render pass.
    pub unsafe fn record(&mut self, c: &Ctx, cmd: vk::CommandBuffer, app: &App) {
        #[cfg(feature = "text")]
        self.record_atlas(c, cmd, app);
        let world = &app.world;
        let (Some(mats), Some(texs)) = (
            world.resource::<Assets<Material>>(),
            world.resource::<Assets<Texture>>(),
        ) else {
            return;
        };
        for (i, m) in mats.0.iter().enumerate() {
            if m.textures.is_empty() || self.sets.get(i).is_some_and(|s| s.is_some()) {
                continue;
            }
            if m.textures.len() > SLOTS {
                die(MATERIAL_HAS_MORE_THAN_4_TEXTURES);
            }
            let mut views = [vk::ImageView::null(); SLOTS];
            for (k, h) in m.textures.iter().enumerate() {
                let t = h.0 as usize;
                if self.images.len() <= t {
                    self.images.resize_with(t + 1, || None);
                }
                if self.images[t].is_none() {
                    self.images[t] = Some(upload(c, cmd, &texs.0[t]));
                }
                views[k] = self.images[t].as_ref().unwrap().view;
            }
            let set = self.make_set(c, &views[..m.textures.len()], self.sampler);
            if self.sets.len() <= i {
                self.sets.resize_with(i + 1, || None);
            }
            self.sets[i] = Some(set);
        }
    }

    #[cfg(feature = "text")]
    unsafe fn record_atlas(&mut self, c: &Ctx, cmd: vk::CommandBuffer, app: &App) {
        if self.atlas.is_some() {
            return;
        }
        let Some(a) = app.world.resource::<crate::text::Atlas>() else {
            return;
        };
        let n = (a.w * a.h) as usize;
        if a.rgb.len() < n * 3 {
            die(FONT_ATLAS_MISSING_OR_TOO_SMALL);
        }
        // RGB8 is not a format every GPU can sample, so pad to RGBA
        let mut px = Vec::with_capacity(n * 4);
        for p in a.rgb[..n * 3].chunks_exact(3) {
            px.extend_from_slice(&[p[0], p[1], p[2], 255]);
        }
        let img = upload(
            c,
            cmd,
            &Texture {
                w: a.w,
                h: a.h,
                format: TexFormat::Rgba8,
                px,
            },
        );
        let samp = if a.nearest {
            self.nearest
        } else {
            self.sampler
        };
        self.atlas = Some(self.make_set(c, &[img.view], samp));
    }

    /// A set with `views` at bindings 0.. and the sampler at 4.
    unsafe fn make_set(
        &self,
        c: &Ctx,
        views: &[vk::ImageView],
        sampler: vk::Sampler,
    ) -> vk::DescriptorSet {
        let mut set = vk::DescriptorSet::default();
        ck(
            vkAllocateDescriptorSets(
                c.device,
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(self.pool)
                    .set_layouts(slice::from_ref(&self.layout)),
                &mut set,
            ),
            ALLOCATE_DESCRIPTOR_SETS_FAILED,
        );
        let infos: Vec<_> = views
            .iter()
            .map(|&v| {
                vk::DescriptorImageInfo::default()
                    .image_view(v)
                    .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            })
            .collect();
        let samp = [vk::DescriptorImageInfo::default().sampler(sampler)];
        let mut writes: Vec<_> = infos
            .iter()
            .enumerate()
            .map(|(k, i)| {
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(k as u32)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .image_info(slice::from_ref(i))
            })
            .collect();
        writes.push(
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(SLOTS as u32)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .image_info(&samp),
        );
        vkUpdateDescriptorSets(
            c.device,
            writes.len() as u32,
            writes.as_ptr(),
            0,
            ptr::null(),
        );
        set
    }

    /// Binds the set of material `mat` (the atlas for `Material::msdf()`, recognized by its shader);
    /// materials without textures are untouched.
    #[cfg_attr(not(feature = "text"), allow(unused_variables))]
    pub unsafe fn bind(
        &self,
        cmd: vk::CommandBuffer,
        layout: vk::PipelineLayout,
        mat: usize,
        shader: &'static Align4<[u8]>,
    ) {
        #[cfg(feature = "text")]
        if ptr::eq(shader, crate::text::MSDF) {
            let Some(set) = self.atlas else {
                die(FONT_ATLAS_MISSING_OR_TOO_SMALL)
            };
            return self.bind_set(cmd, layout, set);
        }
        if let Some(&Some(set)) = self.sets.get(mat) {
            self.bind_set(cmd, layout, set);
        }
    }

    unsafe fn bind_set(
        &self,
        cmd: vk::CommandBuffer,
        layout: vk::PipelineLayout,
        set: vk::DescriptorSet,
    ) {
        vkCmdBindDescriptorSets(
            cmd,
            vk::PipelineBindPoint::GRAPHICS,
            layout,
            0,
            1,
            &set,
            0,
            ptr::null(),
        );
    }
}

unsafe fn make_sampler(c: &Ctx, filter: vk::Filter) -> vk::Sampler {
    mk(CREATE_SAMPLER_FAILED, |p| {
        vkCreateSampler(
            c.device,
            &vk::SamplerCreateInfo::default()
                .mag_filter(filter)
                .min_filter(filter)
                .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
            NA,
            p,
        )
    })
}

/// Makes a sampled image from `t` and records the copy into `cmd`. The staging buffer is never freed, like every
/// buffer here; it is texture-sized, once.
unsafe fn upload(c: &Ctx, cmd: vk::CommandBuffer, t: &Texture) -> Target {
    let (format, bpp) = match t.format {
        TexFormat::R8 => (vk::Format::R8_UNORM, 1),
        TexFormat::Rgba8 => (vk::Format::R8G8B8A8_UNORM, 4),
    };
    let n = (t.w * t.h) as usize * bpp;
    if t.px.len() < n {
        die(TEXTURE_PIXELS_SHORTER_THAN_W_X_H);
    }
    let stage = make_buffer(c, n as u64, vk::BufferUsageFlags::TRANSFER_SRC, true);
    ptr::copy_nonoverlapping(t.px.as_ptr(), stage.ptr, n);
    let img = make_target(
        c,
        vk::Extent2D {
            width: t.w,
            height: t.h,
        },
        format,
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
            width: t.w,
            height: t.h,
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
    img
}
