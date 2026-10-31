//! Feature `cull`: a compute shader (`cull.wgsl`) keeps, per camera, only the instances inside the frustum.
//! `cull_off.rs` is the same interface without it: every instance is drawn straight from the input buffer.
use crate::{
    cams::{MAX_CAMERAS, View},
    gpu::*,
    scene::{CAP, Frame, GROUPS, Inst},
};
use ash::vk;
use core::ptr;
use errors::prelude::*;
use shader::Align4;
use vulkan_sys::*;

static CULL: &Align4<[u8]> = shader::include_shader!("cull");

#[repr(C)]
struct CullPc {
    vp: [f32; 16],
    count: u32,
    cam: u32,
    _pad: [u32; 2],
}

pub(crate) struct Cull {
    layout: vk::PipelineLayout,
    pipe: vk::Pipeline,
    visible: [Buf; FRAMES], // survivors per camera and group slice (device-local, GPU only)
    sets: [vk::DescriptorSet; FRAMES],
}

impl Cull {
    /// 3 storage buffers (input, visible, draws) per frame + view/count as push constants.
    pub unsafe fn new(c: &Ctx, frames: &[Frame; FRAMES]) -> Self {
        let device = c.device;
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
        let push = [vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::COMPUTE)
            .size(size_of::<CullPc>() as u32)];
        let set_layouts = [set_layout];
        let layout: vk::PipelineLayout = mk(CREATE_PIPELINE_LAYOUT_FAILED, |p| {
            vkCreatePipelineLayout(
                device,
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(&set_layouts)
                    .push_constant_ranges(&push),
                NA,
                p,
            )
        });
        let module = shader_module(device, CULL);
        let mut pipe = vk::Pipeline::default();
        ck(
            vkCreateComputePipelines(
                device,
                vk::PipelineCache::null(),
                1,
                &vk::ComputePipelineCreateInfo::default()
                    .stage(
                        vk::PipelineShaderStageCreateInfo::default()
                            .stage(vk::ShaderStageFlags::COMPUTE)
                            .module(module)
                            .name(c"cull"),
                    )
                    .layout(layout),
                NA,
                &mut pipe,
            ),
            CREATE_COMPUTE_PIPELINES_FAILED,
        );
        vkDestroyShaderModule(device, module, NA);

        let pool_sizes = [vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(3 * FRAMES as u32)];
        let dpool: vk::DescriptorPool = mk(CREATE_DESCRIPTOR_POOL_FAILED, |p| {
            vkCreateDescriptorPool(
                device,
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(FRAMES as u32)
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

        let inst_bytes = (GROUPS * CAP * size_of::<Inst>()) as u64;
        let visible: [Buf; FRAMES] = core::array::from_fn(|_| {
            make_buffer(
                c,
                inst_bytes * MAX_CAMERAS as u64,
                vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::VERTEX_BUFFER,
                false,
            )
        });
        for f in 0..FRAMES {
            let infos = [frames[f].input.buf, visible[f].buf, frames[f].draws.buf].map(|b| {
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
        }
        Self {
            layout,
            pipe,
            visible,
            sets,
        }
    }

    /// Culling reads one packed list of instances.
    pub fn dst(&self, n: usize, _group: usize, _in_group: usize) -> usize {
        n
    }

    /// The culling shader counts the survivors into instanceCount.
    pub fn instance_count(&self, _in_group: usize) -> u32 {
        0
    }

    /// Instance buffer and byte offset to draw camera-group `slice` from.
    pub fn instances(
        &self,
        f: usize,
        _fr: &Frame,
        slice: usize,
        _group: usize,
    ) -> (vk::Buffer, u64) {
        (
            self.visible[f].buf,
            (slice * CAP * size_of::<Inst>()) as u64,
        )
    }

    /// One dispatch per camera over the `n` instances, then a barrier before the draws read the result.
    pub unsafe fn dispatch(&self, cmd: vk::CommandBuffer, f: usize, views: &[View], n: usize) {
        if n == 0 {
            return;
        }
        vkCmdBindPipeline(cmd, vk::PipelineBindPoint::COMPUTE, self.pipe);
        vkCmdBindDescriptorSets(
            cmd,
            vk::PipelineBindPoint::COMPUTE,
            self.layout,
            0,
            1,
            &self.sets[f],
            0,
            ptr::null(),
        );
        for (k, v) in views.iter().enumerate() {
            let pc = CullPc {
                vp: v.vp,
                count: n as u32,
                cam: k as u32,
                _pad: [0; 2],
            };
            vkCmdPushConstants(
                cmd,
                self.layout,
                vk::ShaderStageFlags::COMPUTE,
                0,
                size_of::<CullPc>() as u32,
                ptr::from_ref(&pc).cast(),
            );
            vkCmdDispatch(cmd, (n as u32).div_ceil(64), 1, 1);
        }
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
}
