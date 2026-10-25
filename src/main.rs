#![allow(unsafe_op_in_unsafe_fn)]
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use ash::{khr, vk};
use core::{
    alloc::{GlobalAlloc, Layout},
    ffi::{CStr, c_char, c_int, c_void},
    ptr,
};

#[link(name = "c")]
unsafe extern "C" {}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { libc::abort() }
}

struct Malloc;

unsafe impl GlobalAlloc for Malloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if l.align() <= 16 {
            libc::malloc(l.size()) as *mut u8
        } else {
            let mut p: *mut c_void = ptr::null_mut();
            libc::posix_memalign(&mut p, l.align(), l.size());
            p as *mut u8
        }
    }
    unsafe fn dealloc(&self, p: *mut u8, _: Layout) {
        libc::free(p as *mut c_void)
    }
}

#[global_allocator]
static ALLOC: Malloc = Malloc;

fn die(msg: &CStr) -> ! {
    unsafe {
        libc::puts(msg.as_ptr());
        libc::exit(1)
    }
}

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

type A = *const vk::AllocationCallbacks<'static>;
const NA: A = ptr::null();

// Direct imports from the loader: ash's Device/Instance tables look up ~200
// functions by name, we only need these.
#[link(name = "libvulkan.so.1", kind = "dylib", modifiers = "+verbatim")]
unsafe extern "system" {
    fn vkCreateInstance(
        i: *const vk::InstanceCreateInfo<'_>,
        a: A,
        o: *mut vk::Instance,
    ) -> vk::Result;
    fn vkCreateXcbSurfaceKHR(
        i: vk::Instance,
        c: *const vk::XcbSurfaceCreateInfoKHR<'_>,
        a: A,
        o: *mut vk::SurfaceKHR,
    ) -> vk::Result;
    fn vkEnumeratePhysicalDevices(
        i: vk::Instance,
        n: *mut u32,
        o: *mut vk::PhysicalDevice,
    ) -> vk::Result;
    fn vkGetPhysicalDeviceProperties(p: vk::PhysicalDevice, o: *mut vk::PhysicalDeviceProperties);
    fn vkGetPhysicalDeviceQueueFamilyProperties(
        p: vk::PhysicalDevice,
        n: *mut u32,
        o: *mut vk::QueueFamilyProperties,
    );
    fn vkGetPhysicalDeviceSurfaceSupportKHR(
        p: vk::PhysicalDevice,
        q: u32,
        s: vk::SurfaceKHR,
        o: *mut vk::Bool32,
    ) -> vk::Result;
    fn vkGetPhysicalDeviceSurfaceCapabilitiesKHR(
        p: vk::PhysicalDevice,
        s: vk::SurfaceKHR,
        o: *mut vk::SurfaceCapabilitiesKHR,
    ) -> vk::Result;
    fn vkGetPhysicalDeviceSurfaceFormatsKHR(
        p: vk::PhysicalDevice,
        s: vk::SurfaceKHR,
        n: *mut u32,
        o: *mut vk::SurfaceFormatKHR,
    ) -> vk::Result;
    fn vkCreateDevice(
        p: vk::PhysicalDevice,
        c: *const vk::DeviceCreateInfo<'_>,
        a: A,
        o: *mut vk::Device,
    ) -> vk::Result;
    fn vkGetDeviceQueue(d: vk::Device, f: u32, i: u32, o: *mut vk::Queue);
    fn vkCreateSwapchainKHR(
        d: vk::Device,
        c: *const vk::SwapchainCreateInfoKHR<'_>,
        a: A,
        o: *mut vk::SwapchainKHR,
    ) -> vk::Result;
    fn vkGetSwapchainImagesKHR(
        d: vk::Device,
        s: vk::SwapchainKHR,
        n: *mut u32,
        o: *mut vk::Image,
    ) -> vk::Result;
    fn vkDestroySwapchainKHR(d: vk::Device, s: vk::SwapchainKHR, a: A);
    fn vkCreateImageView(
        d: vk::Device,
        c: *const vk::ImageViewCreateInfo<'_>,
        a: A,
        o: *mut vk::ImageView,
    ) -> vk::Result;
    fn vkDestroyImageView(d: vk::Device, v: vk::ImageView, a: A);
    fn vkCreateSemaphore(
        d: vk::Device,
        c: *const vk::SemaphoreCreateInfo<'_>,
        a: A,
        o: *mut vk::Semaphore,
    ) -> vk::Result;
    fn vkDestroySemaphore(d: vk::Device, s: vk::Semaphore, a: A);
    fn vkCreateFence(
        d: vk::Device,
        c: *const vk::FenceCreateInfo<'_>,
        a: A,
        o: *mut vk::Fence,
    ) -> vk::Result;
    fn vkCreateShaderModule(
        d: vk::Device,
        c: *const vk::ShaderModuleCreateInfo<'_>,
        a: A,
        o: *mut vk::ShaderModule,
    ) -> vk::Result;
    fn vkDestroyShaderModule(d: vk::Device, m: vk::ShaderModule, a: A);
    fn vkCreatePipelineLayout(
        d: vk::Device,
        c: *const vk::PipelineLayoutCreateInfo<'_>,
        a: A,
        o: *mut vk::PipelineLayout,
    ) -> vk::Result;
    fn vkCreateGraphicsPipelines(
        d: vk::Device,
        cache: vk::PipelineCache,
        n: u32,
        c: *const vk::GraphicsPipelineCreateInfo<'_>,
        a: A,
        o: *mut vk::Pipeline,
    ) -> vk::Result;
    fn vkCreateCommandPool(
        d: vk::Device,
        c: *const vk::CommandPoolCreateInfo<'_>,
        a: A,
        o: *mut vk::CommandPool,
    ) -> vk::Result;
    fn vkAllocateCommandBuffers(
        d: vk::Device,
        c: *const vk::CommandBufferAllocateInfo<'_>,
        o: *mut vk::CommandBuffer,
    ) -> vk::Result;
    fn vkWaitForFences(
        d: vk::Device,
        n: u32,
        f: *const vk::Fence,
        all: vk::Bool32,
        t: u64,
    ) -> vk::Result;
    fn vkResetFences(d: vk::Device, n: u32, f: *const vk::Fence) -> vk::Result;
    fn vkDeviceWaitIdle(d: vk::Device) -> vk::Result;
    fn vkAcquireNextImageKHR(
        d: vk::Device,
        s: vk::SwapchainKHR,
        t: u64,
        sem: vk::Semaphore,
        f: vk::Fence,
        o: *mut u32,
    ) -> vk::Result;
    fn vkResetCommandBuffer(c: vk::CommandBuffer, f: vk::CommandBufferResetFlags) -> vk::Result;
    fn vkBeginCommandBuffer(
        c: vk::CommandBuffer,
        i: *const vk::CommandBufferBeginInfo<'_>,
    ) -> vk::Result;
    fn vkEndCommandBuffer(c: vk::CommandBuffer) -> vk::Result;
    fn vkCmdPipelineBarrier(
        c: vk::CommandBuffer,
        s: vk::PipelineStageFlags,
        d: vk::PipelineStageFlags,
        f: vk::DependencyFlags,
        nm: u32,
        m: *const vk::MemoryBarrier<'_>,
        nb: u32,
        b: *const vk::BufferMemoryBarrier<'_>,
        ni: u32,
        i: *const vk::ImageMemoryBarrier<'_>,
    );
    fn vkCmdBeginRendering(c: vk::CommandBuffer, i: *const vk::RenderingInfo<'_>);
    fn vkCmdEndRendering(c: vk::CommandBuffer);
    fn vkCmdBindPipeline(c: vk::CommandBuffer, bp: vk::PipelineBindPoint, p: vk::Pipeline);
    fn vkCmdSetViewport(c: vk::CommandBuffer, first: u32, n: u32, v: *const vk::Viewport);
    fn vkCmdSetScissor(c: vk::CommandBuffer, first: u32, n: u32, r: *const vk::Rect2D);
    fn vkCmdDraw(c: vk::CommandBuffer, v: u32, i: u32, fv: u32, fi: u32);
    fn vkQueueSubmit(
        q: vk::Queue,
        n: u32,
        s: *const vk::SubmitInfo<'_>,
        f: vk::Fence,
    ) -> vk::Result;
    fn vkQueuePresentKHR(q: vk::Queue, i: *const vk::PresentInfoKHR<'_>) -> vk::Result;
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

type Conn = c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct Cookie {
    sequence: u32,
}
#[repr(C)]
struct Screen {
    root: u32,
}
#[repr(C)]
struct ScreenIter {
    data: *const Screen,
    rem: c_int,
    index: c_int,
}
#[repr(C)]
struct AtomReply {
    response_type: u8,
    pad0: u8,
    sequence: u16,
    length: u32,
    atom: u32,
}
#[repr(C)]
struct Event {
    response_type: u8,
    detail: u8,
    sequence: u16,
}
#[repr(C)]
struct ClientMsg {
    response_type: u8,
    format: u8,
    sequence: u16,
    window: u32,
    type_: u32,
    data: [u32; 5],
}

#[link(name = "libxcb.so.1", kind = "dylib", modifiers = "+verbatim")]
unsafe extern "C" {
    fn xcb_connect(display: *const c_char, screen: *mut c_int) -> *mut Conn;
    fn xcb_connection_has_error(c: *mut Conn) -> c_int;
    fn xcb_get_setup(c: *mut Conn) -> *const c_void;
    fn xcb_setup_roots_iterator(setup: *const c_void) -> ScreenIter;
    fn xcb_generate_id(c: *mut Conn) -> u32;
    fn xcb_create_window(
        c: *mut Conn,
        depth: u8,
        wid: u32,
        parent: u32,
        x: i16,
        y: i16,
        width: u16,
        height: u16,
        border_width: u16,
        class: u16,
        visual: u32,
        value_mask: u32,
        value_list: *const c_void,
    ) -> Cookie;
    fn xcb_change_property(
        c: *mut Conn,
        mode: u8,
        window: u32,
        property: u32,
        type_: u32,
        format: u8,
        data_len: u32,
        data: *const c_void,
    ) -> Cookie;
    fn xcb_intern_atom(c: *mut Conn, only_if_exists: u8, len: u16, name: *const c_char) -> Cookie;
    fn xcb_intern_atom_reply(c: *mut Conn, cookie: Cookie, err: *mut *mut c_void)
    -> *mut AtomReply;
    fn xcb_map_window(c: *mut Conn, window: u32) -> Cookie;
    fn xcb_flush(c: *mut Conn) -> c_int;
    fn xcb_poll_for_event(c: *mut Conn) -> *mut Event;
}

const XCB_CW_EVENT_MASK: u32 = 0x800;
const XCB_EVENT_MASK_KEY_PRESS: u32 = 1;
const XCB_EVENT_MASK_STRUCTURE_NOTIFY: u32 = 1 << 17;
const XCB_KEY_PRESS: u8 = 2;
const XCB_CONFIGURE_NOTIFY: u8 = 22;
const XCB_CLIENT_MESSAGE: u8 = 33;
const XCB_ATOM_ATOM: u32 = 4;
const XCB_ATOM_STRING: u32 = 31;
const XCB_ATOM_WM_NAME: u32 = 39;
const KEYCODE_ESCAPE: u8 = 9;

unsafe fn intern(c: *mut Conn, name: &str) -> u32 {
    let ck = xcb_intern_atom(c, 0, name.len() as u16, name.as_ptr() as *const c_char);
    let r = xcb_intern_atom_reply(c, ck, ptr::null_mut());
    if r.is_null() {
        die(c"xcb_intern_atom failed");
    }
    let atom = (*r).atom;
    libc::free(r as *mut c_void);
    atom
}

#[repr(C, align(4))]
struct Align4<T: ?Sized>(T);

macro_rules! include_shader {
    ($name:literal) => {
        &Align4(*include_bytes!(concat!(
            env!("OUT_DIR"),
            "/",
            $name,
            ".spv"
        )))
    };
}

static SPV: &Align4<[u8]> = include_shader!("tri");

fn words(b: &'static Align4<[u8]>) -> &'static [u32] {
    unsafe { core::slice::from_raw_parts(b.0.as_ptr() as *const u32, b.0.len() / 4) }
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

/// C entry point, called once by libc's startup code.
///
/// # Safety
///
/// Must only be called by the C runtime as the process entry point, at most
/// once. `_argc` and `_argv` are ignored, so they may be anything. The caller
/// must provide a working libc, X server (`DISPLAY`) and Vulkan 1.3 driver;
/// the function terminates the process via `die` when one of them is missing
/// rather than returning an error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn main(_argc: c_int, _argv: *const *const c_char) -> c_int {
    unsafe { run() };
    0
}

unsafe fn run() {
    let conn = xcb_connect(ptr::null(), ptr::null_mut());
    if conn.is_null() || xcb_connection_has_error(conn) != 0 {
        die(c"cannot connect to X server (is DISPLAY set?)");
    }
    let root = (*xcb_setup_roots_iterator(xcb_get_setup(conn)).data).root;

    let win = xcb_generate_id(conn);
    let event_mask = [XCB_EVENT_MASK_KEY_PRESS | XCB_EVENT_MASK_STRUCTURE_NOTIFY];
    xcb_create_window(
        conn,
        0,
        win,
        root,
        0,
        0,
        WIDTH,
        HEIGHT,
        0,
        1,
        0,
        XCB_CW_EVENT_MASK,
        event_mask.as_ptr() as *const c_void,
    );

    let title = b"Vulkan triangle";
    xcb_change_property(
        conn,
        0,
        win,
        XCB_ATOM_WM_NAME,
        XCB_ATOM_STRING,
        8,
        title.len() as u32,
        title.as_ptr() as *const c_void,
    );
    let wm_protocols = intern(conn, "WM_PROTOCOLS");
    let wm_delete = intern(conn, "WM_DELETE_WINDOW");
    xcb_change_property(
        conn,
        0,
        win,
        wm_protocols,
        XCB_ATOM_ATOM,
        32,
        1,
        &wm_delete as *const u32 as *const c_void,
    );
    xcb_map_window(conn, win);
    xcb_flush(conn);

    let app = vk::ApplicationInfo::default().api_version(vk::API_VERSION_1_3);
    let inst_exts = [khr::surface::NAME.as_ptr(), khr::xcb_surface::NAME.as_ptr()];
    let instance: vk::Instance = mk(c"create_instance failed (no Vulkan driver?)", |p| {
        vkCreateInstance(
            &vk::InstanceCreateInfo::default()
                .application_info(&app)
                .enabled_extension_names(&inst_exts),
            NA,
            p,
        )
    });

    let surface: vk::SurfaceKHR = mk(c"create_xcb_surface failed", |p| {
        vkCreateXcbSurfaceKHR(
            instance,
            &vk::XcbSurfaceCreateInfoKHR::default()
                .connection(conn as *mut _)
                .window(win),
            NA,
            p,
        )
    });

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

    let shader = shader_module(device, SPV);
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

    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
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

    let color_formats = [sc.format];
    let mut rendering =
        vk::PipelineRenderingCreateInfo::default().color_attachment_formats(&color_formats);

    let layout: vk::PipelineLayout = mk(c"create_pipeline_layout failed", |p| {
        vkCreatePipelineLayout(device, &vk::PipelineLayoutCreateInfo::default(), NA, p)
    });
    let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
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
            device,
            vk::PipelineCache::null(),
            1,
            &pipeline_info,
            NA,
            &mut pipeline,
        ),
        c"create_graphics_pipelines failed",
    );
    vkDestroyShaderModule(device, shader, NA);

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

    let mut running = true;
    let mut resized = false;
    let mut frame = 0usize;

    while running {
        loop {
            let e = xcb_poll_for_event(conn);
            if e.is_null() {
                break;
            }
            match (*e).response_type & 0x7f {
                XCB_CLIENT_MESSAGE => {
                    if (*(e as *const ClientMsg)).data[0] == wm_delete {
                        running = false;
                    }
                }
                XCB_CONFIGURE_NOTIFY => resized = true,
                XCB_KEY_PRESS if (*e).detail == KEYCODE_ESCAPE => {
                    running = false;
                }
                _ => {}
            }
            libc::free(e as *mut c_void);
        }
        if !running {
            break;
        }

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
        vkCmdBindPipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pipeline);
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
        vkCmdDraw(cmd, 3, 1, 0, 0);
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

        frame = (frame + 1) % FRAMES;
    }

    // no teardown, the OS reclaims everything on exit
}
