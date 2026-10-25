#![allow(unsafe_op_in_unsafe_fn)]
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use ash::{khr, vk};
use core::{ffi::CStr, ptr};
use platform::die;
use shader::{Align4, include_shader, words};
use vulkan_sys::*;

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

static SPV: &Align4<[u8]> = include_shader!("tri");

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

unsafe fn run() {
    let w = platform::open(WIDTH, HEIGHT, c"Vulkan triangle");

    let app = vk::ApplicationInfo::default().api_version(vk::API_VERSION_1_3);
    let inst_exts = [khr::surface::NAME.as_ptr(), platform::SURFACE_EXT.as_ptr()];
    let instance: vk::Instance = mk(c"create_instance failed (no Vulkan driver?)", |p| {
        vkCreateInstance(
            &vk::InstanceCreateInfo::default()
                .application_info(&app)
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

    let mut resized = false;
    let mut shown = false;
    let mut frame = 0usize;

    loop {
        if !platform::poll(&w, &mut resized) {
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

        if !shown {
            platform::show(&w);
            shown = true;
        }

        frame = (frame + 1) % FRAMES;
    }

    // no teardown, the OS reclaims everything on exit
}

platform::entry!(run);
