// Material contract (see gfx::Material): mesh attributes at locations 0.., per-instance data at 4, 5 and 6,
// view-projection matrix in the immediate block.
struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(4) a: vec4<f32>,    // x, y, z, yaw
    @location(5) s: vec4<f32>,    // scale x, y, z
    @location(6) tint: vec4<f32>, // rgb
}

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) dist: f32,
    @location(3) wpos: vec3<f32>,
}

var<immediate> vp: mat4x4<f32>;

// Rotation about the y axis.
fn yaw(v: vec3<f32>, c: f32, s: f32) -> vec3<f32> {
    return vec3<f32>(v.x * c + v.z * s, v.y, -v.x * s + v.z * c);
}

@vertex
fn vs_main(v: VsIn) -> VsOut {
    let c = cos(v.a.w);
    let s = sin(v.a.w);
    var out: VsOut;
    let world = yaw(v.pos * v.s.xyz, c, s) + v.a.xyz;
    out.wpos = world;
    out.pos = vp * vec4<f32>(world, 1.0);
    out.normal = yaw(v.normal, c, s);
    out.color = v.color * v.tint.rgb;
    out.dist = out.pos.w; // view-space depth
    return out;
}

const SKY = vec3<f32>(0.53, 0.68, 0.85); // same as Camera::default().sky

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let light = normalize(vec3<f32>(0.4, 1.0, 0.3));
    // placeholder texture: 0.5 unit checker with thin grid lines, projected along the dominant axis of the normal
    let n = abs(normalize(in.normal));
    var uv = in.wpos.xy;
    if (n.y > n.x && n.y > n.z) {
        uv = in.wpos.xz;
    } else if (n.x > n.z) {
        uv = in.wpos.zy;
    }
    let f = fract(uv * 2.0);
    let line = step(min(min(f.x, f.y), min(1.0 - f.x, 1.0 - f.y)), 0.05);
    let checker = 0.8 + 0.2 * f32((i32(floor(uv.x * 2.0)) + i32(floor(uv.y * 2.0))) & 1);
    let base = in.color * checker * (1.0 - 0.4 * line);
    let lit = base * (0.35 + 0.65 * max(dot(normalize(in.normal), light), 0.0));
    let fog = clamp((in.dist - 25.0) / 60.0, 0.0, 1.0);
    return vec4<f32>(mix(lit, SKY, fog), 1.0);
}
