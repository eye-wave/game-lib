// Built-in material for `Material::msdf()`: text meshes of `x, y, z, u, v` vertices (the `font` crate's layout),
// filled with the instance's `Color` (white when absent). Same instance contract as flat.wgsl; the atlas is
// group 0 (binding 0 = the MSDF, binding 1 = a linear clamp sampler).
struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(4) a: vec4<f32>,    // x, y, z, angle
    @location(5) s: vec4<f32>,    // scale x, y
    @location(6) tint: vec4<f32>, // rgb
}

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec3<f32>,
}

@group(0) @binding(0) var atlas: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
var<immediate> vp: mat4x4<f32>;

@vertex
fn vs_main(v: VsIn) -> VsOut {
    let c = cos(v.a.w);
    let s = sin(v.a.w);
    let p = v.pos.xy * v.s.xy;
    var out: VsOut;
    out.pos = vp * vec4<f32>(p.x * c - p.y * s + v.a.x, p.x * s + p.y * c + v.a.y, v.pos.z + v.a.z, 1.0);
    out.uv = v.uv;
    out.color = v.tint.rgb;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let m = textureSample(atlas, samp, in.uv).rgb;
    // the median of the three channels is the signed distance; 0.5 is the outline
    let d = max(min(m.r, m.g), min(max(m.r, m.g), m.b));
    // alpha ramps over one screen pixel around the outline; `aa` pipelines turn it into sample coverage (no
    // blending anywhere, so without `aa` the edge pixels just draw solid, half a pixel fatter)
    let a = clamp((d - 0.5) / max(fwidth(d), 1e-4) + 0.5, 0.0, 1.0);
    if a == 0.0 {
        discard;
    }
    return vec4<f32>(in.color, a);
}
