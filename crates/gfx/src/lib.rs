//! Window, Vulkan and the render loop, behind a ecs API.
//!
//! Game code only touches `Mesh`, `Material`, `Transform`, `Handle`, `Assets`, `Input` and `app.run(..)`.
//! Edit the Vulkan below only for new kinds of mesh/material pipelines (see `Material`, `make_pipeline`).
#![no_std]
#![allow(unsafe_op_in_unsafe_fn)]

extern crate alloc;

use alloc::vec::Vec;
use ash::khr;
pub use ash::vk;
use core::{
    ffi::{CStr, c_void},
    marker::PhantomData,
    ptr, slice,
};
use ecs::App;
use platform::{die, input::Keys};
use shader::{Align4, words};
use vulkan_sys::*;

// ---------------------------------------------------------------- game-facing API

/// Typed index into `Assets<T>`; it is the component you put on entities.
pub struct Handle<T>(u32, PhantomData<fn() -> T>);
impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Handle<T> {}

/// Resource holding every `T`; `add` returns the handle.
pub struct Assets<T>(Vec<T>);
impl<T> Default for Assets<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}
impl<T> Assets<T> {
    pub fn add(&mut self, v: T) -> Handle<T> {
        self.0.push(v);
        Handle(self.0.len() as u32 - 1, PhantomData)
    }
}

/// Vertex data as raw bytes: any vertex type, as long as the `Material` describes its layout.
pub struct Mesh {
    data: Vec<u8>,
    count: u32,
}
impl Mesh {
    /// `V` should be `#[repr(C)]` plain data.
    pub fn new<V>(verts: &[V]) -> Self {
        let bytes =
            unsafe { slice::from_raw_parts(verts.as_ptr().cast::<u8>(), size_of_val(verts)) };
        Self {
            data: bytes.to_vec(),
            count: verts.len() as u32,
        }
    }
}

/// A shader plus the vertex layout it reads. One Vulkan pipeline is built per material, on first use.
/// The vertex stage gets a `var<immediate> vec2<f32>` = the entity's `Transform`.
pub struct Material {
    pub shader: &'static Align4<[u8]>, // entry points: vs_main, fs_main
    pub stride: u32,
    pub attrs: &'static [vk::Format], // tightly packed, location = index
}
impl Material {
    /// For `Vertex`: location 0 = pos, location 1 = color.
    pub const fn colored(shader: &'static Align4<[u8]>) -> Self {
        Self {
            shader,
            stride: size_of::<Vertex>() as u32,
            attrs: &[vk::Format::R32G32_SFLOAT, vk::Format::R32G32B32_SFLOAT],
        }
    }
}

/// 2D position + RGB color, the vertex of `Material::colored`.
#[repr(C)]
pub struct Vertex {
    pub pos: [f32; 2],
    pub color: [f32; 3],
}

/// Clip-space translation (y up). Its bytes go straight to the shader, so keep it repr(C).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Transform {
    pub x: f32,
    pub y: f32,
}
impl Transform {
    pub const fn from_xy(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// Resource: keyboard state, one `platform::input::*` key id per key (`KeyW`, `ArrowLeft`, `Space`, ...).
#[derive(Clone, Copy, Default)]
pub struct Input {
    now: Keys,
    prev: Keys, // state at the previous frame
}
impl Input {
    pub fn is_holding(&self, key: u8) -> bool {
        self.now.get(key)
    }
    /// True only on the frame the key went down (a tap shorter than one frame is missed).
    pub fn just_pressed(&self, key: u8) -> bool {
        self.now.get(key) && !self.prev.get(key)
    }
    /// +1 while any of `plus` is held, -1 while any of `minus` is, 0 for both or neither.
    pub fn axis(&self, plus: &[u8], minus: &[u8]) -> f32 {
        let any = |ks: &[u8]| ks.iter().any(|&k| self.is_holding(k));
        any(plus) as i8 as f32 - any(minus) as i8 as f32
    }
}

/// `app.run(title)`: open a window and drive the app until it closes.
/// Adds the `Assets<Mesh>`, `Assets<Material>` and `Input` resources first, so startup systems can use them.
pub trait RunExt {
    fn run(self, title: &CStr);
}
impl RunExt for App {
    fn run(self, title: &CStr) {
        unsafe { run(self, title) }
    }
}

// ---------------------------------------------------------------- vulkan helpers

trait OrDie<T> {
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

unsafe fn ck(r: vk::Result, m: &CStr) {
    if r != vk::Result::SUCCESS {
        die(m)
    }
}

// Call a `vkCreate*`-style function and return its output handle.
unsafe fn mk<T: Default>(m: &CStr, f: impl FnOnce(*mut T) -> vk::Result) -> T {
    let mut v = T::default();
    ck(f(&mut v), m);
    v
}

// Vulkan's two-call "count, then fill" enumeration pattern.
unsafe fn list<T: Default + Clone>(f: impl Fn(*mut u32, *mut T)) -> Vec<T> {
    let mut n = 0;
    f(&mut n, ptr::null_mut());
    let mut v = alloc::vec![T::default(); n as usize];
    f(&mut n, v.as_mut_ptr());
    v
}

unsafe fn shader_module(d: vk::Device, spv: &'static Align4<[u8]>) -> vk::ShaderModule {
    mk(c"create_shader_module failed", |p| {
        vkCreateShaderModule(
            d,
            &vk::ShaderModuleCreateInfo::default().code(words(spv)),
            NA,
            p,
        )
    })
}

const WIDTH: u16 = 800;
const HEIGHT: u16 = 600;
const FRAMES: usize = 2;

struct Ctx {
    device: vk::Device,
    phys: vk::PhysicalDevice,
    surface: vk::SurfaceKHR,
}

struct Swapchain {
    handle: vk::SwapchainKHR,
    format: vk::Format,
    extent: vk::Extent2D,
    images: Vec<vk::Image>,
    views: Vec<vk::ImageView>,
    render_done: Vec<vk::Semaphore>,
}

fn color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .level_count(1)
        .layer_count(1)
}

unsafe fn make_swapchain(c: &Ctx, old: vk::SwapchainKHR) -> Swapchain {
    let mut caps = vk::SurfaceCapabilitiesKHR::default();
    ck(
        vkGetPhysicalDeviceSurfaceCapabilitiesKHR(c.phys, c.surface, &mut caps),
        c"surface capabilities failed",
    );
    let formats = list(|n, p| {
        ck(
            vkGetPhysicalDeviceSurfaceFormatsKHR(c.phys, c.surface, n, p),
            c"surface formats failed",
        )
    });
    let fmt = formats
        .iter()
        .find(|f| {
            f.format == vk::Format::B8G8R8A8_UNORM
                && f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
        })
        .unwrap_or(&formats[0]);

    let extent = if caps.current_extent.width == u32::MAX {
        vk::Extent2D {
            width: WIDTH as u32,
            height: HEIGHT as u32,
        }
    } else {
        caps.current_extent
    };

    let mut count = caps.min_image_count + 1;
    if caps.max_image_count > 0 {
        count = count.min(caps.max_image_count);
    }

    let info = vk::SwapchainCreateInfoKHR::default()
        .surface(c.surface)
        .min_image_count(count)
        .image_format(fmt.format)
        .image_color_space(fmt.color_space)
        .image_extent(extent)
        .image_array_layers(1)
        .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
        .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
        .pre_transform(caps.current_transform)
        .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
        .present_mode(vk::PresentModeKHR::FIFO)
        .clipped(true)
        .old_swapchain(old);

    let handle = mk(c"create_swapchain failed", |p| {
        vkCreateSwapchainKHR(c.device, &info, NA, p)
    });
    let images = list(|n, p| {
        ck(
            vkGetSwapchainImagesKHR(c.device, handle, n, p),
            c"get_swapchain_images failed",
        )
    });

    let views = images
        .iter()
        .map(|&image| {
            mk(c"create_image_view failed", |p| {
                vkCreateImageView(
                    c.device,
                    &vk::ImageViewCreateInfo::default()
                        .image(image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(fmt.format)
                        .subresource_range(color_range()),
                    NA,
                    p,
                )
            })
        })
        .collect::<Vec<_>>();

    let render_done = images
        .iter()
        .map(|_| {
            mk(c"create_semaphore failed", |p| {
                vkCreateSemaphore(c.device, &vk::SemaphoreCreateInfo::default(), NA, p)
            })
        })
        .collect::<Vec<_>>();

    Swapchain {
        handle,
        format: fmt.format,
        extent,
        images,
        views,
        render_done,
    }
}

unsafe fn destroy_swapchain(c: &Ctx, sc: &Swapchain) {
    for &s in &sc.render_done {
        vkDestroySemaphore(c.device, s, NA);
    }
    for &v in &sc.views {
        vkDestroyImageView(c.device, v, NA);
    }
    vkDestroySwapchainKHR(c.device, sc.handle, NA);
}

unsafe fn transition(
    cmd: vk::CommandBuffer,
    image: vk::Image,
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
        .subresource_range(color_range())
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

struct GpuMesh {
    buf: vk::Buffer,
    count: u32,
}

// one host-visible buffer + allocation per mesh, never freed. Add staging/suballocation when meshes multiply.
unsafe fn upload_mesh(c: &Ctx, m: &Mesh) -> GpuMesh {
    let size = m.data.len() as u64;
    let buf: vk::Buffer = mk(c"create_buffer failed", |p| {
        vkCreateBuffer(
            c.device,
            &vk::BufferCreateInfo::default()
                .size(size)
                .usage(vk::BufferUsageFlags::VERTEX_BUFFER)
                .sharing_mode(vk::SharingMode::EXCLUSIVE),
            NA,
            p,
        )
    });
    let mut req = vk::MemoryRequirements::default();
    vkGetBufferMemoryRequirements(c.device, buf, &mut req);
    let mut props = vk::PhysicalDeviceMemoryProperties::default();
    vkGetPhysicalDeviceMemoryProperties(c.phys, &mut props);
    let want = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
    let ty = (0..props.memory_type_count)
        .find(|&i| {
            req.memory_type_bits & (1 << i) != 0
                && props.memory_types[i as usize].property_flags.contains(want)
        })
        .or_die(c"no host-visible memory type");
    let mem: vk::DeviceMemory = mk(c"allocate_memory failed", |p| {
        vkAllocateMemory(
            c.device,
            &vk::MemoryAllocateInfo::default()
                .allocation_size(req.size)
                .memory_type_index(ty),
            NA,
            p,
        )
    });
    ck(
        vkBindBufferMemory(c.device, buf, mem, 0),
        c"bind_buffer_memory failed",
    );
    let mut dst: *mut c_void = ptr::null_mut();
    ck(
        vkMapMemory(
            c.device,
            mem,
            0,
            size,
            vk::MemoryMapFlags::empty(),
            &mut dst,
        ),
        c"map_memory failed",
    );
    ptr::copy_nonoverlapping(m.data.as_ptr(), dst.cast(), m.data.len());
    GpuMesh {
        buf,
        count: m.count,
    }
}

/// The one place that turns a `Material` into Vulkan state: edit here for blending, depth, topology, extra inputs...
unsafe fn make_pipeline(
    c: &Ctx,
    layout: vk::PipelineLayout,
    format: vk::Format,
    m: &Material,
) -> vk::Pipeline {
    let shader = shader_module(c.device, m.shader);
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

    let binds = [vk::VertexInputBindingDescription::default()
        .stride(m.stride)
        .input_rate(vk::VertexInputRate::VERTEX)];
    let mut offset = 0;
    let attrs: Vec<_> = m
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
                _ => die(c"Material attr format unsupported (add its size in make_pipeline)"),
            };
            a
        })
        .collect();
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
        .vertex_binding_descriptions(&binds)
        .vertex_attribute_descriptions(&attrs);
    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
    let viewport_state = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);
    let raster = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::NONE)
        .front_face(vk::FrontFace::CLOCKWISE)
        .line_width(1.0);
    let multisample = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);
    let blend_att = [vk::PipelineColorBlendAttachmentState::default()
        .color_write_mask(vk::ColorComponentFlags::RGBA)];
    let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_att);
    let dyn_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dyn_states);
    let color_formats = [format];
    let mut rendering =
        vk::PipelineRenderingCreateInfo::default().color_attachment_formats(&color_formats);

    let info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport_state)
        .rasterization_state(&raster)
        .multisample_state(&multisample)
        .color_blend_state(&blend)
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
        c"create_graphics_pipelines failed",
    );
    vkDestroyShaderModule(c.device, shader, NA);
    pipeline
}

/// Cache slot `i`, building it with `f` the first time.
fn slot<T>(v: &mut Vec<Option<T>>, i: usize, f: impl FnOnce() -> T) -> &T {
    if v.len() <= i {
        v.resize_with(i + 1, || None);
    }
    v[i].get_or_insert_with(f)
}

unsafe fn run(mut app: App, title: &CStr) {
    app.world.insert_resource(Assets::<Mesh>::default());
    app.world.insert_resource(Assets::<Material>::default());
    app.world.insert_resource(Input::default());

    let w = platform::open(WIDTH, HEIGHT, title);
    let vkapp = vk::ApplicationInfo::default().api_version(vk::API_VERSION_1_3);
    let inst_exts = [khr::surface::NAME.as_ptr(), platform::SURFACE_EXT.as_ptr()];
    let instance: vk::Instance = mk(c"create_instance failed (no Vulkan driver?)", |p| {
        vkCreateInstance(
            &vk::InstanceCreateInfo::default()
                .application_info(&vkapp)
                .enabled_extension_names(&inst_exts),
            NA,
            p,
        )
    });

    let surface = platform::surface(instance, &w);

    let (phys, qf) = list(|n, p| {
        ck(
            vkEnumeratePhysicalDevices(instance, n, p),
            c"enumerate_physical_devices failed",
        )
    })
    .into_iter()
    .filter(|&p| {
        let mut props = vk::PhysicalDeviceProperties::default();
        vkGetPhysicalDeviceProperties(p, &mut props);
        props.api_version >= vk::API_VERSION_1_3
    })
    .find_map(|p| {
        let qs = list(|n, q| vkGetPhysicalDeviceQueueFamilyProperties(p, n, q));
        qs.iter().enumerate().find_map(|(i, q)| {
            let mut can_present = 0;
            let _ = vkGetPhysicalDeviceSurfaceSupportKHR(p, i as u32, surface, &mut can_present);
            (q.queue_flags.contains(vk::QueueFlags::GRAPHICS) && can_present != 0)
                .then_some((p, i as u32))
        })
    })
    .or_die(c"no Vulkan 1.3 GPU that can present to this window");

    let prio = [1.0f32];
    let queue_info = [vk::DeviceQueueCreateInfo::default()
        .queue_family_index(qf)
        .queue_priorities(&prio)];
    let mut f13 = vk::PhysicalDeviceVulkan13Features::default().dynamic_rendering(true);
    let dev_exts = [khr::swapchain::NAME.as_ptr()];
    let device: vk::Device = mk(c"create_device failed", |p| {
        vkCreateDevice(
            phys,
            &vk::DeviceCreateInfo::default()
                .queue_create_infos(&queue_info)
                .enabled_extension_names(&dev_exts)
                .push_next(&mut f13),
            NA,
            p,
        )
    });
    let mut queue = vk::Queue::default();
    vkGetDeviceQueue(device, qf, 0, &mut queue);

    let ctx = Ctx {
        device,
        phys,
        surface,
    };

    let mut sc = make_swapchain(&ctx, vk::SwapchainKHR::null());

    let push = [vk::PushConstantRange::default()
        .stage_flags(vk::ShaderStageFlags::VERTEX)
        .size(size_of::<Transform>() as u32)];
    let layout: vk::PipelineLayout = mk(c"create_pipeline_layout failed", |p| {
        vkCreatePipelineLayout(
            device,
            &vk::PipelineLayoutCreateInfo::default().push_constant_ranges(&push),
            NA,
            p,
        )
    });

    let pool: vk::CommandPool = mk(c"create_command_pool failed", |p| {
        vkCreateCommandPool(
            device,
            &vk::CommandPoolCreateInfo::default()
                .queue_family_index(qf)
                .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
            NA,
            p,
        )
    });
    let mut cmds = [vk::CommandBuffer::default(); FRAMES];
    ck(
        vkAllocateCommandBuffers(
            device,
            &vk::CommandBufferAllocateInfo::default()
                .command_pool(pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(FRAMES as u32),
            cmds.as_mut_ptr(),
        ),
        c"allocate_command_buffers failed",
    );
    let image_avail: [vk::Semaphore; FRAMES] = core::array::from_fn(|_| {
        mk(c"create_semaphore failed", |p| {
            vkCreateSemaphore(device, &vk::SemaphoreCreateInfo::default(), NA, p)
        })
    });
    let fences: [vk::Fence; FRAMES] = core::array::from_fn(|_| {
        mk(c"create_fence failed", |p| {
            vkCreateFence(
                device,
                &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                NA,
                p,
            )
        })
    });

    let mut resized = false;
    let mut shown = false;
    let mut frame = 0usize;
    let mut keys = Keys::default();
    let (mut gpu_meshes, mut pipes): (Vec<Option<GpuMesh>>, Vec<Option<vk::Pipeline>>) =
        (Vec::new(), Vec::new());
    let mut last = platform::now();

    loop {
        if !platform::poll(&w, &mut resized, &mut keys) {
            break;
        }
        let now = platform::now();
        let dt = (now - last) as f32;
        last = now;
        let input = app.world.resource_mut::<Input>().unwrap();
        (input.prev, input.now) = (input.now, keys);
        app.run_frame(dt);

        ck(
            vkWaitForFences(device, 1, &fences[frame], vk::TRUE, u64::MAX),
            c"wait_for_fences failed",
        );

        if resized {
            ck(vkDeviceWaitIdle(device), c"device_wait_idle failed");
            let new = make_swapchain(&ctx, sc.handle);
            destroy_swapchain(&ctx, &sc);
            sc = new;
            resized = false;
        }

        let mut idx = 0u32;
        match vkAcquireNextImageKHR(
            device,
            sc.handle,
            u64::MAX,
            image_avail[frame],
            vk::Fence::null(),
            &mut idx,
        ) {
            vk::Result::SUCCESS | vk::Result::SUBOPTIMAL_KHR => {}
            vk::Result::ERROR_OUT_OF_DATE_KHR => {
                resized = true;
                continue;
            }
            _ => die(c"acquire_next_image failed"),
        }
        let idx = idx as usize;
        ck(
            vkResetFences(device, 1, &fences[frame]),
            c"reset_fences failed",
        );

        let cmd = cmds[frame];
        ck(
            vkResetCommandBuffer(cmd, vk::CommandBufferResetFlags::empty()),
            c"reset_command_buffer failed",
        );
        ck(
            vkBeginCommandBuffer(
                cmd,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            ),
            c"begin_command_buffer failed",
        );

        transition(
            cmd,
            sc.images[idx],
            (
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            ),
            (
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            ),
            (
                vk::AccessFlags::empty(),
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            ),
        );

        let attachments = [vk::RenderingAttachmentInfo::default()
            .image_view(sc.views[idx])
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .clear_value(vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [0.05, 0.05, 0.08, 1.0],
                },
            })];
        let area = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: sc.extent,
        };
        vkCmdBeginRendering(
            cmd,
            &vk::RenderingInfo::default()
                .render_area(area)
                .layer_count(1)
                .color_attachments(&attachments),
        );
        vkCmdSetViewport(
            cmd,
            0,
            1,
            &vk::Viewport {
                x: 0.0,
                y: 0.0,
                width: sc.extent.width as f32,
                height: sc.extent.height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            },
        );
        vkCmdSetScissor(cmd, 0, 1, &area);
        let world = &app.world;
        let meshes = world.resource::<Assets<Mesh>>().unwrap();
        let mats = world.resource::<Assets<Material>>().unwrap();
        for e in world.query_of::<(Transform, Handle<Mesh>, Handle<Material>)>() {
            let t = world.get::<Transform>(e).unwrap();
            let mi = world.get::<Handle<Mesh>>(e).unwrap().0 as usize;
            let ma = world.get::<Handle<Material>>(e).unwrap().0 as usize;
            // rebinds per draw; sort by material when draw counts matter.
            let pipe = *slot(&mut pipes, ma, || {
                make_pipeline(&ctx, layout, sc.format, &mats.0[ma])
            });
            let m = slot(&mut gpu_meshes, mi, || upload_mesh(&ctx, &meshes.0[mi]));
            vkCmdBindPipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pipe);
            vkCmdPushConstants(
                cmd,
                layout,
                vk::ShaderStageFlags::VERTEX,
                0,
                size_of::<Transform>() as u32,
                ptr::from_ref(t).cast(),
            );
            vkCmdBindVertexBuffers(cmd, 0, 1, &m.buf, &0);
            vkCmdDraw(cmd, m.count, 1, 0, 0);
        }
        vkCmdEndRendering(cmd);

        transition(
            cmd,
            sc.images[idx],
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
        ck(vkEndCommandBuffer(cmd), c"end_command_buffer failed");

        let wait = [image_avail[frame]];
        let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        let cbs = [cmd];
        let signal = [sc.render_done[idx]];
        ck(
            vkQueueSubmit(
                queue,
                1,
                &vk::SubmitInfo::default()
                    .wait_semaphores(&wait)
                    .wait_dst_stage_mask(&wait_stages)
                    .command_buffers(&cbs)
                    .signal_semaphores(&signal),
                fences[frame],
            ),
            c"queue_submit failed",
        );

        let swapchains = [sc.handle];
        let indices = [idx as u32];
        match vkQueuePresentKHR(
            queue,
            &vk::PresentInfoKHR::default()
                .wait_semaphores(&signal)
                .swapchains(&swapchains)
                .image_indices(&indices),
        ) {
            vk::Result::SUCCESS => {}
            vk::Result::SUBOPTIMAL_KHR | vk::Result::ERROR_OUT_OF_DATE_KHR => resized = true,
            _ => die(c"queue_present failed"),
        }

        if !shown {
            platform::show(&w);
            shown = true;
        }

        frame = (frame + 1) % FRAMES;
    }

    // no teardown, the OS reclaims everything on exit
}
