// Loading screen: all black but a progress bar. Same contract as crates/gfx/src/post.wgsl, with
// params = (width, height, seconds, progress 0..1) where progress is PostProcess.1.
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
    let px = (in.uv - vec2<f32>(0.5, 0.5)) * params.xy; // pixels from the screen center
    let half = vec2<f32>(min(params.x * 0.3, 320.0), 4.0); // bar half-size
    let d = abs(px) - half;
    let inside = step(max(d.x, d.y), 0.0);
    let filled = step(px.x, (clamp(params.w, 0.0, 1.0) * 2.0 - 1.0) * half.x);
    return vec4<f32>(vec3<f32>(inside * mix(0.12, 1.0, filled)), 1.0);
}
