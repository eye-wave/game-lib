//! Window, Vulkan and the 3D render loop, behind a Bevy-ish API.
//!
//! Game code only touches `Mesh`, `Material`, `Transform`, `Color`, `Camera`, `Handle`, `Assets`, `Input` and `app.run(..)`.
//! Rendering is instanced and GPU-culled: every frame all drawable entities go into one instance buffer, a compute
//! shader (`cull.wgsl`) keeps the ones inside the camera frustum, and one indirect draw per (mesh, material) pair draws
//! them into an offscreen target (with depth). A fullscreen post-process pass (`PostProcess`) then writes the swapchain.
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
use errors::prelude::*;
use math::FloatExt;
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
    radius2: f32, // squared bounding radius around the origin, for culling
}
impl Mesh {
    /// `V` should be `#[repr(C)]` plain data whose first three `f32`s are the vertex position (used to bound the mesh).
    pub fn new<V>(verts: &[V]) -> Self {
        let bytes =
            unsafe { slice::from_raw_parts(verts.as_ptr().cast::<u8>(), size_of_val(verts)) };
        let radius2 = verts
            .iter()
            .map(|v| unsafe {
                let p = ptr::from_ref(v).cast::<f32>();
                *p * *p + *p.add(1) * *p.add(1) + *p.add(2) * *p.add(2)
            })
            .fold(0.0, |a, b| if b > a { b } else { a }); // not f32::max: that links libm
        Self {
            data: bytes.to_vec(),
            count: verts.len() as u32,
            radius2,
        }
    }
}

/// A shader plus the vertex layout it reads. One Vulkan pipeline is built per material, on first use.
/// Vertex shader contract (see `src/lit.wgsl`): mesh attributes at locations 0..`attrs.len()` (max 4),
/// `@location(4) vec4<f32>` = x, y, z, yaw, `@location(5) vec4<f32>` = scale xyz, `@location(6) vec4<f32>` = tint rgb
/// (from `Transform` and `Color`), and `var<immediate> mat4x4<f32>` = view * projection.
pub struct Material {
    pub shader: &'static Align4<[u8]>, // entry points: vs_main, fs_main
    pub stride: u32,
    pub attrs: &'static [vk::Format], // tightly packed, location = index
}
impl Material {
    /// For `Vertex`: location 0 = pos, 1 = normal, 2 = color.
    pub const fn lit(shader: &'static Align4<[u8]>) -> Self {
        Self {
            shader,
            stride: size_of::<Vertex>() as u32,
            attrs: &[vk::Format::R32G32B32_SFLOAT; 3],
        }
    }
}

/// Position, normal and color: the vertex of `Material::lit`.
#[repr(C)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 3],
}

/// World position (y up), rotation about the y axis in radians, and per-axis scale.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Transform {
    pub pos: [f32; 3],
    pub yaw: f32,
    pub scale: [f32; 3],
}
impl Default for Transform {
    fn default() -> Self {
        Self::from_pos(0.0, 0.0, 0.0)
    }
}
impl Transform {
    pub const fn from_pos(x: f32, y: f32, z: f32) -> Self {
        Self {
            pos: [x, y, z],
            yaw: 0.0,
            scale: [1.0; 3],
        }
    }
}

/// Optional tint multiplied into the mesh's vertex colors (white when absent).
#[derive(Clone, Copy)]
pub struct Color(pub [f32; 3]);

/// Resource: the viewpoint. Right-handed, y up; at yaw 0 and pitch 0 it looks down -z, and a positive yaw turns left.
/// `focal` is 1 / tan(vertical fov / 2). `sky` is the clear color.
#[derive(Clone, Copy)]
pub struct Camera {
    pub pos: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub focal: f32,
    pub near: f32,
    pub far: f32,
    pub sky: [f32; 3],
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            pos: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            focal: 1.428, // 70 degrees
            near: 0.1,
            far: 200.0,
            sky: [0.53, 0.68, 0.85],
        }
    }
}

/// Resource: the fullscreen post-processing shader (contract in `crates/gfx/src/post.wgsl`).
/// `None` is a passthrough. It is built the first time it is seen, so set it any time before drawing.
#[derive(Clone, Copy, Default)]
pub struct PostProcess(pub Option<&'static Align4<[u8]>>);

/// Resource: whether the mouse is captured (default true). Insert `MouseCapture(false)` before `app.run(..)` to keep
/// the normal pointer, e.g. in 2D apps that don't use `mouse_delta`. Changing it while running is not supported.
#[derive(Clone, Copy)]
pub struct MouseCapture(pub bool);

/// Resource: keyboard and mouse. Keys are `platform::input::*` ids (`KeyW`, `ArrowLeft`, `Space`, ...).
/// The mouse is captured (hidden, confined, recentered) while the window has focus, so it only reports motion
/// (`mouse_delta`), like an FPS.
#[derive(Clone, Copy, Default)]
pub struct Input {
    now: Keys,
    prev: Keys, // state at the previous frame
    delta: [f32; 2],
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
    /// Mouse motion since the last frame (x right, y down): raw device units on Linux with XInput2, else pixels.
    pub fn mouse_delta(&self) -> [f32; 2] {
        self.delta
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
    mk(CREATE_SHADER_MODULE_FAILED, |p| {
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
    scene: Target, // offscreen color the scene is drawn into
    depth: Target,
}

/// An image the renderer owns, with its memory and view.
struct Target {
    image: vk::Image,
    view: vk::ImageView,
    mem: vk::DeviceMemory,
}

fn color_range() -> vk::ImageSubresourceRange {
    range(vk::ImageAspectFlags::COLOR)
}

fn range(aspect: vk::ImageAspectFlags) -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange::default()
        .aspect_mask(aspect)
        .level_count(1)
        .layer_count(1)
}

unsafe fn make_swapchain(c: &Ctx, old: vk::SwapchainKHR) -> Swapchain {
    let mut caps = vk::SurfaceCapabilitiesKHR::default();
    ck(
        vkGetPhysicalDeviceSurfaceCapabilitiesKHR(c.phys, c.surface, &mut caps),
        GET_SURFACE_CAPABILITIES_FAILED,
    );
    let formats = list(|n, p| {
        ck(
            vkGetPhysicalDeviceSurfaceFormatsKHR(c.phys, c.surface, n, p),
            GET_SURFACE_FORMATS_FAILED,
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

    let handle = mk(CREATE_SWAPCHAIN_FAILED, |p| {
        vkCreateSwapchainKHR(c.device, &info, NA, p)
    });
    let images = list(|n, p| {
        ck(
            vkGetSwapchainImagesKHR(c.device, handle, n, p),
            GET_SWAPCHAIN_IMAGES_FAILED,
        )
    });

    let views = images
        .iter()
        .map(|&image| {
            mk(CREATE_IMAGE_VIEW_FAILED, |p| {
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
            mk(CREATE_SEMAPHORE_FAILED, |p| {
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
        scene: make_target(
            c,
            extent,
            SCENE_FORMAT,
            vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED,
            vk::ImageAspectFlags::COLOR,
        ),
        depth: make_target(
            c,
            extent,
            DEPTH_FORMAT,
            vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
            vk::ImageAspectFlags::DEPTH,
        ),
    }
}

unsafe fn make_target(
    c: &Ctx,
    e: vk::Extent2D,
    format: vk::Format,
    usage: vk::ImageUsageFlags,
    aspect: vk::ImageAspectFlags,
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
                .samples(vk::SampleCountFlags::TYPE_1)
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

unsafe fn destroy_swapchain(c: &Ctx, sc: &Swapchain) {
    for &s in &sc.render_done {
        vkDestroySemaphore(c.device, s, NA);
    }
    for &v in &sc.views {
        vkDestroyImageView(c.device, v, NA);
    }
    for t in [&sc.scene, &sc.depth] {
        vkDestroyImageView(c.device, t.view, NA);
        vkDestroyImage(c.device, t.image, NA);
        vkFreeMemory(c.device, t.mem, NA);
    }
    vkDestroySwapchainKHR(c.device, sc.handle, NA);
}

unsafe fn transition(
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

const GROUPS: usize = 4; // distinct (mesh, material) pairs drawn per frame
const CAP: usize = 4096; // instances per group; must match cull.wgsl
const SCENE_FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM; // offscreen color, sampled by the post pass
const DEPTH_FORMAT: vk::Format = vk::Format::D32_SFLOAT;
static CULL: &Align4<[u8]> = shader::include_shader!("cull");
static POST: &Align4<[u8]> = shader::include_shader!("post");

/// One drawable, as the culling shader and the vertex shader read it (std430: 3 x vec4).
#[repr(C)]
#[derive(Clone, Copy)]
struct Inst {
    a: [f32; 4],     // x, y, z, yaw
    s: [f32; 4],     // scale xyz, group
    color: [f32; 4], // tint rgb, mesh bounding radius squared
}

#[repr(C)]
struct CullPc {
    vp: [f32; 16],
    count: u32,
    _pad: [u32; 3],
}

/// View * projection, column-major, depth range 0..1 (see `Camera` for the conventions).
fn view_proj(c: &Camera, aspect: f32) -> [f32; 16] {
    let (sy, cy, sp, cp) = (c.yaw.sin(), (c.yaw).cos(), (c.pitch).sin(), (c.pitch).cos());
    let f = [-sy * cp, sp, -cy * cp]; // forward
    let r = [cy, 0.0, -sy]; // right
    let u = [sp * sy, cp, sp * cy]; // up
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let (fx, fy) = (c.focal / aspect, c.focal);
    let a = c.far / (c.near - c.far);
    let b = c.near * c.far / (c.near - c.far);
    let (fp, p) = (dot(f, c.pos), c.pos);
    let rows = [
        [r[0] * fx, r[1] * fx, r[2] * fx, -dot(r, p) * fx],
        [u[0] * fy, u[1] * fy, u[2] * fy, -dot(u, p) * fy],
        [-f[0] * a, -f[1] * a, -f[2] * a, fp * a + b],
        [f[0], f[1], f[2], -fp],
    ];
    core::array::from_fn(|i| rows[i % 4][i / 4]) // column-major
}

struct Buf {
    buf: vk::Buffer,
    ptr: *mut u8, // mapped; null for device-local buffers
}

/// Buffers one in-flight frame owns (the CPU rewrites them while the other frame is on the GPU).
struct Frame {
    input: Buf,   // every drawable, written by the CPU
    visible: Buf, // survivors of culling, per group slice (device-local, GPU only)
    draws: Buf,   // one VkDrawIndirectCommand per group; the culling shader fills in instanceCount
    set: vk::DescriptorSet,
}

struct GpuMesh {
    buf: vk::Buffer,
    count: u32,
}

/// Device memory for `req`: host-visible and coherent, or device-local.
unsafe fn alloc_mem(c: &Ctx, req: vk::MemoryRequirements, host: bool) -> vk::DeviceMemory {
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

// one allocation per buffer, never freed. Suballocate when buffers multiply.
unsafe fn make_buffer(c: &Ctx, size: u64, usage: vk::BufferUsageFlags, host: bool) -> Buf {
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
    build_pipeline(c, layout, m.shader, format, true, &vertex_input)
}

/// The fullscreen post-process pipeline: no vertex buffers, no depth.
unsafe fn make_post_pipeline(
    c: &Ctx,
    layout: vk::PipelineLayout,
    format: vk::Format,
    shader: &'static Align4<[u8]>,
) -> vk::Pipeline {
    build_pipeline(
        c,
        layout,
        shader,
        format,
        false,
        &vk::PipelineVertexInputStateCreateInfo::default(),
    )
}

unsafe fn build_pipeline(
    c: &Ctx,
    layout: vk::PipelineLayout,
    spv: &'static Align4<[u8]>,
    format: vk::Format,
    depth: bool,
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

/// Point the post-process descriptor set at the current scene image.
unsafe fn write_post_set(
    c: &Ctx,
    set: vk::DescriptorSet,
    sampler: vk::Sampler,
    view: vk::ImageView,
) {
    let image = [vk::DescriptorImageInfo::default()
        .image_view(view)
        .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
    let samp = [vk::DescriptorImageInfo::default().sampler(sampler)];
    let writes = [
        vk::WriteDescriptorSet::default()
            .dst_set(set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .image_info(&image),
        vk::WriteDescriptorSet::default()
            .dst_set(set)
            .dst_binding(1)
            .descriptor_type(vk::DescriptorType::SAMPLER)
            .image_info(&samp),
    ];
    vkUpdateDescriptorSets(c.device, 2, writes.as_ptr(), 0, ptr::null());
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
    if app.world.resource::<MouseCapture>().is_none() {
        app.world.insert_resource(MouseCapture(true));
    }
    app.world.insert_resource(Camera::default());
    app.world.insert_resource(PostProcess::default());

    let w = platform::open(WIDTH, HEIGHT, title);
    let vkapp = vk::ApplicationInfo::default().api_version(vk::API_VERSION_1_3);
    let inst_exts = [khr::surface::NAME.as_ptr(), platform::SURFACE_EXT.as_ptr()];
    let instance: vk::Instance = mk(CREATE_VULKAN_INSTANCE_FAILED_NO_DRIVER, |p| {
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
            ENUMERATE_PHYSICAL_DEVICES_FAILED,
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
    .or_die(NO_VULKAN_1_3_GPU_THAT_CAN_PRESENT_TO_THIS_WINDOW);

    let prio = [1.0f32];
    let queue_info = [vk::DeviceQueueCreateInfo::default()
        .queue_family_index(qf)
        .queue_priorities(&prio)];
    let mut f13 = vk::PhysicalDeviceVulkan13Features::default().dynamic_rendering(true);
    let dev_exts = [khr::swapchain::NAME.as_ptr()];
    let device: vk::Device = mk(CREATE_LOGICAL_DEVICE_FAILED, |p| {
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

    // graphics: the camera view is the only push constant
    let push = [vk::PushConstantRange::default()
        .stage_flags(vk::ShaderStageFlags::VERTEX)
        .size(size_of::<[f32; 16]>() as u32)];
    let layout: vk::PipelineLayout = mk(CREATE_PIPELINE_LAYOUT_FAILED, |p| {
        vkCreatePipelineLayout(
            device,
            &vk::PipelineLayoutCreateInfo::default().push_constant_ranges(&push),
            NA,
            p,
        )
    });

    // culling: 3 storage buffers (input, visible, draws) + view/count as push constants
    let set_binds: [vk::DescriptorSetLayoutBinding; 3] = core::array::from_fn(|i| {
        vk::DescriptorSetLayoutBinding::default()
            .binding(i as u32)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE)
    });
    let set_layout: vk::DescriptorSetLayout = mk(CREATE_DESCRIPTOR_SET_LAYOUT_FAILED, |p| {
        vkCreateDescriptorSetLayout(
            device,
            &vk::DescriptorSetLayoutCreateInfo::default().bindings(&set_binds),
            NA,
            p,
        )
    });
    let cull_push = [vk::PushConstantRange::default()
        .stage_flags(vk::ShaderStageFlags::COMPUTE)
        .size(size_of::<CullPc>() as u32)];
    let set_layouts = [set_layout];
    let cull_layout: vk::PipelineLayout = mk(CREATE_PIPELINE_LAYOUT_FAILED, |p| {
        vkCreatePipelineLayout(
            device,
            &vk::PipelineLayoutCreateInfo::default()
                .set_layouts(&set_layouts)
                .push_constant_ranges(&cull_push),
            NA,
            p,
        )
    });
    let cull_module = shader_module(device, CULL);
    let mut cull_pipe = vk::Pipeline::default();
    ck(
        vkCreateComputePipelines(
            device,
            vk::PipelineCache::null(),
            1,
            &vk::ComputePipelineCreateInfo::default()
                .stage(
                    vk::PipelineShaderStageCreateInfo::default()
                        .stage(vk::ShaderStageFlags::COMPUTE)
                        .module(cull_module)
                        .name(c"cull"),
                )
                .layout(cull_layout),
            NA,
            &mut cull_pipe,
        ),
        CREATE_COMPUTE_PIPELINES_FAILED,
    );
    vkDestroyShaderModule(device, cull_module, NA);

    let pool_sizes = [
        vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(3 * FRAMES as u32),
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
                .max_sets(FRAMES as u32 + 1)
                .pool_sizes(&pool_sizes),
            NA,
            p,
        )
    });
    let all_layouts = [set_layout; FRAMES];
    let mut sets = [vk::DescriptorSet::default(); FRAMES];
    ck(
        vkAllocateDescriptorSets(
            device,
            &vk::DescriptorSetAllocateInfo::default()
                .descriptor_pool(dpool)
                .set_layouts(&all_layouts),
            sets.as_mut_ptr(),
        ),
        ALLOCATE_DESCRIPTOR_SETS_FAILED,
    );
    let frames: [Frame; FRAMES] = core::array::from_fn(|f| {
        let storage = vk::BufferUsageFlags::STORAGE_BUFFER;
        let inst_bytes = (GROUPS * CAP * size_of::<Inst>()) as u64;
        let input = make_buffer(&ctx, inst_bytes, storage, true);
        let visible = make_buffer(
            &ctx,
            inst_bytes,
            storage | vk::BufferUsageFlags::VERTEX_BUFFER,
            false,
        );
        let draws = make_buffer(
            &ctx,
            (GROUPS * size_of::<[u32; 4]>()) as u64,
            storage | vk::BufferUsageFlags::INDIRECT_BUFFER,
            true,
        );
        let infos = [input.buf, visible.buf, draws.buf].map(|b| {
            [vk::DescriptorBufferInfo::default()
                .buffer(b)
                .range(vk::WHOLE_SIZE)]
        });
        let writes: [vk::WriteDescriptorSet; 3] = core::array::from_fn(|i| {
            vk::WriteDescriptorSet::default()
                .dst_set(sets[f])
                .dst_binding(i as u32)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&infos[i])
        });
        vkUpdateDescriptorSets(device, 3, writes.as_ptr(), 0, ptr::null());
        Frame {
            input,
            visible,
            draws,
            set: sets[f],
        }
    });

    // post processing: the scene color is sampled by a fullscreen pass into the swapchain
    let post_binds = [
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
    let post_set_layout: vk::DescriptorSetLayout = mk(CREATE_DESCRIPTOR_SET_LAYOUT_FAILED, |p| {
        vkCreateDescriptorSetLayout(
            device,
            &vk::DescriptorSetLayoutCreateInfo::default().bindings(&post_binds),
            NA,
            p,
        )
    });
    let post_set_layouts = [post_set_layout];
    let post_push = [vk::PushConstantRange::default()
        .stage_flags(vk::ShaderStageFlags::FRAGMENT)
        .size(size_of::<[f32; 4]>() as u32)];
    let post_layout: vk::PipelineLayout = mk(CREATE_PIPELINE_LAYOUT_FAILED, |p| {
        vkCreatePipelineLayout(
            device,
            &vk::PipelineLayoutCreateInfo::default()
                .set_layouts(&post_set_layouts)
                .push_constant_ranges(&post_push),
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
    let mut post_set = vk::DescriptorSet::default();
    ck(
        vkAllocateDescriptorSets(
            device,
            &vk::DescriptorSetAllocateInfo::default()
                .descriptor_pool(dpool)
                .set_layouts(&post_set_layouts),
            &mut post_set,
        ),
        ALLOCATE_DESCRIPTOR_SETS_FAILED,
    );
    write_post_set(&ctx, post_set, sampler, sc.scene.view);

    let pool: vk::CommandPool = mk(CREATE_COMMAND_POOL_FAILED, |p| {
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
        ALLOCATE_COMMAND_BUFFERS_FAILED,
    );
    let image_avail: [vk::Semaphore; FRAMES] = core::array::from_fn(|_| {
        mk(CREATE_SEMAPHORE_FAILED, |p| {
            vkCreateSemaphore(device, &vk::SemaphoreCreateInfo::default(), NA, p)
        })
    });
    let fences: [vk::Fence; FRAMES] = core::array::from_fn(|_| {
        mk(CREATE_FENCE_FAILED, |p| {
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
    let start = last;
    let center = |e: vk::Extent2D| [(e.width / 2) as i32, (e.height / 2) as i32];
    let mut post_pipe: Option<(&'static Align4<[u8]>, vk::Pipeline)> = None;

    loop {
        if !platform::poll(&w, &mut resized, &mut keys) {
            break;
        }
        let now = platform::now();
        let dt = (now - last) as f32;
        last = now;
        let c = center(sc.extent);
        let delta = if app.world.resource::<MouseCapture>().unwrap().0 {
            platform::mouse_delta(&w, c[0], c[1])
        } else {
            [0.0; 2]
        };
        let input = app.world.resource_mut::<Input>().unwrap();
        (input.prev, input.now) = (input.now, keys);
        input.delta = delta;
        app.run_frame(dt);

        ck(
            vkWaitForFences(device, 1, &fences[frame], vk::TRUE, u64::MAX),
            WAIT_FOR_FENCES_FAILED,
        );

        if resized {
            ck(vkDeviceWaitIdle(device), WAIT_FOR_DEVICE_IDLE_FAILED);
            let new = make_swapchain(&ctx, sc.handle);
            destroy_swapchain(&ctx, &sc);
            sc = new;
            write_post_set(&ctx, post_set, sampler, sc.scene.view);
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
            _ => die(ACQUIRE_NEXT_IMAGE_FAILED),
        }
        let idx = idx as usize;
        ck(
            vkResetFences(device, 1, &fences[frame]),
            RESET_FENCES_FAILED,
        );

        let cmd = cmds[frame];
        ck(
            vkResetCommandBuffer(cmd, vk::CommandBufferResetFlags::empty()),
            RESET_COMMAND_BUFFER_FAILED,
        );
        ck(
            vkBeginCommandBuffer(
                cmd,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            ),
            BEGIN_COMMAND_BUFFER_FAILED,
        );

        // ---- collect drawables, then cull them on the GPU
        let fr = &frames[frame];
        let world = &app.world;
        let meshes = world.resource::<Assets<Mesh>>().unwrap();
        let mats = world.resource::<Assets<Material>>().unwrap();
        let cam = *world.resource::<Camera>().unwrap();
        let (ew, eh) = (sc.extent.width as f32, sc.extent.height as f32);
        let vp = view_proj(&cam, ew / eh);

        let mut groups: Vec<(usize, usize)> = Vec::new(); // (mesh, material) per group
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
            let t = world.get::<Transform>(e).unwrap();
            let c = world.get::<Color>(e).map_or([1.0; 3], |c| c.0);
            fr.input.ptr.cast::<Inst>().add(n).write(Inst {
                a: [t.pos[0], t.pos[1], t.pos[2], t.yaw],
                s: [t.scale[0], t.scale[1], t.scale[2], g as f32],
                color: [c[0], c[1], c[2], meshes.0[mi].radius2],
            });
            n += 1;
        }
        for (g, &(mi, _)) in groups.iter().enumerate() {
            let m = slot(&mut gpu_meshes, mi, || upload_mesh(&ctx, &meshes.0[mi]));
            // instanceCount starts at 0; the culling shader counts survivors
            fr.draws
                .ptr
                .cast::<[u32; 4]>()
                .add(g)
                .write([m.count, 0, 0, 0]);
        }
        if n > 0 {
            vkCmdBindPipeline(cmd, vk::PipelineBindPoint::COMPUTE, cull_pipe);
            vkCmdBindDescriptorSets(
                cmd,
                vk::PipelineBindPoint::COMPUTE,
                cull_layout,
                0,
                1,
                &fr.set,
                0,
                ptr::null(),
            );
            let pc = CullPc {
                vp,
                count: n as u32,
                _pad: [0; 3],
            };
            vkCmdPushConstants(
                cmd,
                cull_layout,
                vk::ShaderStageFlags::COMPUTE,
                0,
                size_of::<CullPc>() as u32,
                ptr::from_ref(&pc).cast(),
            );
            vkCmdDispatch(cmd, (n as u32).div_ceil(64), 1, 1);
            let barrier = [vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                .dst_access_mask(
                    vk::AccessFlags::INDIRECT_COMMAND_READ | vk::AccessFlags::VERTEX_ATTRIBUTE_READ,
                )];
            vkCmdPipelineBarrier(
                cmd,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::PipelineStageFlags::DRAW_INDIRECT | vk::PipelineStageFlags::VERTEX_INPUT,
                vk::DependencyFlags::empty(),
                1,
                barrier.as_ptr(),
                0,
                ptr::null(),
                0,
                ptr::null(),
            );
        }

        // ---- scene pass: offscreen color + depth
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
            sc.scene.image,
            vk::ImageAspectFlags::COLOR,
            (
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            ),
            // waits for the previous frame's post pass to finish reading this image
            (
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            ),
            (
                vk::AccessFlags::empty(),
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            ),
        );
        transition(
            cmd,
            sc.depth.image,
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
        let color = [vk::RenderingAttachmentInfo::default()
            .image_view(sc.scene.view)
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .clear_value(vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [cam.sky[0], cam.sky[1], cam.sky[2], 1.0],
                },
            })];
        let depth = vk::RenderingAttachmentInfo::default()
            .image_view(sc.depth.view)
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
        for (g, &(mi, ma)) in groups.iter().enumerate() {
            // one pipeline bind per group; fine while groups are few.
            let pipe = *slot(&mut pipes, ma, || {
                make_pipeline(&ctx, layout, SCENE_FORMAT, &mats.0[ma])
            });
            let m = gpu_meshes[mi].as_ref().unwrap();
            vkCmdBindPipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pipe);
            vkCmdPushConstants(
                cmd,
                layout,
                vk::ShaderStageFlags::VERTEX,
                0,
                size_of::<[f32; 16]>() as u32,
                vp.as_ptr().cast(),
            );
            let bufs = [m.buf, fr.visible.buf];
            let offs = [0, (g * CAP * size_of::<Inst>()) as u64];
            vkCmdBindVertexBuffers(cmd, 0, 2, bufs.as_ptr(), offs.as_ptr());
            vkCmdDrawIndirect(cmd, fr.draws.buf, (g * size_of::<[u32; 4]>()) as u64, 1, 16);
        }
        vkCmdEndRendering(cmd);
        transition(
            cmd,
            sc.scene.image,
            vk::ImageAspectFlags::COLOR,
            (
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            ),
            (
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
            ),
            (
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
                vk::AccessFlags::SHADER_READ,
            ),
        );

        // ---- post pass: scene -> swapchain
        transition(
            cmd,
            sc.images[idx],
            vk::ImageAspectFlags::COLOR,
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
        let target = [vk::RenderingAttachmentInfo::default()
            .image_view(sc.views[idx])
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
        let shader = world.resource::<PostProcess>().unwrap().0.unwrap_or(POST);
        // swapping the shader at runtime leaks the old pipeline.
        let post = match post_pipe {
            Some((s, p)) if ptr::eq(s, shader) => p,
            _ => {
                let p = make_post_pipeline(&ctx, post_layout, sc.format, shader);
                post_pipe = Some((shader, p));
                p
            }
        };
        vkCmdBindPipeline(cmd, vk::PipelineBindPoint::GRAPHICS, post);
        vkCmdBindDescriptorSets(
            cmd,
            vk::PipelineBindPoint::GRAPHICS,
            post_layout,
            0,
            1,
            &post_set,
            0,
            ptr::null(),
        );
        let params = [ew, eh, (now - start) as f32, 0.0];
        vkCmdPushConstants(
            cmd,
            post_layout,
            vk::ShaderStageFlags::FRAGMENT,
            0,
            size_of::<[f32; 4]>() as u32,
            params.as_ptr().cast(),
        );
        vkCmdDraw(cmd, 3, 1, 0, 0);
        vkCmdEndRendering(cmd);
        transition(
            cmd,
            sc.images[idx],
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
        ck(vkEndCommandBuffer(cmd), END_COMMAND_BUFFER_FAILED);

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
            QUEUE_SUBMIT_FAILED,
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
            _ => die(QUEUE_PRESENT_FAILED),
        }

        if !shown {
            platform::show(&w);
            shown = true;
        }

        frame = (frame + 1) % FRAMES;
    }

    // no teardown, the OS reclaims everything on exit
}
