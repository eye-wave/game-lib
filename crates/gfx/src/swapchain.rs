//! The swapchain and the images that live and die with it.
use crate::gpu::*;
use alloc::vec::Vec;
use ash::vk;
use errors::prelude::*;
use vulkan_sys::*;

pub(crate) struct Swapchain {
    pub handle: vk::SwapchainKHR,
    pub format: vk::Format,
    pub extent: vk::Extent2D,
    pub images: Vec<vk::Image>,
    pub views: Vec<vk::ImageView>,
    pub render_done: Vec<vk::Semaphore>,
    #[cfg(feature = "post")]
    pub scene: Target, // offscreen color the cameras draw into, sampled by the post pass
    #[cfg(not(feature = "multicam"))]
    pub depth: Target, // the one camera draws straight into `out`, so it needs depth here
}

impl Swapchain {
    /// The image the cameras end up in: the offscreen scene with `post`, else the swapchain image itself.
    #[cfg(feature = "post")]
    pub fn out(&self, _idx: usize) -> (vk::Image, vk::ImageView) {
        (self.scene.image, self.scene.view)
    }
    #[cfg(not(feature = "post"))]
    pub fn out(&self, idx: usize) -> (vk::Image, vk::ImageView) {
        (self.images[idx], self.views[idx])
    }

    /// Color format the scene pipelines render in.
    pub fn draw_format(&self) -> vk::Format {
        if cfg!(feature = "multicam") || cfg!(feature = "post") {
            SCENE_FORMAT
        } else {
            self.format
        }
    }
}

pub(crate) unsafe fn make_swapchain(c: &Ctx, old: vk::SwapchainKHR) -> Swapchain {
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

    // without `post` the cameras write the swapchain image directly; with `multicam` that is by blit
    let mut usage = vk::ImageUsageFlags::COLOR_ATTACHMENT;
    if !cfg!(feature = "post") && cfg!(feature = "multicam") {
        usage |= vk::ImageUsageFlags::TRANSFER_DST;
    }
    let info = vk::SwapchainCreateInfoKHR::default()
        .surface(c.surface)
        .min_image_count(count)
        .image_format(fmt.format)
        .image_color_space(fmt.color_space)
        .image_extent(extent)
        .image_array_layers(1)
        .image_usage(usage)
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
        #[cfg(feature = "post")]
        scene: make_target(
            c,
            extent,
            SCENE_FORMAT,
            // cameras blit into it (`multicam`) or draw into it, then the post pass samples it
            vk::ImageUsageFlags::SAMPLED
                | if cfg!(feature = "multicam") {
                    vk::ImageUsageFlags::TRANSFER_DST
                } else {
                    vk::ImageUsageFlags::COLOR_ATTACHMENT
                },
            vk::ImageAspectFlags::COLOR,
        ),
        #[cfg(not(feature = "multicam"))]
        depth: make_target(
            c,
            extent,
            DEPTH_FORMAT,
            vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
            vk::ImageAspectFlags::DEPTH,
        ),
    }
}

pub(crate) unsafe fn destroy_swapchain(c: &Ctx, sc: &Swapchain) {
    for &s in &sc.render_done {
        vkDestroySemaphore(c.device, s, NA);
    }
    for &v in &sc.views {
        vkDestroyImageView(c.device, v, NA);
    }
    #[cfg(feature = "post")]
    destroy_target(c, &sc.scene);
    #[cfg(not(feature = "multicam"))]
    destroy_target(c, &sc.depth);
    vkDestroySwapchainKHR(c.device, sc.handle, NA);
}
