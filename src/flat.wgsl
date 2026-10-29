// Material contract (see gfx::Material): mesh attributes at locations 0.., per-instance data at 4 and 5,
// camera in the immediate block.
struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) color: vec3<f32>,
    @location(4) inst: vec4<f32>, // x, y, scale, rotation
    @location(5) tint: vec4<f32>,
}

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec3<f32>,
}

var<immediate> view: vec4<f32>; // camera x, y, half width, half height

// WGSL clip space is y-up; naga flips Y when targeting Vulkan.
@vertex
fn vs_main(v: VsIn) -> VsOut {
    let c = cos(v.inst.w);
    let s = sin(v.inst.w);
    let p = vec2<f32>(v.pos.x * c - v.pos.y * s, v.pos.x * s + v.pos.y * c) * v.inst.z + v.inst.xy;
    var out: VsOut;
    out.pos = vec4<f32>((p - view.xy) / view.zw, 0.0, 1.0);
    out.color = v.color * v.tint.rgb;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
