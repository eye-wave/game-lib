// Autotile layer (`Kind::Autotile`, RPG Maker A2 floor autotiles): one quad covers the whole level and this
// fragment shader draws it. The layer is an R8 texture (one texel per cell: 0 = empty, else the A2 kind 0..31 + 1,
// row 0 = top of the map); the sheet is the RGBA A2 sheet (8 x 4 kinds, each 2 x 3 tiles of 48 px = 4 x 6
// subtiles of 24 px). Per fragment: world position -> cell -> the 8 neighbours from the layer -> autotile shape
// 0..46 -> FLOOR_AUTOTILE_TABLE gives the 4 subtiles -> textureLoad (no sampler: pixel art stays exact at any
// zoom). Empty cells and see-through texels are discarded, so the layers below show; there is no blending.
// The quad's vertices are -0.5..0.5, so uv 0..1 spans the map and the Transform scale decides the tile size.
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

@group(0) @binding(0) var tileset: texture_2d<f32>;
@group(0) @binding(1) var level: texture_2d<f32>;
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
const SUB = 24;

// Tilemap.FLOOR_AUTOTILE_TABLE of RPG Maker MV: per shape, the (x, y) subtile of the kind's 4 x 6 grid for
// the top-left, top-right, bottom-left and bottom-right quarter of the tile.
// (x + 4 * y) of each quarter's subtile, 5 bits per quarter, TL in the low bits.
var<private> TABLE: array<u32, 48> = array<u32, 48>(
    0x6BA32u, 0x6BA22u, 0x6B872u, 0x6B862u, 0x3BA32u, 0x3BA22u,
    0x3B872u, 0x3B862u, 0x69A32u, 0x69A22u, 0x69872u, 0x69862u,
    0x39A32u, 0x39A22u, 0x39872u, 0x39862u, 0x6B230u, 0x6B070u,
    0x3B230u, 0x3B070u, 0x6B92Au, 0x3B92Au, 0x6992Au, 0x3992Au,
    0x7BA72u, 0x79A72u, 0x7BA62u, 0x79A62u, 0xADA32u, 0xADA22u,
    0xAD872u, 0xAD862u, 0x7B270u, 0xAD92Au, 0x6B128u, 0x3B128u,
    0x7B96Au, 0x7996Au, 0xBDA72u, 0xBDA62u, 0xAD230u, 0xAD070u,
    0x7B168u, 0xAD128u, 0xBD270u, 0xBD96Au, 0xBD168u, 0x29020u,
);

fn kind_at(c: vec2<i32>) -> u32 {
    // outside the map counts as the edge cell, so autotiles run off the map instead of closing at its border
    let d = vec2<i32>(textureDimensions(level));
    return u32(textureLoad(level, clamp(c, vec2(0), d - 1), 0).r * 255.0 + 0.5);
}

// The 0..46 autotile shape of cell `c`: which of its 4 sides and 4 corners border another kind.
// Bits of `e` / `d`: 1 = left, 2 = top, 4 = right, 8 = bottom (edges); 1 = top-left, 2 = top-right,
// 4 = bottom-right, 8 = bottom-left (corners). A corner only matters while both sides touching it are same-kind.
fn shape(c: vec2<i32>, k: u32) -> u32 {
    var e = 0u;
    if kind_at(c + vec2(-1, 0)) != k { e |= 1u; }
    if kind_at(c + vec2(0, -1)) != k { e |= 2u; }
    if kind_at(c + vec2(1, 0)) != k { e |= 4u; }
    if kind_at(c + vec2(0, 1)) != k { e |= 8u; }
    var d = 0u;
    if (e & 3u) == 0u && kind_at(c + vec2(-1, -1)) != k { d |= 1u; }
    if (e & 6u) == 0u && kind_at(c + vec2(1, -1)) != k { d |= 2u; }
    if (e & 12u) == 0u && kind_at(c + vec2(1, 1)) != k { d |= 4u; }
    if (e & 9u) == 0u && kind_at(c + vec2(-1, 1)) != k { d |= 8u; }
    // base shape per edge mask, then the corner bits that are free for that mask (in the table's order)
    switch e {
        case 0u: { return d; }
        case 1u: { return 16u + ((d >> 1u) & 1u) + ((d >> 1u) & 2u); }                 // left: TR, BR
        case 2u: { return 20u + ((d >> 2u) & 1u) + ((d >> 2u) & 2u); }                 // top: BR, BL
        case 4u: { return 24u + ((d >> 3u) & 1u) + ((d << 1u) & 2u); }                 // right: BL, TL
        case 8u: { return 28u + (d & 1u) + (d & 2u); }                                 // bottom: TL, TR
        case 5u: { return 32u; }                                                       // left + right
        case 10u: { return 33u; }                                                      // top + bottom
        case 3u: { return 34u + ((d >> 2u) & 1u); }                                    // left + top: BR
        case 6u: { return 36u + ((d >> 3u) & 1u); }                                    // top + right: BL
        case 12u: { return 38u + (d & 1u); }                                           // right + bottom: TL
        case 9u: { return 40u + ((d >> 1u) & 1u); }                                    // bottom + left: TR
        case 7u: { return 42u; }                                                       // left + top + right
        case 11u: { return 43u; }                                                      // left + top + bottom
        case 13u: { return 44u; }                                                      // left + right + bottom
        case 14u: { return 45u; }                                                      // top + right + bottom
        default: { return 46u; }                                                       // alone
    }
}

// Pixel `p` (0..47) of the tile (`kind`, `shape`) in the sheet.
fn tile_px(kind: u32, sh: u32, p: vec2<i32>) -> vec4<f32> {
    let q = p / SUB; // which quarter: 0 or 1 in x and y
    let code = (TABLE[sh] >> u32(5 * (q.y * 2 + q.x))) & 31u;
    let sub = vec2<i32>(i32(code & 3u), i32(code >> 2u));
    let origin = vec2<i32>(i32(kind % 8u), i32(kind / 8u)) * vec2(2 * TILE, 3 * TILE);
    return textureLoad(tileset, origin + sub * SUB + p % SUB, 0);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(textureDimensions(level));
    // cell units from the map's top-left (v is up in the quad, rows go down in the layer), then whole pixels
    let w = vec2(in.uv.x, 1.0 - in.uv.y) * vec2<f32>(dims);
    let cell = vec2<i32>(floor(w));
    let p = vec2<i32>(floor(fract(w) * f32(TILE)));
    let v = kind_at(cell); // kind + 1; neighbours are compared by it, so empty counts as a kind of its own
    if v == 0u {
        discard;
    }
    let t = tile_px(v - 1u, shape(cell, v), p);
    if t.a < 0.5 {
        discard;
    }
    return vec4<f32>(t.rgb, 1.0);
}
