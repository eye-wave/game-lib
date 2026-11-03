// Tile layer (`Kind::Tiles`): every cell is one 48 px cell of a sheet that is 16 cells wide. The layer is an R8
// texture (one texel per cell: 0 = empty, else row * 16 + column + 1, row 0 = top of the map). Empty cells and
// see-through texels are discarded, so the layers below show; the alpha is the opacity (`tint.z`, for a blending
// material). See autotile.wgsl.
struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(4) a: vec4<f32>,    // x, y, z, angle
    @location(5) s: vec4<f32>,    // scale x, y
    @location(6) tint: vec4<f32>, // (structure, has structures, opacity): see `owns`
}

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) world: vec2<f32>, // world units = cells, y up, the map's bottom left corner at 0
    @location(1) @interpolate(flat) tint: vec3<f32>,
}

@group(0) @binding(0) var sheet: texture_2d<f32>;
@group(0) @binding(1) var layer: texture_2d<f32>;
@group(0) @binding(2) var owner: texture_2d<f32>; // per cell: 0, or the structure index + 1
var<immediate> vp: mat4x4<f32>;

// The whole-map quad (tint.x = 0) draws the cells no structure owns, if the layer has structures (tint.y); a
// structure's quad (tint.x = its index + 1, over its rectangle only) draws the ones it owns.
fn owns(cell: vec2<i32>, tint: vec3<f32>) -> bool {
    let o = textureLoad(owner, clamp(cell, vec2(0), vec2<i32>(textureDimensions(owner)) - 1), 0).r * 255.0;
    return tint.y < 0.5 || abs(o - tint.x) < 0.5;
}

@vertex
fn vs_main(v: VsIn) -> VsOut {
    let c = cos(v.a.w);
    let s = sin(v.a.w);
    let p = v.pos.xy * v.s.xy;
    var out: VsOut;
    out.pos = vp * vec4<f32>(p.x * c - p.y * s + v.a.x, p.x * s + p.y * c + v.a.y, v.pos.z + v.a.z, 1.0);
    out.world = vec2<f32>(p.x * c - p.y * s + v.a.x, p.x * s + p.y * c + v.a.y);
    out.tint = v.tint.rgb;
    return out;
}

const TILE = 48;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(textureDimensions(layer));
    let w = vec2(in.world.x, f32(dims.y) - in.world.y);
    let cell = vec2<i32>(floor(w));
    let p = vec2<i32>(floor(fract(w) * f32(TILE)));
    let v = i32(textureLoad(layer, clamp(cell, vec2(0), dims - 1), 0).r * 255.0 + 0.5);
    if v == 0 || !owns(cell, in.tint) {
        discard;
    }
    let cols = i32(textureDimensions(sheet).x) / TILE;
    let t = textureLoad(sheet, vec2((v - 1) % cols, (v - 1) / cols) * TILE + p, 0);
    if t.a < 0.5 {
        discard;
    }
    return vec4<f32>(t.rgb, in.tint.z);
}
