#![no_std]

use ash::vk;
use core::{ffi::c_void, ptr};

pub type A = *const vk::AllocationCallbacks<'static>;
pub const NA: A = ptr::null();

// Direct imports from the loader: ash's Device/Instance tables look up ~200
// functions by name, we only need these.
#[cfg_attr(
    unix,
    link(name = "libvulkan.so.1", kind = "dylib", modifiers = "+verbatim")
)]
#[cfg_attr(windows, link(name = "vulkan-1", kind = "raw-dylib"))]
unsafe extern "system" {
    pub fn vkCreateInstance(
        i: *const vk::InstanceCreateInfo<'_>,
        a: A,
        o: *mut vk::Instance,
    ) -> vk::Result;
    #[cfg(unix)]
    pub fn vkCreateXcbSurfaceKHR(
        i: vk::Instance,
        c: *const vk::XcbSurfaceCreateInfoKHR<'_>,
        a: A,
        o: *mut vk::SurfaceKHR,
    ) -> vk::Result;
    #[cfg(windows)]
    pub fn vkCreateWin32SurfaceKHR(
        i: vk::Instance,
        c: *const vk::Win32SurfaceCreateInfoKHR<'_>,
        a: A,
        o: *mut vk::SurfaceKHR,
    ) -> vk::Result;
    pub fn vkEnumeratePhysicalDevices(
        i: vk::Instance,
        n: *mut u32,
        o: *mut vk::PhysicalDevice,
    ) -> vk::Result;
    pub fn vkGetPhysicalDeviceProperties(
        p: vk::PhysicalDevice,
        o: *mut vk::PhysicalDeviceProperties,
    );
    pub fn vkGetPhysicalDeviceQueueFamilyProperties(
        p: vk::PhysicalDevice,
        n: *mut u32,
        o: *mut vk::QueueFamilyProperties,
    );
    pub fn vkGetPhysicalDeviceSurfaceSupportKHR(
        p: vk::PhysicalDevice,
        q: u32,
        s: vk::SurfaceKHR,
        o: *mut vk::Bool32,
    ) -> vk::Result;
    pub fn vkGetPhysicalDeviceSurfaceCapabilitiesKHR(
        p: vk::PhysicalDevice,
        s: vk::SurfaceKHR,
        o: *mut vk::SurfaceCapabilitiesKHR,
    ) -> vk::Result;
    pub fn vkGetPhysicalDeviceSurfaceFormatsKHR(
        p: vk::PhysicalDevice,
        s: vk::SurfaceKHR,
        n: *mut u32,
        o: *mut vk::SurfaceFormatKHR,
    ) -> vk::Result;
    pub fn vkCreateDevice(
        p: vk::PhysicalDevice,
        c: *const vk::DeviceCreateInfo<'_>,
        a: A,
        o: *mut vk::Device,
    ) -> vk::Result;
    pub fn vkGetDeviceQueue(d: vk::Device, f: u32, i: u32, o: *mut vk::Queue);
    pub fn vkCreateSwapchainKHR(
        d: vk::Device,
        c: *const vk::SwapchainCreateInfoKHR<'_>,
        a: A,
        o: *mut vk::SwapchainKHR,
    ) -> vk::Result;
    pub fn vkGetSwapchainImagesKHR(
        d: vk::Device,
        s: vk::SwapchainKHR,
        n: *mut u32,
        o: *mut vk::Image,
    ) -> vk::Result;
    pub fn vkDestroySwapchainKHR(d: vk::Device, s: vk::SwapchainKHR, a: A);
    pub fn vkCreateImageView(
        d: vk::Device,
        c: *const vk::ImageViewCreateInfo<'_>,
        a: A,
        o: *mut vk::ImageView,
    ) -> vk::Result;
    pub fn vkDestroyImageView(d: vk::Device, v: vk::ImageView, a: A);
    pub fn vkCreateSemaphore(
        d: vk::Device,
        c: *const vk::SemaphoreCreateInfo<'_>,
        a: A,
        o: *mut vk::Semaphore,
    ) -> vk::Result;
    pub fn vkDestroySemaphore(d: vk::Device, s: vk::Semaphore, a: A);
    pub fn vkCreateFence(
        d: vk::Device,
        c: *const vk::FenceCreateInfo<'_>,
        a: A,
        o: *mut vk::Fence,
    ) -> vk::Result;
    pub fn vkCreateShaderModule(
        d: vk::Device,
        c: *const vk::ShaderModuleCreateInfo<'_>,
        a: A,
        o: *mut vk::ShaderModule,
    ) -> vk::Result;
    pub fn vkDestroyShaderModule(d: vk::Device, m: vk::ShaderModule, a: A);
    pub fn vkCreatePipelineLayout(
        d: vk::Device,
        c: *const vk::PipelineLayoutCreateInfo<'_>,
        a: A,
        o: *mut vk::PipelineLayout,
    ) -> vk::Result;
    pub fn vkCreateGraphicsPipelines(
        d: vk::Device,
        cache: vk::PipelineCache,
        n: u32,
        c: *const vk::GraphicsPipelineCreateInfo<'_>,
        a: A,
        o: *mut vk::Pipeline,
    ) -> vk::Result;
    pub fn vkCreateCommandPool(
        d: vk::Device,
        c: *const vk::CommandPoolCreateInfo<'_>,
        a: A,
        o: *mut vk::CommandPool,
    ) -> vk::Result;
    pub fn vkAllocateCommandBuffers(
        d: vk::Device,
        c: *const vk::CommandBufferAllocateInfo<'_>,
        o: *mut vk::CommandBuffer,
    ) -> vk::Result;
    pub fn vkWaitForFences(
        d: vk::Device,
        n: u32,
        f: *const vk::Fence,
        all: vk::Bool32,
        t: u64,
    ) -> vk::Result;
    pub fn vkResetFences(d: vk::Device, n: u32, f: *const vk::Fence) -> vk::Result;
    pub fn vkDeviceWaitIdle(d: vk::Device) -> vk::Result;
    pub fn vkAcquireNextImageKHR(
        d: vk::Device,
        s: vk::SwapchainKHR,
        t: u64,
        sem: vk::Semaphore,
        f: vk::Fence,
        o: *mut u32,
    ) -> vk::Result;
    pub fn vkResetCommandBuffer(c: vk::CommandBuffer, f: vk::CommandBufferResetFlags)
    -> vk::Result;
    pub fn vkBeginCommandBuffer(
        c: vk::CommandBuffer,
        i: *const vk::CommandBufferBeginInfo<'_>,
    ) -> vk::Result;
    pub fn vkEndCommandBuffer(c: vk::CommandBuffer) -> vk::Result;
    pub fn vkCmdPipelineBarrier(
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
    pub fn vkCmdBeginRendering(c: vk::CommandBuffer, i: *const vk::RenderingInfo<'_>);
    pub fn vkCmdEndRendering(c: vk::CommandBuffer);
    pub fn vkCmdBindPipeline(c: vk::CommandBuffer, bp: vk::PipelineBindPoint, p: vk::Pipeline);
    pub fn vkCmdSetViewport(c: vk::CommandBuffer, first: u32, n: u32, v: *const vk::Viewport);
    pub fn vkCmdSetScissor(c: vk::CommandBuffer, first: u32, n: u32, r: *const vk::Rect2D);
    pub fn vkCmdDraw(c: vk::CommandBuffer, v: u32, i: u32, fv: u32, fi: u32);
    pub fn vkGetPhysicalDeviceMemoryProperties(
        p: vk::PhysicalDevice,
        o: *mut vk::PhysicalDeviceMemoryProperties,
    );
    pub fn vkCreateBuffer(
        d: vk::Device,
        c: *const vk::BufferCreateInfo<'_>,
        a: A,
        o: *mut vk::Buffer,
    ) -> vk::Result;
    pub fn vkGetBufferMemoryRequirements(
        d: vk::Device,
        b: vk::Buffer,
        o: *mut vk::MemoryRequirements,
    );
    pub fn vkAllocateMemory(
        d: vk::Device,
        c: *const vk::MemoryAllocateInfo<'_>,
        a: A,
        o: *mut vk::DeviceMemory,
    ) -> vk::Result;
    pub fn vkBindBufferMemory(
        d: vk::Device,
        b: vk::Buffer,
        m: vk::DeviceMemory,
        off: vk::DeviceSize,
    ) -> vk::Result;
    pub fn vkMapMemory(
        d: vk::Device,
        m: vk::DeviceMemory,
        off: vk::DeviceSize,
        size: vk::DeviceSize,
        f: vk::MemoryMapFlags,
        o: *mut *mut c_void,
    ) -> vk::Result;
    pub fn vkCmdBindVertexBuffers(
        c: vk::CommandBuffer,
        first: u32,
        n: u32,
        b: *const vk::Buffer,
        off: *const vk::DeviceSize,
    );
    pub fn vkCmdPushConstants(
        c: vk::CommandBuffer,
        l: vk::PipelineLayout,
        s: vk::ShaderStageFlags,
        off: u32,
        size: u32,
        v: *const c_void,
    );
    pub fn vkQueueSubmit(
        q: vk::Queue,
        n: u32,
        s: *const vk::SubmitInfo<'_>,
        f: vk::Fence,
    ) -> vk::Result;
    pub fn vkQueuePresentKHR(q: vk::Queue, i: *const vk::PresentInfoKHR<'_>) -> vk::Result;
}
