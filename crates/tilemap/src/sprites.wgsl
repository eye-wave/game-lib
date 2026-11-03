// Sprites (`Kind::Sprites`): one instance per object, drawn as its own quad so it can sort by depth with the hero
// and the other sprites (see `tilemap::depth`). The Transform scale is the size in cells, and the entity's Color
// is (column, row, opacity) of the sprite's top-left cell in the sheet, so every sprite of a sheet is one instanced
// draw. See-through texels are discarded; the alpha is the opacity (for a blending material, see
// `tilemap::see_through`).
struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(4) a: vec4<f32>,    // x, y, z, angle
    @location(5) s: vec4<f32>,    // scale x, y: the size in cells
    @location(6) tint: vec4<f32>, // rgb = (column, row, opacity)
}

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>, // 0..1 across the sprite, v up
    @location(1) @interpolate(flat) src: vec2<f32>,
    @location(2) @interpolate(flat) size: vec2<f32>,
    @location(3) @interpolate(flat) fade: f32,
}

@group(0) @binding(0) var sheet: texture_2d<f32>;
var<immediate> vp: mat4x4<f32>;

@vertex
fn vs_main(v: VsIn) -> VsOut {
    let c = cos(v.a.w);
    let s = sin(v.a.w);
    let p = v.pos.xy * v.s.xy;
    var out: VsOut;
    out.pos = vp * vec4<f32>(p.x * c - p.y * s + v.a.x, p.x * s + p.y * c + v.a.y, v.pos.z + v.a.z, 1.0);
    out.uv = v.pos.xy + 0.5;
    out.src = v.tint.xy;
    out.size = v.s.xy;
    out.fade = v.tint.z;
    return out;
}

const TILE = 48;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let px = vec2(in.uv.x, 1.0 - in.uv.y) * in.size * f32(TILE);
    let q = min(vec2<i32>(floor(px)), vec2<i32>(in.size * f32(TILE)) - 1);
    let t = textureLoad(sheet, vec2<i32>(in.src) * TILE + q, 0);
    if t.a < 0.5 {
        discard;
    }
    return vec4<f32>(t.rgb, in.fade);
}
