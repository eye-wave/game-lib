// Terrain chunks drawn as triangle strips (layout in `Terrain::mesh`, main.rs). A vertex stores only its height and its
// normal as yaw / pitch (snorm16): x and z are rebuilt from the vertex index (a chunk is one draw with its own first
// vertex, so the index also says which chunk), and the color comes from the height.
// The consts must match main.rs. The mesh sits at world coordinates, so the instance's position / scale are unused.
const C = 32u;   // cells per chunk side
const CPS = 15u; // chunks per side
const CELL = 0.3;
const HALF = 72.0; // half the terrain width
const HMAX = 12.0;
const ROW = 2u * C + 4u;  // strip vertices per grid row: 2 * (C + 1), plus 2 joiners (degenerate triangles)
const PER = C * ROW - 2u; // vertices per chunk: the last row has no joiners
const PI = 3.14159265;

struct VsIn {
    @builtin(vertex_index) vi: u32,
    @location(0) h: f32,
    @location(1) n: vec2<f32>,    // yaw / pi, pitch / (pi / 2)
    @location(6) tint: vec4<f32>, // rgb
}

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) dist: f32,
}

var<immediate> vp: mat4x4<f32>;

@vertex
fn vs_main(v: VsIn) -> VsOut {
    let chunk = v.vi / PER;
    let k = v.vi % PER;
    let row = k / ROW;
    let t = k % ROW;
    // t < 2C+2: strip vertex t is (t/2, row + t%2); then two joiners: (C, row+1), (0, row+1)
    var i = C;
    var j = row + 1u;
    if t < 2u * C + 2u {
        i = t >> 1u;
        j = row + (t & 1u);
    } else if t == 2u * C + 3u {
        i = 0u;
    }
    let x = f32((chunk % CPS) * C + i) * CELL - HALF;
    let z = f32((chunk / CPS) * C + j) * CELL - HALF;
    let yaw = v.n.x * PI;
    let pitch = v.n.y * (0.5 * PI);
    let q = h_color(v.h);
    var out: VsOut;
    out.pos = vp * vec4<f32>(x, v.h, z, 1.0);
    out.normal = vec3<f32>(cos(pitch) * sin(yaw), sin(pitch), cos(pitch) * cos(yaw));
    out.color = q * v.tint.rgb;
    out.dist = out.pos.w; // view-space depth
    return out;
}

fn h_color(h: f32) -> vec3<f32> {
    let k = h / HMAX;
    return vec3<f32>(0.25 + 0.5 * k, 0.6 - 0.2 * k, 0.2 + 0.2 * k);
}

const SKY = vec3<f32>(0.53, 0.68, 0.85); // same as Camera::default().sky

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let light = normalize(vec3<f32>(0.4, 1.0, 0.3));
    let lit = in.color * (0.35 + 0.65 * max(dot(normalize(in.normal), light), 0.0));
    let fog = clamp((in.dist - 25.0) / 60.0, 0.0, 1.0);
    return vec4<f32>(mix(lit, SKY, fog), 1.0);
}
