//! Fatal-error codes: `die(errors::X)` shows only the short code, so no message text ends up in the binary.
//! This file is the lookup table; 1xx setup, 2xx resources, 3xx rendering, 4xx linux, 5xx windows, 6xx engine.
#![no_std]

pub mod vulkan_setup {
    use core::ffi::CStr;

    pub const CREATE_VULKAN_INSTANCE_FAILED_NO_DRIVER: &CStr = c"error101";
    pub const ENUMERATE_PHYSICAL_DEVICES_FAILED: &CStr = c"error102";
    pub const NO_VULKAN_1_3_GPU_THAT_CAN_PRESENT_TO_THIS_WINDOW: &CStr = c"error103";
    pub const CREATE_LOGICAL_DEVICE_FAILED: &CStr = c"error104";
    pub const CREATE_SURFACE_FAILED: &CStr = c"error105";
    pub const GET_SURFACE_CAPABILITIES_FAILED: &CStr = c"error106";
    pub const GET_SURFACE_FORMATS_FAILED: &CStr = c"error107";
}

pub mod vulkan_resources {
    use core::ffi::CStr;

    pub const CREATE_SHADER_MODULE_FAILED: &CStr = c"error201";
    pub const CREATE_SWAPCHAIN_FAILED: &CStr = c"error202";
    pub const GET_SWAPCHAIN_IMAGES_FAILED: &CStr = c"error203";
    pub const CREATE_IMAGE_VIEW_FAILED: &CStr = c"error204";
    pub const CREATE_BUFFER_FAILED: &CStr = c"error205";
    pub const NO_SUITABLE_MEMORY_TYPE: &CStr = c"error206";
    pub const ALLOCATE_MEMORY_FAILED: &CStr = c"error207";
    pub const BIND_BUFFER_MEMORY_FAILED: &CStr = c"error208";
    pub const MAP_MEMORY_FAILED: &CStr = c"error209";
    pub const CREATE_PIPELINE_LAYOUT_FAILED: &CStr = c"error210";
    pub const CREATE_COMMAND_POOL_FAILED: &CStr = c"error211";
    pub const ALLOCATE_COMMAND_BUFFERS_FAILED: &CStr = c"error212";
    pub const CREATE_SEMAPHORE_FAILED: &CStr = c"error213";
    pub const CREATE_FENCE_FAILED: &CStr = c"error214";
    pub const CREATE_DESCRIPTOR_SET_LAYOUT_FAILED: &CStr = c"error215";
    pub const CREATE_DESCRIPTOR_POOL_FAILED: &CStr = c"error216";
    pub const ALLOCATE_DESCRIPTOR_SETS_FAILED: &CStr = c"error217";
}

pub mod vulkan_rendering {
    use core::ffi::CStr;

    pub const CREATE_GRAPHICS_PIPELINES_FAILED: &CStr = c"error301";
    pub const WAIT_FOR_FENCES_FAILED: &CStr = c"error302";
    pub const WAIT_FOR_DEVICE_IDLE_FAILED: &CStr = c"error303";
    pub const ACQUIRE_NEXT_IMAGE_FAILED: &CStr = c"error304";
    pub const RESET_FENCES_FAILED: &CStr = c"error305";
    pub const RESET_COMMAND_BUFFER_FAILED: &CStr = c"error306";
    pub const BEGIN_COMMAND_BUFFER_FAILED: &CStr = c"error307";
    pub const END_COMMAND_BUFFER_FAILED: &CStr = c"error308";
    pub const QUEUE_SUBMIT_FAILED: &CStr = c"error309";
    pub const QUEUE_PRESENT_FAILED: &CStr = c"error310";
    pub const CREATE_COMPUTE_PIPELINES_FAILED: &CStr = c"error311";
}

pub mod linux_xcb {
    use core::ffi::CStr;

    pub const CANNOT_CONNECT_TO_X_SERVER_DISPLAY_NOT_SET: &CStr = c"error401";
    pub const XCB_INTERN_ATOM_FAILED: &CStr = c"error402";
}

pub mod windows {
    use core::ffi::CStr;

    pub const REGISTER_CLASS_FAILED: &CStr = c"error501";
    pub const CREATE_WINDOW_FAILED: &CStr = c"error502";
}

pub mod game_engine {
    use core::ffi::CStr;

    pub const MATERIAL_ATTRIBUTE_FORMAT_UNSUPPORTED_ADD_ITS_SIZE_IN_MAKE_PIPELINE: &CStr =
        c"error601";
    pub const MATERIAL_HAS_MORE_THAN_4_VERTEX_ATTRIBUTES: &CStr = c"error602";
}

pub mod unknown {
    use core::ffi::CStr;

    pub const UNKNOWN_OR_UNCLASSIFIED_ERROR: &CStr = c"error999";
}

pub mod prelude {
    pub use super::game_engine::*;
    pub use super::linux_xcb::*;
    pub use super::unknown::*;
    pub use super::vulkan_rendering::*;
    pub use super::vulkan_resources::*;
    pub use super::vulkan_setup::*;
    pub use super::windows::*;
}
