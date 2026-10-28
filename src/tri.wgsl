struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) color: vec3<f32>,
}

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec3<f32>,
}

// Per-draw translation, set from the entity's Pos (Vulkan push constant).
var<immediate> offset: vec2<f32>;

// WGSL clip space is y-up; naga flips Y when targeting Vulkan.
@vertex
fn vs_main(v: VsIn) -> VsOut {
    var out: VsOut;
    out.pos = vec4<f32>(v.pos + offset, 0.0, 1.0);
    out.color = v.color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
