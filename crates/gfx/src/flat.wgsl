// Built-in material for `Material::flat()`: 2D shapes made of `Vec3` positions, filled with the instance's `Color`
// (white when absent) and rotated about z by `Transform.yaw`. Same contract as the material shaders in apps/.
struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(4) a: vec4<f32>,    // x, y, z, angle
    @location(5) s: vec4<f32>,    // scale x, y
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
    let p = v.pos.xy * v.s.xy;
    var out: VsOut;
    out.pos = vp * vec4<f32>(p.x * c - p.y * s + v.a.x, p.x * s + p.y * c + v.a.y, v.pos.z + v.a.z, 1.0);
    out.color = v.tint.rgb;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
