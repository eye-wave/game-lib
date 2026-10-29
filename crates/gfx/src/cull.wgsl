// GPU frustum culling. One thread per instance: instances whose bounding sphere touches the camera frustum are
// appended to their group's slice of `visible`, and that group's indirect draw command counts them.
// Must match CAP / Inst in lib.rs.
const CAP = 4096u;

struct Inst {
    a: vec4<f32>,     // x, y, z, yaw
    s: vec4<f32>,     // scale x, y, z, group
    color: vec4<f32>, // tint rgb, mesh bounding radius squared
}

struct Draw {
    vertex_count: u32,
    instance_count: atomic<u32>,
    first_vertex: u32,
    first_instance: u32,
}

struct Pc {
    vp: mat4x4<f32>, // view * projection (column-major)
    count: u32,
}

@group(0) @binding(0) var<storage, read> input: array<Inst>;
@group(0) @binding(1) var<storage, read_write> visible: array<Inst>;
@group(0) @binding(2) var<storage, read_write> draws: array<Draw>;
var<immediate> pc: Pc;

// Sphere vs the 6 frustum planes, taken from the rows of the view-projection matrix (depth range 0..1).
fn outside(c: vec3<f32>, r: f32) -> bool {
    let m = pc.vp;
    let r0 = vec4<f32>(m[0][0], m[1][0], m[2][0], m[3][0]);
    let r1 = vec4<f32>(m[0][1], m[1][1], m[2][1], m[3][1]);
    let r2 = vec4<f32>(m[0][2], m[1][2], m[2][2], m[3][2]);
    let r3 = vec4<f32>(m[0][3], m[1][3], m[2][3], m[3][3]);
    var planes = array<vec4<f32>, 6>(r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2);
    for (var i = 0; i < 6; i++) {
        let p = planes[i];
        if dot(p.xyz, c) + p.w < -r * length(p.xyz) {
            return true;
        }
    }
    return false;
}

@compute @workgroup_size(64)
fn cull(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= pc.count {
        return;
    }
    let i = input[id.x];
    let r = sqrt(i.color.w) * max(max(i.s.x, i.s.y), i.s.z); // bounding sphere radius in world units
    if outside(i.a.xyz, r) {
        return;
    }
    let g = u32(i.s.w);
    let n = atomicAdd(&draws[g].instance_count, 1u);
    visible[g * CAP + n] = i;
}
