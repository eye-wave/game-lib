// Demo post-process: chromatic aberration + vignette + a little animated film grain.
// Same contract as crates/gfx/src/post.wgsl.
@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
var<immediate> params: vec4<f32>; // width, height, seconds

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(p.x, 1.0 - p.y);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let d = in.uv - vec2<f32>(0.5, 0.5);
    let r2 = dot(d, d);
    let off = d * (0.05 * r2); // red/blue drift apart towards the edges
    var col = vec3<f32>(
        textureSample(scene, samp, in.uv + off).r,
        textureSample(scene, samp, in.uv).g,
        textureSample(scene, samp, in.uv - off).b,
    );
    col *= 1.0 - 0.9 * smoothstep(0.08, 0.5, r2); // vignette
    let n = fract(sin(dot(in.uv * params.xy + vec2<f32>(params.z, 0.0), vec2<f32>(12.9898, 78.233))) * 43758.5453);
    col += (n - 0.5) * 0.03; // grain
    return vec4<f32>(col, 1.0);
}
