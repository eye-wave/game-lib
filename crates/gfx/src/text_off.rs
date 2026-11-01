//! `text` is off: no atlas, no descriptor set. Same interface as `text.rs`.
use crate::gpu::Ctx;
use ash::vk;
use ecs::App;
use shader::Align4;

pub(crate) struct Text;

impl Text {
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
        _: &'static Align4<[u8]>,
    ) {
    }
}
