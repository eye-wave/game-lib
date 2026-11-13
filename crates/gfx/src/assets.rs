//! `Handle`, `Assets`, `Mesh` and `Material`: the asset side of the game-facing API.
//! `Material::flat` lives in `cam2d.rs` and `Material::lit` in `lit.rs`.
use alloc::vec::Vec;
use ash::vk;
use core::{marker::PhantomData, slice};
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
    pub fn get_mut(&mut self, h: Handle<T>) -> &mut T {
        &mut self.0[h.0 as usize]
    }
}

/// Vertex data as raw bytes: any vertex type, as long as the `Material` describes its layout.
pub struct Mesh {
    pub(crate) data: Vec<u8>,
    pub(crate) count: u32,
    #[cfg(feature = "cull")]
    pub(crate) radius2: f32, // squared bounding radius around the origin, for culling
    /// Feature `ranges`. Multi-draw: when not empty, only these `[first vertex, vertex count]` ranges are drawn, all
    /// in one call, instead of every vertex. Edit it each frame (`Assets::get_mut`) to draw a changing subset, e.g.
    /// the visible parts of a big mesh. Every range is drawn once with the first instance of the entity's group: put
    /// one entity at the origin and have the shader read absolute positions. `first` is what the shader sees in
    /// `vertex_index`.
    #[cfg(feature = "ranges")]
    pub ranges: Vec<[u32; 2]>,
}

fn bytes<V>(verts: &[V]) -> Vec<u8> {
    unsafe { slice::from_raw_parts(verts.as_ptr().cast::<u8>(), size_of_val(verts)) }.to_vec()
}
impl Mesh {
    /// `V` should be `#[repr(C)]` plain data whose first three `f32`s are the vertex position (used to bound the mesh
    /// for `cull`).
    pub fn new<V>(verts: &[V]) -> Self {
        #[cfg(feature = "cull")]
        let radius2 = verts
            .iter()
            .map(|v| unsafe {
                let p = core::ptr::from_ref(v).cast::<f32>();
                *p * *p + *p.add(1) * *p.add(1) + *p.add(2) * *p.add(2)
            })
            .fold(0.0, |a, b| if b > a { b } else { a }); // not f32::max: that links libm
        #[cfg(not(feature = "cull"))]
        let radius2 = 0.0;
        Self::with_radius2(verts, radius2)
    }

    /// Any plain vertex layout (the first three `f32`s need not be a position), with the bounding radius around the
    /// origin, squared, given instead of measured. Without `cull` it is ignored.
    pub fn with_radius2<V>(verts: &[V], _radius2: f32) -> Self {
        Self {
            data: bytes(verts),
            count: verts.len() as u32,
            #[cfg(feature = "cull")]
            radius2: _radius2,
            #[cfg(feature = "ranges")]
            ranges: Vec::new(),
        }
    }

    /// Squared bounding radius around the origin; 0 without `cull`, which is the only reader.
    #[cfg(feature = "cull")]
    pub(crate) fn radius2(&self) -> f32 {
        self.radius2
    }
    #[cfg(not(feature = "cull"))]
    pub(crate) fn radius2(&self) -> f32 {
        0.0
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
    /// See-through: the fragment's alpha blends it over what is behind (straight alpha). Blending materials draw
    /// after every opaque one and still write depth, so put only what is see-through right now on them (swap the
    /// entity's material back when it is opaque again).
    pub blend: bool,
    /// Draw the vertices as one triangle strip per draw instead of a triangle list.
    pub strip: bool,
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
            blend: false,
            strip: false,
        }
    }
    pub const fn strips(mut self) -> Self {
        self.strip = true;
        self
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
