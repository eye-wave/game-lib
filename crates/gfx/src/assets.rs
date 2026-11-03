//! `Handle`, `Assets`, `Mesh` and `Material`: the asset side of the game-facing API.
//! `Material::flat` lives in `cam2d.rs` and `Material::lit` in `lit.rs`.
use alloc::vec::Vec;
use ash::vk;
use core::{marker::PhantomData, ptr, slice};
use shader::Align4;

/// Typed index into `Assets<T>`; it is the component you put on entities.
pub struct Handle<T>(pub(crate) u32, PhantomData<fn() -> T>);
impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Handle<T> {}

/// Resource holding every `T`; `add` returns the handle.
pub struct Assets<T>(pub(crate) Vec<T>);
impl<T> Default for Assets<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}
impl<T> Assets<T> {
    pub fn add(&mut self, v: T) -> Handle<T> {
        self.0.push(v);
        Handle(self.0.len() as u32 - 1, PhantomData)
    }
}

/// Vertex data as raw bytes: any vertex type, as long as the `Material` describes its layout.
pub struct Mesh {
    pub(crate) data: Vec<u8>,
    pub(crate) count: u32,
    pub(crate) radius2: f32, // squared bounding radius around the origin, for culling
}
impl Mesh {
    /// `V` should be `#[repr(C)]` plain data whose first three `f32`s are the vertex position (used to bound the mesh).
    pub fn new<V>(verts: &[V]) -> Self {
        let bytes =
            unsafe { slice::from_raw_parts(verts.as_ptr().cast::<u8>(), size_of_val(verts)) };
        let radius2 = verts
            .iter()
            .map(|v| unsafe {
                let p = ptr::from_ref(v).cast::<f32>();
                *p * *p + *p.add(1) * *p.add(1) + *p.add(2) * *p.add(2)
            })
            .fold(0.0, |a, b| if b > a { b } else { a }); // not f32::max: that links libm
        Self {
            data: bytes.to_vec(),
            count: verts.len() as u32,
            radius2,
        }
    }
}

/// A shader plus the vertex layout it reads. One Vulkan pipeline is built per material, on first use.
/// Vertex shader contract (see `src/lit.wgsl`): mesh attributes at locations 0..`attrs.len()` (max 4),
/// `@location(4) vec4<f32>` = x, y, z, yaw, `@location(5) vec4<f32>` = scale xyz, `@location(6) vec4<f32>` = tint rgb
/// (from `Transform` and `Color`), and `var<immediate> mat4x4<f32>` = view * projection.
pub struct Material {
    pub shader: &'static Align4<[u8]>, // entry points: vs_main, fs_main
    pub stride: u32,
    pub attrs: &'static [vk::Format], // tightly packed, location = index
    /// Images the fragment shader reads (feature `textures`, at most 4): texture `k` is
    /// `@group(0) @binding(k) var t: texture_2d<f32>`, and `@group(0) @binding(4)` is a linear clamp sampler.
    pub textures: Vec<Handle<Texture>>,
}
impl Material {
    pub const fn new(
        shader: &'static Align4<[u8]>,
        stride: u32,
        attrs: &'static [vk::Format],
    ) -> Self {
        Self {
            shader,
            stride,
            attrs,
            textures: Vec::new(),
        }
    }
    pub fn with_textures(mut self, textures: &[Handle<Texture>]) -> Self {
        self.textures = textures.to_vec();
        self
    }
}

/// Pixel format of a `Texture`. Both are UNORM, not SRGB: values reach the shader as stored.
#[derive(Clone, Copy, PartialEq)]
pub enum TexFormat {
    R8,    // one byte per pixel (a luma / data texture)
    Rgba8, // straight alpha
}

/// An image for `Material::textures`: `px` is tightly packed, top row first. Uploaded once, the first frame a
/// material that lists it exists; later changes are ignored.
pub struct Texture {
    pub w: u32,
    pub h: u32,
    pub format: TexFormat,
    pub px: Vec<u8>,
}
