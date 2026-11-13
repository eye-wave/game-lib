//! Vulkan plumbing every other module shares: errors, memory, images, buffers, barriers, pipelines.
use alloc::vec::Vec;
use ash::vk;
use core::{
    ffi::{CStr, c_void},
    ptr,
};
use errors::prelude::*;
use platform::die;
use shader::{Align4, words};
use vulkan_sys::*;

pub(crate) const WIDTH: u16 = 800;
pub(crate) const HEIGHT: u16 = 600;
pub(crate) const FRAMES: usize = 2;
pub(crate) const SCENE_FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM; // offscreen color
pub(crate) const DEPTH_FORMAT: vk::Format = vk::Format::D32_SFLOAT;
/// Samples per pixel of the scene pass: 4x MSAA with feature `aa`, else none.
pub(crate) const SAMPLES: vk::SampleCountFlags = if cfg!(feature = "aa") {
    vk::SampleCountFlags::TYPE_4
} else {
    vk::SampleCountFlags::TYPE_1
};

pub(crate) struct Ctx {
    pub device: vk::Device,
    pub phys: vk::PhysicalDevice,
    pub surface: vk::SurfaceKHR,
}

pub(crate) trait OrDie<T> {
    fn or_die(self, msg: &CStr) -> T;
}
impl<T, E> OrDie<T> for Result<T, E> {
    fn or_die(self, msg: &CStr) -> T {
        match self {
            Ok(v) => v,
            Err(_) => die(msg),
        }
    }
}
impl<T> OrDie<T> for Option<T> {
    fn or_die(self, msg: &CStr) -> T {
        match self {
            Some(v) => v,
            None => die(msg),
        }
    }
}

pub(crate) unsafe fn ck(r: vk::Result, m: &CStr) {
    if r != vk::Result::SUCCESS {
        die(m)
    }
}

// Call a `vkCreate*`-style function and return its output handle.
pub(crate) unsafe fn mk<T: Default>(m: &CStr, f: impl FnOnce(*mut T) -> vk::Result) -> T {
    let mut v = T::default();
    ck(f(&mut v), m);
    v
}

// Vulkan's two-call "count, then fill" enumeration pattern.
pub(crate) unsafe fn list<T: Default + Clone>(f: impl Fn(*mut u32, *mut T)) -> Vec<T> {
    let mut n = 0;
    f(&mut n, ptr::null_mut());
    let mut v = alloc::vec![T::default(); n as usize];
    f(&mut n, v.as_mut_ptr());
    v
}

pub(crate) unsafe fn shader_module(d: vk::Device, spv: &'static Align4<[u8]>) -> vk::ShaderModule {
    mk(CREATE_SHADER_MODULE_FAILED, |p| {
        vkCreateShaderModule(
            d,
            &vk::ShaderModuleCreateInfo::default().code(words(spv)),
            NA,
            p,
        )
    })
}

/// Where the swapchain image waits for the acquire semaphore (the first stage that may touch it).
pub(crate) fn acquire_stages() -> vk::PipelineStageFlags {
    vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT | vk::PipelineStageFlags::TRANSFER
}

pub(crate) fn color_range() -> vk::ImageSubresourceRange {
    range(vk::ImageAspectFlags::COLOR)
}

pub(crate) fn range(aspect: vk::ImageAspectFlags) -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange::default()
        .aspect_mask(aspect)
        .level_count(1)
        .layer_count(1)
}

/// An image the renderer owns, with its memory and view.
pub(crate) struct Target {
    pub image: vk::Image,
    pub view: vk::ImageView,
    pub mem: vk::DeviceMemory,
}

pub(crate) unsafe fn make_target(
    c: &Ctx,
    e: vk::Extent2D,
    format: vk::Format,
    usage: vk::ImageUsageFlags,
    aspect: vk::ImageAspectFlags,
) -> Target {
    make_target_ms(c, e, format, usage, aspect, vk::SampleCountFlags::TYPE_1)
}

pub(crate) unsafe fn make_target_ms(
    c: &Ctx,
    e: vk::Extent2D,
    format: vk::Format,
    usage: vk::ImageUsageFlags,
    aspect: vk::ImageAspectFlags,
    samples: vk::SampleCountFlags,
) -> Target {
    let image: vk::Image = mk(CREATE_IMAGE_FAILED, |p| {
        vkCreateImage(
            c.device,
            &vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(format)
                .extent(vk::Extent3D {
                    width: e.width,
                    height: e.height,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(samples)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(usage)
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .initial_layout(vk::ImageLayout::UNDEFINED),
            NA,
            p,
        )
    });
    let mut req = vk::MemoryRequirements::default();
    vkGetImageMemoryRequirements(c.device, image, &mut req);
    let mem = alloc_mem(c, req, false);
    ck(
        vkBindImageMemory(c.device, image, mem, 0),
        BIND_IMAGE_MEMORY_FAILED,
    );
    let view = mk(CREATE_IMAGE_VIEW_FAILED, |p| {
        vkCreateImageView(
            c.device,
            &vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(format)
                .subresource_range(range(aspect)),
            NA,
            p,
        )
    });
    Target { image, view, mem }
}

pub(crate) unsafe fn destroy_target(c: &Ctx, t: &Target) {
    vkDestroyImageView(c.device, t.view, NA);
    vkDestroyImage(c.device, t.image, NA);
    vkFreeMemory(c.device, t.mem, NA);
}

pub(crate) unsafe fn transition(
    cmd: vk::CommandBuffer,
    image: vk::Image,
    aspect: vk::ImageAspectFlags,
    (old, new): (vk::ImageLayout, vk::ImageLayout),
    (src_stage, dst_stage): (vk::PipelineStageFlags, vk::PipelineStageFlags),
    (src_access, dst_access): (vk::AccessFlags, vk::AccessFlags),
) {
    let barrier = vk::ImageMemoryBarrier::default()
        .old_layout(old)
        .new_layout(new)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(range(aspect))
        .src_access_mask(src_access)
        .dst_access_mask(dst_access);
    vkCmdPipelineBarrier(
        cmd,
        src_stage,
        dst_stage,
        vk::DependencyFlags::empty(),
        0,
        ptr::null(),
        0,
        ptr::null(),
        1,
        &barrier,
    );
}

/// Device memory for `req`: host-visible and coherent, or device-local.
pub(crate) unsafe fn alloc_mem(
    c: &Ctx,
    req: vk::MemoryRequirements,
    host: bool,
) -> vk::DeviceMemory {
    let mut props = vk::PhysicalDeviceMemoryProperties::default();
    vkGetPhysicalDeviceMemoryProperties(c.phys, &mut props);
    let want = if host {
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT
    } else {
        vk::MemoryPropertyFlags::DEVICE_LOCAL
    };
    let ty = (0..props.memory_type_count)
        .find(|&i| {
            req.memory_type_bits & (1 << i) != 0
                && props.memory_types[i as usize].property_flags.contains(want)
        })
        .or_die(NO_SUITABLE_MEMORY_TYPE);
    let mem: vk::DeviceMemory = mk(ALLOCATE_MEMORY_FAILED, |p| {
        vkAllocateMemory(
            c.device,
            &vk::MemoryAllocateInfo::default()
                .allocation_size(req.size)
                .memory_type_index(ty),
            NA,
            p,
        )
    });
    mem
}

pub(crate) struct Buf {
    pub buf: vk::Buffer,
    pub ptr: *mut u8, // mapped; null for device-local buffers
}

// one allocation per buffer, never freed. Suballocate when buffers multiply.
pub(crate) unsafe fn make_buffer(
    c: &Ctx,
    size: u64,
    usage: vk::BufferUsageFlags,
    host: bool,
) -> Buf {
    let buf: vk::Buffer = mk(CREATE_BUFFER_FAILED, |p| {
        vkCreateBuffer(
            c.device,
            &vk::BufferCreateInfo::default()
                .size(size)
                .usage(usage)
                .sharing_mode(vk::SharingMode::EXCLUSIVE),
            NA,
            p,
        )
    });
    let mut req = vk::MemoryRequirements::default();
    vkGetBufferMemoryRequirements(c.device, buf, &mut req);
    let mem = alloc_mem(c, req, host);
    ck(
        vkBindBufferMemory(c.device, buf, mem, 0),
        BIND_BUFFER_MEMORY_FAILED,
    );
    let mut ptr: *mut c_void = ptr::null_mut();
    if host {
        ck(
            vkMapMemory(
                c.device,
                mem,
                0,
                size,
                vk::MemoryMapFlags::empty(),
                &mut ptr,
            ),
            MAP_MEMORY_FAILED,
        );
    }
    Buf {
        buf,
        ptr: ptr.cast(),
    }
}

/// Graphics pipeline with a `vs_main` / `fs_main` module, used by scene materials and the post pass.
#[allow(clippy::too_many_arguments)]
pub(crate) unsafe fn build_pipeline(
    c: &Ctx,
    layout: vk::PipelineLayout,
    spv: &'static Align4<[u8]>,
    format: vk::Format,
    depth: bool,
    blend: bool,
    samples: vk::SampleCountFlags,
    topology: vk::PrimitiveTopology,
    vertex_input: &vk::PipelineVertexInputStateCreateInfo,
) -> vk::Pipeline {
    let shader = shader_module(c.device, spv);
    let stages = [
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(shader)
            .name(c"vs_main"),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(shader)
            .name(c"fs_main"),
    ];
    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default().topology(topology);
    let viewport_state = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);
    let raster = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::NONE)
        .front_face(vk::FrontFace::CLOCKWISE)
        .line_width(1.0);
    // alpha to coverage: a fragment's alpha becomes the share of samples it covers (smooth MSDF text edges, no
    // sorting; opaque materials output alpha 1 and are unaffected). Blending materials use their alpha for that.
    let multisample = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(samples)
        .alpha_to_coverage_enable(samples != vk::SampleCountFlags::TYPE_1 && !blend);
    let blend_att = [vk::PipelineColorBlendAttachmentState::default()
        .color_write_mask(vk::ColorComponentFlags::RGBA)
        .blend_enable(blend)
        .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
        .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .color_blend_op(vk::BlendOp::ADD)
        .src_alpha_blend_factor(vk::BlendFactor::ONE)
        .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .alpha_blend_op(vk::BlendOp::ADD)];
    let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_att);
    let dyn_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dyn_states);
    let color_formats = [format];
    let mut rendering =
        vk::PipelineRenderingCreateInfo::default().color_attachment_formats(&color_formats);
    if depth {
        rendering = rendering.depth_attachment_format(DEPTH_FORMAT);
    }
    let depth_state = vk::PipelineDepthStencilStateCreateInfo::default()
        .depth_test_enable(depth)
        .depth_write_enable(depth)
        .depth_compare_op(vk::CompareOp::LESS);

    let info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&stages)
        .vertex_input_state(vertex_input)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport_state)
        .rasterization_state(&raster)
        .multisample_state(&multisample)
        .color_blend_state(&blend)
        .depth_stencil_state(&depth_state)
        .dynamic_state(&dynamic)
        .layout(layout)
        .push_next(&mut rendering);
    let mut pipeline = vk::Pipeline::default();
    ck(
        vkCreateGraphicsPipelines(
            c.device,
            vk::PipelineCache::null(),
            1,
            &info,
            NA,
            &mut pipeline,
        ),
        CREATE_GRAPHICS_PIPELINES_FAILED,
    );
    vkDestroyShaderModule(c.device, shader, NA);
    pipeline
}
