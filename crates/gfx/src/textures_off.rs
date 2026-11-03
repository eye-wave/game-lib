//! `textures` is off: no descriptor sets. Same interface as `textures.rs`.
use crate::gpu::Ctx;
use ash::vk;
use ecs::App;
use shader::Align4;

pub(crate) struct Textures;

impl Textures {
    pub unsafe fn new(_: &Ctx) -> Self {
        Self
    }

    pub fn layouts(&self) -> &[vk::DescriptorSetLayout] {
        &[]
    }

    pub unsafe fn record(&mut self, _: &Ctx, _: vk::CommandBuffer, _: &App) {}

    pub unsafe fn bind(
        &self,
        _: vk::CommandBuffer,
        _: vk::PipelineLayout,
        _: usize,
        _: &'static Align4<[u8]>,
    ) {
    }
}
