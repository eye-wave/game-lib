// Tile layer (`Kind::Tiles`): every cell is one 48 px cell of a sheet that is 16 cells wide. The layer is an R8
// texture (one texel per cell: 0 = empty, else row * 16 + column + 1, row 0 = top of the map). Empty cells and
// see-through texels are discarded, so the layers below show; there is no blending. See autotile.wgsl.
struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(4) a: vec4<f32>,    // x, y, z, angle
    @location(5) s: vec4<f32>,    // scale x, y
    @location(6) tint: vec4<f32>, // unused
}

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>, // 0..1 across the map, v up
}

@group(0) @binding(0) var sheet: texture_2d<f32>;
@group(0) @binding(1) var layer: texture_2d<f32>;
var<immediate> vp: mat4x4<f32>;

@vertex
fn vs_main(v: VsIn) -> VsOut {
    let c = cos(v.a.w);
    let s = sin(v.a.w);
    let p = v.pos.xy * v.s.xy;
    var out: VsOut;
    out.pos = vp * vec4<f32>(p.x * c - p.y * s + v.a.x, p.x * s + p.y * c + v.a.y, v.pos.z + v.a.z, 1.0);
    out.uv = v.pos.xy + 0.5;
    return out;
}

const TILE = 48;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(textureDimensions(layer));
    let w = vec2(in.uv.x, 1.0 - in.uv.y) * vec2<f32>(dims);
    let cell = vec2<i32>(floor(w));
    let p = vec2<i32>(floor(fract(w) * f32(TILE)));
    let v = i32(textureLoad(layer, clamp(cell, vec2(0), dims - 1), 0).r * 255.0 + 0.5);
    if v == 0 {
        discard;
    }
    let cols = i32(textureDimensions(sheet).x) / TILE;
    let t = textureLoad(sheet, vec2((v - 1) % cols, (v - 1) / cols) * TILE + p, 0);
    if t.a < 0.5 {
        discard;
    }
    return vec4<f32>(t.rgb, 1.0);
}
