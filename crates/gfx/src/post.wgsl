// Default post-process: passthrough. A custom one (PostProcess resource) follows the same contract:
//  - group 0: binding 0 = the rendered scene, binding 1 = a linear clamp sampler
//  - immediate block: vec4 = (width, height, seconds, PostProcess.1)
//  - entry points vs_main (draws 3 vertices, no vertex buffers) and fs_main
@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
var<immediate> params: vec4<f32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// One oversized triangle covering the screen.
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
    return textureSample(scene, samp, in.uv);
}
