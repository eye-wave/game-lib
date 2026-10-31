//! Fatal-error codes: `die(errors::X)` shows only the short code, so no message text ends up in the binary.
//! This file is the lookup table; 1xx setup, 2xx resources, 3xx rendering, 4xx linux, 5xx windows, 6xx engine, 7xx assets.
#![no_std]

#[cfg(feature = "generate-error-docs")]
extern crate std;

#[cfg(feature = "generate-error-docs")]
pub struct ErrorInfo {
    pub module: &'static str,
    pub name: &'static str,
    pub code: u16,
    pub description: &'static str,
}

macro_rules! codes {
    ($($m:ident {
        $($name:ident = $n:literal => $description:literal),+ $(,)?
    })+) => {
        $(
            pub mod $m {
                use core::ffi::CStr;

                $(
                    pub const $name: &CStr =
                        match CStr::from_bytes_with_nul(
                            concat!("error", $n, "\0").as_bytes()
                        ) {
                            Ok(c) => c,
                            Err(_) => panic!(),
                        };
                )+
            }
        )+

        pub mod prelude {
            $(pub use super::$m::*;)+
        }

        #[cfg(feature = "generate-error-docs")]
        pub const ALL_ERRORS: &[ErrorInfo] = &[
            $(
                $(
                    ErrorInfo {
                        module: stringify!($m),
                        name: stringify!($name),
                        code: $n,
                        description: $description,
                    },
                )+
            )+
        ];
    };
}

codes! {
    vulkan_setup {
        CREATE_VULKAN_INSTANCE_FAILED_NO_DRIVER = 101 => "vkCreateInstance failed: no Vulkan driver/loader installed, it is older than 1.3, or it lacks the surface extensions the window needs",
        ENUMERATE_PHYSICAL_DEVICES_FAILED = 102 => "listing the GPUs failed: broken driver or out of memory",
        NO_VULKAN_1_3_GPU_THAT_CAN_PRESENT_TO_THIS_WINDOW = 103 => "no GPU offers Vulkan 1.3 with a graphics queue that can present to this window: driver too old, software-only, or the window is on another GPU",
        CREATE_LOGICAL_DEVICE_FAILED = 104 => "vkCreateDevice failed: the GPU lacks VK_KHR_swapchain or Vulkan 1.3 dynamic rendering, or the driver errored",
        CREATE_SURFACE_FAILED = 105 => "Vulkan could not make a surface for the window: the xcb/win32 surface extension is missing or the window is invalid",
        GET_SURFACE_CAPABILITIES_FAILED = 106 => "asking the surface for its size limits failed: surface lost (window gone, display server dropped)",
        GET_SURFACE_FORMATS_FAILED = 107 => "listing the surface's pixel formats failed: same causes as 106",
    }
    vulkan_resources {
        CREATE_SHADER_MODULE_FAILED = 201 => "the driver rejected a SPIR-V module: corrupt shader build or a capability this GPU lacks",
        CREATE_SWAPCHAIN_FAILED = 202 => "swapchain creation failed: surface lost, unsupported size or format, or out of memory",
        GET_SWAPCHAIN_IMAGES_FAILED = 203 => "fetching the swapchain's images failed",
        CREATE_IMAGE_VIEW_FAILED = 204 => "creating a view of an image (swapchain, render target or depth) failed: out of memory or unsupported format",
        CREATE_BUFFER_FAILED = 205 => "vkCreateBuffer failed: out of memory, or a size past the device limit (too many meshes or instances)",
        NO_SUITABLE_MEMORY_TYPE = 206 => "no GPU memory type is both allowed for the resource and has the wanted properties (host-visible+coherent for CPU-written buffers, device-local otherwise)",
        ALLOCATE_MEMORY_FAILED = 207 => "out of GPU or host memory (or too many allocations)",
        BIND_BUFFER_MEMORY_FAILED = 208 => "attaching memory to a buffer failed: out of memory or driver error",
        MAP_MEMORY_FAILED = 209 => "mapping a CPU-visible buffer into the address space failed: out of memory or address space",
        CREATE_PIPELINE_LAYOUT_FAILED = 210 => "creating a pipeline layout (descriptor sets + push constants) failed: out of memory",
        CREATE_COMMAND_POOL_FAILED = 211 => "creating the command pool failed: out of memory",
        ALLOCATE_COMMAND_BUFFERS_FAILED = 212 => "allocating the frame command buffers failed: out of memory",
        CREATE_SEMAPHORE_FAILED = 213 => "creating a frame semaphore failed: out of memory",
        CREATE_FENCE_FAILED = 214 => "creating a frame fence failed: out of memory",
        CREATE_DESCRIPTOR_SET_LAYOUT_FAILED = 215 => "creating a descriptor set layout failed: out of memory",
        CREATE_DESCRIPTOR_POOL_FAILED = 216 => "creating the descriptor pool failed: out of memory",
        ALLOCATE_DESCRIPTOR_SETS_FAILED = 217 => "allocating descriptor sets failed: pool exhausted or out of memory",
        CREATE_IMAGE_FAILED = 218 => "creating an offscreen image (scene color or depth) failed: out of memory, or the window is bigger than the GPU's image limit",
        BIND_IMAGE_MEMORY_FAILED = 219 => "attaching memory to an image failed: out of memory or driver error",
        CREATE_SAMPLER_FAILED = 220 => "creating the sampler for the post-processing pass failed: out of memory",
    }
    vulkan_rendering {
        CREATE_GRAPHICS_PIPELINES_FAILED = 301 => "building a graphics pipeline failed: the driver rejected the shader or vertex layout of a Material, or ran out of memory",
        WAIT_FOR_FENCES_FAILED = 302 => "waiting for an earlier frame to finish failed: device lost (GPU reset or driver crash) or out of memory",
        WAIT_FOR_DEVICE_IDLE_FAILED = 303 => "waiting for the GPU to go idle (before a resize) failed: device lost",
        ACQUIRE_NEXT_IMAGE_FAILED = 304 => "getting the next swapchain image failed for a reason other than out-of-date (that one only rebuilds the swapchain): surface or device lost",
        RESET_FENCES_FAILED = 305 => "resetting a frame fence failed: out of memory or device lost",
        RESET_COMMAND_BUFFER_FAILED = 306 => "resetting a frame's command buffer failed: out of memory or device lost",
        BEGIN_COMMAND_BUFFER_FAILED = 307 => "starting to record a frame's commands failed: out of memory",
        END_COMMAND_BUFFER_FAILED = 308 => "finishing a frame's commands failed: an error hit while recording them (usually out of memory)",
        QUEUE_SUBMIT_FAILED = 309 => "submitting a frame to the GPU failed: device lost (GPU hang, reset or driver crash) or out of memory",
        QUEUE_PRESENT_FAILED = 310 => "showing a frame failed for a reason other than out-of-date or suboptimal (those only trigger a resize): surface or device lost, or out of memory",
        CREATE_COMPUTE_PIPELINES_FAILED = 311 => "building the culling compute pipeline failed: the driver rejected cull.wgsl's SPIR-V or ran out of memory",
    }
    linux_xcb {
        CANNOT_CONNECT_TO_X_SERVER_DISPLAY_NOT_SET = 401 => "xcb_connect failed: no X server to talk to (DISPLAY unset or wrong: headless, Wayland without XWayland, SSH without forwarding)",
        XCB_INTERN_ATOM_FAILED = 402 => "the X server did not answer an atom lookup (needed for the close button and title): the connection dropped",
    }
    windows {
        REGISTER_CLASS_FAILED = 501 => "RegisterClassA failed: the window class could not be registered (out of resources)",
        CREATE_WINDOW_FAILED = 502 => "CreateWindowExA failed: out of window handles or desktop memory, or there is no interactive desktop (running as a service)",
    }
    game_engine {
        MATERIAL_ATTRIBUTE_FORMAT_UNSUPPORTED_ADD_ITS_SIZE_IN_MAKE_PIPELINE = 601 => "a Material vertex attribute is not an R32..R32G32B32A32 SFLOAT format; add its byte size to the offset match in make_pipeline",
        MATERIAL_HAS_MORE_THAN_4_VERTEX_ATTRIBUTES = 602 => "Material.attrs has more than the 4 vertex attributes the engine supports",
        ALLOC_IN_AUDIO_THREAD = 603 => "debug builds only",
    }
    assets {
        READ_FILE_FAILED = 701 => "",
        IMAGE_DECODE_FAILED = 702 => "corrupt or truncated file",
        IMAGE_LIBRARY_MISSING = 703 => "system decoder (libpng, libturbojpeg, libwebp, GDI+, Windows WebP extension) not installed or too old",
        AUDIO_DECODE_FAILED = 704 => "corrupt file, or silent/empty",
        AUDIO_LIBRARY_MISSING = 705 => "libvorbisfile, libmpg123 or Media Foundation (plus the Windows Vorbis extension for ogg) not installed",
    }
    unknown {
        UNKNOWN_OR_UNCLASSIFIED_ERROR = 999 => ""
    }
}
