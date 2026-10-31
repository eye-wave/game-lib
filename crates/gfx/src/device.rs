//! Instance, surface, physical and logical device, queue.
use crate::gpu::*;
use ash::{khr, vk};
use errors::prelude::*;
use vulkan_sys::*;

/// `make_surface` turns the new instance into the window's surface.
pub(crate) unsafe fn create(
    make_surface: impl FnOnce(vk::Instance) -> vk::SurfaceKHR,
) -> (Ctx, vk::Queue, u32) {
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

    let surface = make_surface(instance);

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

    (
        Ctx {
            device,
            phys,
            surface,
        },
        queue,
        qf,
    )
}
