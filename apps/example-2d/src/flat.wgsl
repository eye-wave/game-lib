// Same material contract as lit.wgsl, minus lighting and fog: mesh attributes at locations 0 and 1,
// per-instance data at 4, 5 and 6, view-projection matrix in the immediate block.
// Transform.yaw is reused as a rotation about z.
struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(4) a: vec4<f32>,    // x, y, z, angle
    @location(5) s: vec4<f32>,    // scale x, y, z
    @location(6) tint: vec4<f32>, // rgb
}

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec3<f32>,
}

var<immediate> vp: mat4x4<f32>;

@vertex
fn vs_main(v: VsIn) -> VsOut {
    let c = cos(v.a.w);
    let s = sin(v.a.w);
    let p = v.pos * v.s.xyz;
    var out: VsOut;
    out.pos = vp * vec4<f32>(p.x * c - p.y * s + v.a.x, p.x * s + p.y * c + v.a.y, p.z + v.a.z, 1.0);
    out.color = v.color * v.tint.rgb;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
