// GPU frustum culling. One thread per instance: instances inside the camera view are appended to their
// group's slice of `visible`, and that group's indirect draw command counts them.
// Must match CAP / Inst in lib.rs.
const CAP = 4096u;

struct Inst {
    a: vec4<f32>,    // x, y, scale, rotation
    color: vec4<f32>,
    info: vec4<f32>, // group, mesh bounding radius squared
}

struct Draw {
    vertex_count: u32,
    instance_count: atomic<u32>,
    first_vertex: u32,
    first_instance: u32,
}

struct Pc {
    view: vec4<f32>, // camera x, y, half width, half height
    count: u32,
}

@group(0) @binding(0) var<storage, read> input: array<Inst>;
@group(0) @binding(1) var<storage, read_write> visible: array<Inst>;
@group(0) @binding(2) var<storage, read_write> draws: array<Draw>;
var<immediate> pc: Pc;

@compute @workgroup_size(64)
fn cull(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= pc.count {
        return;
    }
    let i = input[id.x];
    let r = sqrt(i.info.y) * i.a.z; // bounding circle radius in world units
    let d = abs(i.a.xy - pc.view.xy);
    if d.x > pc.view.z + r || d.y > pc.view.w + r {
        return;
    }
    let g = u32(i.info.x);
    let n = atomicAdd(&draws[g].instance_count, 1u);
    visible[g * CAP + n] = i;
}
