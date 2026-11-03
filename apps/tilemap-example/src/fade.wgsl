// Scene fade for map changes: an iris that closes on the screen center with a soft edge, down to black.
// Same contract as crates/gfx/src/post.wgsl; params = (width, height, seconds, fade) with fade = PostProcess.1,
// 0 = the scene as it is, 1 = black.
@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
var<immediate> params: vec4<f32>;

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
    let c = textureSample(scene, samp, in.uv);
    let a = params.x / params.y;
    let d = length((in.uv - 0.5) * vec2<f32>(a, 1.0)); // distance from the center, in screen heights
    let far = 0.5 * length(vec2<f32>(a, 1.0));          // ... to a corner
    let soft = 0.3;                                      // width of the soft edge
    let r = (1.0 - params.w) * (far + soft);             // the open radius: past every corner at 0, nothing at 1
    let open = 1.0 - smoothstep(r - soft, r, d);
    return vec4<f32>(c.rgb * open, 1.0);
}
