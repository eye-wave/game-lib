//! Fatal-error codes: `die(errors::X)` shows only the short code, so no message text ends up in the binary.
//! This file is the lookup table; 1xx setup, 2xx resources, 3xx rendering, 4xx linux, 5xx windows, 6xx engine.
#![no_std]

/// `module { NAME = 101, ... }` -> `pub mod module { pub const NAME: &CStr = c"error101"; }`, plus a prelude.
macro_rules! codes {
    ($($m:ident { $($name:ident = $n:literal),+ $(,)? })+) => {
        $(
            pub mod $m {
                use core::ffi::CStr;
                $(
                    pub const $name: &CStr =
                        match CStr::from_bytes_with_nul(concat!("error", $n, "\0").as_bytes()) {
                            Ok(c) => c,
                            Err(_) => panic!(),
                        };
                )+
            }
        )+
        pub mod prelude {
            $(pub use super::$m::*;)+
        }
    };
}

codes! {
    vulkan_setup {
        CREATE_VULKAN_INSTANCE_FAILED_NO_DRIVER = 101,
        ENUMERATE_PHYSICAL_DEVICES_FAILED = 102,
        NO_VULKAN_1_3_GPU_THAT_CAN_PRESENT_TO_THIS_WINDOW = 103,
        CREATE_LOGICAL_DEVICE_FAILED = 104,
        CREATE_SURFACE_FAILED = 105,
        GET_SURFACE_CAPABILITIES_FAILED = 106,
        GET_SURFACE_FORMATS_FAILED = 107,
    }
    vulkan_resources {
        CREATE_SHADER_MODULE_FAILED = 201,
        CREATE_SWAPCHAIN_FAILED = 202,
        GET_SWAPCHAIN_IMAGES_FAILED = 203,
        CREATE_IMAGE_VIEW_FAILED = 204,
        CREATE_BUFFER_FAILED = 205,
        NO_SUITABLE_MEMORY_TYPE = 206,
        ALLOCATE_MEMORY_FAILED = 207,
        BIND_BUFFER_MEMORY_FAILED = 208,
        MAP_MEMORY_FAILED = 209,
        CREATE_PIPELINE_LAYOUT_FAILED = 210,
        CREATE_COMMAND_POOL_FAILED = 211,
        ALLOCATE_COMMAND_BUFFERS_FAILED = 212,
        CREATE_SEMAPHORE_FAILED = 213,
        CREATE_FENCE_FAILED = 214,
        CREATE_DESCRIPTOR_SET_LAYOUT_FAILED = 215,
        CREATE_DESCRIPTOR_POOL_FAILED = 216,
        ALLOCATE_DESCRIPTOR_SETS_FAILED = 217,
        CREATE_IMAGE_FAILED = 218,
        BIND_IMAGE_MEMORY_FAILED = 219,
        CREATE_SAMPLER_FAILED = 220,
    }
    vulkan_rendering {
        CREATE_GRAPHICS_PIPELINES_FAILED = 301,
        WAIT_FOR_FENCES_FAILED = 302,
        WAIT_FOR_DEVICE_IDLE_FAILED = 303,
        ACQUIRE_NEXT_IMAGE_FAILED = 304,
        RESET_FENCES_FAILED = 305,
        RESET_COMMAND_BUFFER_FAILED = 306,
        BEGIN_COMMAND_BUFFER_FAILED = 307,
        END_COMMAND_BUFFER_FAILED = 308,
        QUEUE_SUBMIT_FAILED = 309,
        QUEUE_PRESENT_FAILED = 310,
        CREATE_COMPUTE_PIPELINES_FAILED = 311,
    }
    linux_xcb {
        CANNOT_CONNECT_TO_X_SERVER_DISPLAY_NOT_SET = 401,
        XCB_INTERN_ATOM_FAILED = 402,
    }
    windows {
        REGISTER_CLASS_FAILED = 501,
        CREATE_WINDOW_FAILED = 502,
    }
    game_engine {
        MATERIAL_ATTRIBUTE_FORMAT_UNSUPPORTED_ADD_ITS_SIZE_IN_MAKE_PIPELINE = 601,
        MATERIAL_HAS_MORE_THAN_4_VERTEX_ATTRIBUTES = 602,
        ALLOC_IN_AUDIO_THREAD = 603, // debug builds only
    }
    unknown {
        UNKNOWN_OR_UNCLASSIFIED_ERROR = 999,
    }
}
