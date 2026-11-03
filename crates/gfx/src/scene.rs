//! The instanced scene: gathering drawables into the per-frame instance buffer, and drawing one camera's view of them.
use crate::{
    assets::{Assets, Handle, Material, Mesh},
    cams::{MAX_CAMERAS, View},
    cull::Cull,
    gpu::*,
    swapchain::Swapchain,
    textures::Textures,
    transform::{Color, Transform},
};
use alloc::vec::Vec;
use ash::vk;
use core::ptr;
use ecs::App;
use errors::prelude::*;
use platform::die;
use vulkan_sys::*;

pub(crate) const GROUPS: usize = 16; // distinct (mesh, material) pairs drawn per frame
pub(crate) const CAP: usize = 4096; // instances per group; must match cull.wgsl

/// One drawable, as the culling shader and the vertex shader read it (std430: 3 x vec4).
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Inst {
    a: [f32; 4],     // x, y, z, yaw
    s: [f32; 4],     // scale xyz, group
    color: [f32; 4], // tint rgb, mesh bounding radius squared
}

/// Buffers one in-flight frame owns (the CPU rewrites them while the other frame is on the GPU).
pub(crate) struct Frame {
    pub input: Buf, // every drawable, written by the CPU
    pub draws: Buf, // one VkDrawIndirectCommand per group and camera; culling fills in instanceCount
}
impl Frame {
    pub unsafe fn new(c: &Ctx) -> Self {
        let storage = vk::BufferUsageFlags::STORAGE_BUFFER;
        let inst_bytes = (GROUPS * CAP * size_of::<Inst>()) as u64;
        Frame {
            // also drawn from directly when culling is off
            input: make_buffer(
                c,
                inst_bytes,
                storage | vk::BufferUsageFlags::VERTEX_BUFFER,
                true,
            ),
            draws: make_buffer(
                c,
                (MAX_CAMERAS * GROUPS * size_of::<[u32; 4]>()) as u64,
                storage | vk::BufferUsageFlags::INDIRECT_BUFFER,
                true,
            ),
        }
    }
}

struct GpuMesh {
    buf: vk::Buffer,
    count: u32,
}

/// Everything one frame's draw pass needs, borrowed from the renderer.
pub(crate) struct Pass<'a> {
    pub c: &'a Ctx,
    pub cmd: vk::CommandBuffer,
    pub app: &'a App,
    pub fr: &'a Frame,
    pub f: usize, // frame in flight
    pub cull: &'a Cull,
    pub sc: &'a Swapchain,
    pub idx: usize, // acquired swapchain image
    pub views: &'a [View],
    pub groups: &'a [(usize, usize)], // (mesh, material) per group
}

pub(crate) struct Scene {
    layout: vk::PipelineLayout,
    pub tex: Textures,
    meshes: Vec<Option<GpuMesh>>,
    pipes: Vec<Option<vk::Pipeline>>,
    // with `aa`: per camera, the multisampled color and depth it draws into before resolving into the caller's target
    msaa: [Option<(vk::Extent2D, Target, Target)>; MAX_CAMERAS],
}

/// Cache slot `i`, building it with `f` the first time.
fn slot<T>(v: &mut Vec<Option<T>>, i: usize, f: impl FnOnce() -> T) -> &T {
    if v.len() <= i {
        v.resize_with(i + 1, || None);
    }
    v[i].get_or_insert_with(f)
}

impl Scene {
    pub unsafe fn new(c: &Ctx) -> Self {
        let tex = Textures::new(c); // set 0 when `textures` is on, no sets otherwise
        // the camera view is the only push constant
        let push = [vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::VERTEX)
            .size(size_of::<[f32; 16]>() as u32)];
        let layout: vk::PipelineLayout = mk(CREATE_PIPELINE_LAYOUT_FAILED, |p| {
            vkCreatePipelineLayout(
                c.device,
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(tex.layouts())
                    .push_constant_ranges(&push),
                NA,
                p,
            )
        });
        Self {
            layout,
            tex,
            meshes: Vec::new(),
            pipes: Vec::new(),
            msaa: core::array::from_fn(|_| None),
        }
    }

    /// Writes every drawable into `fr.input`, uploads new meshes, and resets the indirect draws of `cams` cameras.
    /// Returns the (mesh, material) groups and how many instances were written.
    pub unsafe fn upload(
        &mut self,
        c: &Ctx,
        app: &App,
        fr: &Frame,
        cull: &Cull,
        cams: usize,
    ) -> (Vec<(usize, usize)>, usize) {
        let world = &app.world;
        let meshes = world.resource::<Assets<Mesh>>().unwrap();
        let mut groups: Vec<(usize, usize)> = Vec::new();
        let mut counts = [0usize; GROUPS];
        let mut n = 0;
        for e in world.query_of::<(Transform, Handle<Mesh>, Handle<Material>)>() {
            let mi = world.get::<Handle<Mesh>>(e).unwrap().0 as usize;
            let ma = world.get::<Handle<Material>>(e).unwrap().0 as usize;
            let g = match groups.iter().position(|&x| x == (mi, ma)) {
                Some(g) => g,
                None if groups.len() < GROUPS => {
                    groups.push((mi, ma));
                    groups.len() - 1
                }
                None => continue, // extra (mesh, material) pairs are dropped; raise GROUPS
            };
            if counts[g] == CAP {
                continue; // extra instances are dropped; raise CAP (and cull.wgsl's)
            }
            counts[g] += 1;
            let dst = cull.dst(n, g, counts[g]);
            let t = world.get::<Transform>(e).unwrap();
            let tint = world.get::<Color>(e).map_or([1.0; 3], |c| c.0);
            fr.input.ptr.cast::<Inst>().add(dst).write(Inst {
                a: [t.pos.x, t.pos.y, t.pos.z, t.yaw],
                s: [t.scale.x, t.scale.y, t.scale.z, g as f32],
                color: [tint[0], tint[1], tint[2], meshes.0[mi].radius2],
            });
            n += 1;
        }
        for &(mi, _) in &groups {
            slot(&mut self.meshes, mi, || upload_mesh(c, &meshes.0[mi]));
        }
        // each camera has its own slice of `draws`
        for k in 0..cams {
            for (g, &(mi, _)) in groups.iter().enumerate() {
                let m = self.meshes[mi].as_ref().unwrap();
                fr.draws.ptr.cast::<[u32; 4]>().add(k * GROUPS + g).write([
                    m.count,
                    cull.instance_count(counts[g]),
                    0,
                    0,
                ]);
            }
        }
        (groups, n)
    }

    /// Camera `k`'s multisampled color and depth views, (re)made for `ext` and ready to be drawn into.
    unsafe fn msaa(
        &mut self,
        p: &Pass,
        k: usize,
        ext: vk::Extent2D,
    ) -> (vk::ImageView, vk::ImageView) {
        let (c, cmd) = (p.c, p.cmd);
        if self.msaa[k].as_ref().is_some_and(|m| m.0 != ext) {
            ck(vkDeviceWaitIdle(c.device), WAIT_FOR_DEVICE_IDLE_FAILED);
            let (_, col, dep) = self.msaa[k].take().unwrap();
            destroy_target(c, &col);
            destroy_target(c, &dep);
        }
        let (_, col, dep) = self.msaa[k].get_or_insert_with(|| {
            (
                ext,
                make_target_ms(
                    c,
                    ext,
                    p.sc.draw_format(),
                    vk::ImageUsageFlags::COLOR_ATTACHMENT,
                    vk::ImageAspectFlags::COLOR,
                    SAMPLES,
                ),
                make_target_ms(
                    c,
                    ext,
                    DEPTH_FORMAT,
                    vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
                    vk::ImageAspectFlags::DEPTH,
                    SAMPLES,
                ),
            )
        });
        // contents are cleared every frame; the barriers only order this frame after the last one's use
        transition(
            cmd,
            col.image,
            vk::ImageAspectFlags::COLOR,
            (
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            ),
            (
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            ),
            (
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            ),
        );
        transition(
            cmd,
            dep.image,
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
        (col.view, dep.view)
    }

    /// Draws every group for camera `k` into `color` / `depth` (both `ext` large), clearing them first.
    /// The caller has already moved both into their attachment layouts.
    pub unsafe fn draw(
        &mut self,
        p: &Pass,
        color: vk::ImageView,
        depth: vk::ImageView,
        ext: vk::Extent2D,
        k: usize,
        v: &View,
    ) {
        let cmd = p.cmd;
        let layout = self.layout;
        let fmt = p.sc.draw_format();
        let mats = p.app.world.resource::<Assets<Material>>().unwrap();
        let area = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: ext,
        };
        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: ext.width as f32,
            height: ext.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        // with `aa` draw into multisampled targets of our own and resolve into `color` (the caller's depth is unused)
        let (draw_color, depth, resolve) = if cfg!(feature = "aa") {
            let (c, d) = self.msaa(p, k, ext);
            (c, d, Some(color))
        } else {
            (color, depth, None)
        };
        let mut color = vk::RenderingAttachmentInfo::default()
            .image_view(draw_color)
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .clear_value(vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [v.sky[0], v.sky[1], v.sky[2], 1.0],
                },
            });
        if let Some(r) = resolve {
            color = color
                .resolve_mode(vk::ResolveModeFlags::AVERAGE)
                .resolve_image_view(r)
                .resolve_image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .store_op(vk::AttachmentStoreOp::DONT_CARE);
        }
        let color = [color];
        let depth = vk::RenderingAttachmentInfo::default()
            .image_view(depth)
            .image_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::DONT_CARE)
            .clear_value(vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue {
                    depth: 1.0,
                    stencil: 0,
                },
            });
        vkCmdBeginRendering(
            cmd,
            &vk::RenderingInfo::default()
                .render_area(area)
                .layer_count(1)
                .color_attachments(&color)
                .depth_attachment(&depth),
        );
        vkCmdSetViewport(cmd, 0, 1, &viewport);
        vkCmdSetScissor(cmd, 0, 1, &area);
        // opaque groups first, then the blending ones over them
        // ponytail: blending groups draw in first-seen order, unsorted: two see-through things overlapping may
        // blend in the wrong order; sort them back to front by z if that shows
        let order = (0..2).flat_map(|pass| {
            p.groups
                .iter()
                .enumerate()
                .filter(move |&(_, &(_, ma))| mats.0[ma].blend == (pass == 1))
        });
        for (g, &(mi, ma)) in order {
            // one pipeline bind per group; fine while groups are few.
            let pipe = *slot(&mut self.pipes, ma, || {
                make_pipeline(p.c, layout, fmt, &mats.0[ma])
            });
            let m = self.meshes[mi].as_ref().unwrap();
            let slice = k * GROUPS + g; // this camera's slice of the visible instances and `draws`
            vkCmdBindPipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pipe);
            self.tex.bind(cmd, layout, ma, mats.0[ma].shader);
            vkCmdPushConstants(
                cmd,
                layout,
                vk::ShaderStageFlags::VERTEX,
                0,
                size_of::<[f32; 16]>() as u32,
                v.vp.as_ptr().cast(),
            );
            let (inst_buf, inst_off) = p.cull.instances(p.f, p.fr, slice, g);
            let bufs = [m.buf, inst_buf];
            let offs = [0, inst_off];
            vkCmdBindVertexBuffers(cmd, 0, 2, bufs.as_ptr(), offs.as_ptr());
            vkCmdDrawIndirect(
                cmd,
                p.fr.draws.buf,
                (slice * size_of::<[u32; 4]>()) as u64,
                1,
                16,
            );
        }
        vkCmdEndRendering(cmd);
    }
}

// meshes live in host-visible memory; add staging to device-local when meshes get big.
unsafe fn upload_mesh(c: &Ctx, m: &Mesh) -> GpuMesh {
    let b = make_buffer(
        c,
        m.data.len() as u64,
        vk::BufferUsageFlags::VERTEX_BUFFER,
        true,
    );
    ptr::copy_nonoverlapping(m.data.as_ptr(), b.ptr, m.data.len());
    GpuMesh {
        buf: b.buf,
        count: m.count,
    }
}

/// The one place that turns a `Material` into Vulkan state: edit here for blending, culling, topology, extra inputs...
unsafe fn make_pipeline(
    c: &Ctx,
    layout: vk::PipelineLayout,
    format: vk::Format,
    m: &Material,
) -> vk::Pipeline {
    if m.attrs.len() > 4 {
        die(MATERIAL_HAS_MORE_THAN_4_VERTEX_ATTRIBUTES);
    }
    let binds = [
        vk::VertexInputBindingDescription::default()
            .stride(m.stride)
            .input_rate(vk::VertexInputRate::VERTEX),
        vk::VertexInputBindingDescription::default()
            .binding(1)
            .stride(size_of::<Inst>() as u32)
            .input_rate(vk::VertexInputRate::INSTANCE),
    ];
    let mut offset = 0;
    let mut attrs: Vec<_> = m
        .attrs
        .iter()
        .enumerate()
        .map(|(i, &f)| {
            let a = vk::VertexInputAttributeDescription::default()
                .location(i as u32)
                .format(f)
                .offset(offset);
            offset += match f {
                vk::Format::R32_SFLOAT => 4,
                vk::Format::R32G32_SFLOAT => 8,
                vk::Format::R32G32B32_SFLOAT => 12,
                vk::Format::R32G32B32A32_SFLOAT => 16,
                _ => die(MATERIAL_ATTRIBUTE_FORMAT_UNSUPPORTED_ADD_ITS_SIZE_IN_MAKE_PIPELINE),
            };
            a
        })
        .collect();
    for (loc, off) in [(4, 0), (5, 16), (6, 32)] {
        attrs.push(
            vk::VertexInputAttributeDescription::default()
                .location(loc)
                .binding(1)
                .format(vk::Format::R32G32B32A32_SFLOAT)
                .offset(off),
        );
    }
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
        .vertex_binding_descriptions(&binds)
        .vertex_attribute_descriptions(&attrs);
    build_pipeline(
        c,
        layout,
        m.shader,
        format,
        true,
        m.blend,
        SAMPLES,
        &vertex_input,
    )
}
