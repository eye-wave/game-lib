//! The frame: acquire, gather and cull drawables, draw the cameras, post-process, submit, present.
//! Everything optional lives in its own module (`cull`, `post`, `multicam`), so this file mentions no features.
use crate::{
    cams,
    cull::Cull,
    device,
    gpu::*,
    multicam::Cams,
    post::Post,
    scene::{Frame, Pass, Scene},
    swapchain::{Swapchain, destroy_swapchain, make_swapchain},
};
use ash::vk;
use ecs::App;
use errors::prelude::*;
use platform::die;
use vulkan_sys::*;

pub(crate) struct Renderer {
    ctx: Ctx,
    queue: vk::Queue,
    sc: Swapchain,
    scene: Scene,
    frames: [Frame; FRAMES],
    cull: Cull,
    post: Post,
    cams: Cams,
    cmds: [vk::CommandBuffer; FRAMES],
    image_avail: [vk::Semaphore; FRAMES],
    fences: [vk::Fence; FRAMES],
    frame: usize,
}

impl Renderer {
    /// `make_surface` turns the new Vulkan instance into the window's surface.
    pub unsafe fn new(make_surface: impl FnOnce(vk::Instance) -> vk::SurfaceKHR) -> Self {
        let (ctx, queue, qf) = device::create(make_surface);
        let device = ctx.device;
        let sc = make_swapchain(&ctx, vk::SwapchainKHR::null());
        let scene = Scene::new(&ctx);
        let frames: [Frame; FRAMES] = core::array::from_fn(|_| Frame::new(&ctx));
        let cull = Cull::new(&ctx, &frames);
        let post = Post::new(&ctx, &sc);

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
        Self {
            ctx,
            queue,
            sc,
            scene,
            frames,
            cull,
            post,
            cams: Cams::new(),
            cmds,
            image_avail,
            fences,
            frame: 0,
        }
    }

    /// Window size in pixels, as of the last swapchain.
    #[cfg(feature = "mouse")]
    pub fn extent(&self) -> vk::Extent2D {
        self.sc.extent
    }

    /// Renders and presents one frame; `false` if the swapchain was out of date and nothing was drawn.
    /// `resized` is read and cleared here, and set when presenting says the swapchain is stale.
    /// `time` is seconds since start (for the post shader).
    pub unsafe fn draw(&mut self, app: &App, resized: &mut bool, time: f32) -> bool {
        let device = self.ctx.device;
        let f = self.frame;
        ck(
            vkWaitForFences(device, 1, &self.fences[f], vk::TRUE, u64::MAX),
            WAIT_FOR_FENCES_FAILED,
        );

        if *resized {
            ck(vkDeviceWaitIdle(device), WAIT_FOR_DEVICE_IDLE_FAILED);
            let new = make_swapchain(&self.ctx, self.sc.handle);
            destroy_swapchain(&self.ctx, &self.sc);
            self.sc = new;
            self.post.resize(&self.ctx, &self.sc);
            *resized = false;
        }

        let mut idx = 0u32;
        match vkAcquireNextImageKHR(
            device,
            self.sc.handle,
            u64::MAX,
            self.image_avail[f],
            vk::Fence::null(),
            &mut idx,
        ) {
            vk::Result::SUCCESS | vk::Result::SUBOPTIMAL_KHR => {}
            vk::Result::ERROR_OUT_OF_DATE_KHR => {
                *resized = true;
                return false;
            }
            _ => die(ACQUIRE_NEXT_IMAGE_FAILED),
        }
        let idx = idx as usize;
        ck(
            vkResetFences(device, 1, &self.fences[f]),
            RESET_FENCES_FAILED,
        );

        let cmd = self.cmds[f];
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

        // collect drawables, then cull them on the GPU
        let (ew, eh) = (self.sc.extent.width as f32, self.sc.extent.height as f32);
        let views = cams::collect(app, ew, eh);
        let fr = &self.frames[f];
        let (groups, n) = self
            .scene
            .upload(&self.ctx, app, fr, &self.cull, views.len());
        self.cull.dispatch(cmd, f, &views, n);

        self.scene.tex.record(&self.ctx, cmd, app); // upload new textures (and the Atlas)

        // draw every camera into the output image, then post-process / present it
        let pass = Pass {
            c: &self.ctx,
            cmd,
            app,
            fr,
            f,
            cull: &self.cull,
            sc: &self.sc,
            idx,
            views: &views,
            groups: &groups,
        };
        self.cams.render(&mut self.scene, &pass);
        self.post.record(&pass, time);
        ck(vkEndCommandBuffer(cmd), END_COMMAND_BUFFER_FAILED);

        let wait = [self.image_avail[f]];
        let wait_stages = [acquire_stages()];
        let cbs = [cmd];
        let signal = [self.sc.render_done[idx]];
        ck(
            vkQueueSubmit(
                self.queue,
                1,
                &vk::SubmitInfo::default()
                    .wait_semaphores(&wait)
                    .wait_dst_stage_mask(&wait_stages)
                    .command_buffers(&cbs)
                    .signal_semaphores(&signal),
                self.fences[f],
            ),
            QUEUE_SUBMIT_FAILED,
        );

        let swapchains = [self.sc.handle];
        let indices = [idx as u32];
        match vkQueuePresentKHR(
            self.queue,
            &vk::PresentInfoKHR::default()
                .wait_semaphores(&signal)
                .swapchains(&swapchains)
                .image_indices(&indices),
        ) {
            vk::Result::SUCCESS => {}
            vk::Result::SUBOPTIMAL_KHR | vk::Result::ERROR_OUT_OF_DATE_KHR => *resized = true,
            _ => die(QUEUE_PRESENT_FAILED),
        }

        self.frame = (f + 1) % FRAMES;
        true
    }
}
