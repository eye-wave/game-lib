// Canvas 2D port of the tilemap crate's drawing: autotile.wgsl / tiles.wgsl / sprites.wgsl per cell, and draw.rs's
// z order (tile layers, then sprites and structures sorted by their bottom row, then layers listed after a sprites
// layer). Sheets are expected alpha-thresholded at 0.5 (see `toSheet` in main.ts), like the shaders' discard.
import type { Doc, Entry, Layer } from './level.ts'

export const TILE = 48
const SUB = 24
export type Sheet = HTMLCanvasElement

// FLOOR_AUTOTILE_TABLE of RPG Maker MV, packed as in autotile.wgsl: (x + 4 * y) of each quarter's subtile,
// 5 bits per quarter, TL in the low bits.
const TABLE = [
  0x6BA32, 0x6BA22, 0x6B872, 0x6B862, 0x3BA32, 0x3BA22, 0x3B872, 0x3B862, 0x69A32, 0x69A22, 0x69872, 0x69862,
  0x39A32, 0x39A22, 0x39872, 0x39862, 0x6B230, 0x6B070, 0x3B230, 0x3B070, 0x6B92A, 0x3B92A, 0x6992A, 0x3992A,
  0x7BA72, 0x79A72, 0x7BA62, 0x79A62, 0xADA32, 0xADA22, 0xAD872, 0xAD862, 0x7B270, 0xAD92A, 0x6B128, 0x3B128,
  0x7B96A, 0x7996A, 0xBDA72, 0xBDA62, 0xAD230, 0xAD070, 0x7B168, 0xAD128, 0xBD270, 0xBD96A, 0xBD168, 0x29020,
]

/** autotile.wgsl `shape`: the 0..46 shape of cell (x, y) whose value is `k`; `at` clamps to the map. */
export function shape(at: (x: number, y: number) => number, x: number, y: number, k: number): number {
  let e = 0, d = 0
  if (at(x - 1, y) !== k) e |= 1
  if (at(x, y - 1) !== k) e |= 2
  if (at(x + 1, y) !== k) e |= 4
  if (at(x, y + 1) !== k) e |= 8
  if (!(e & 3) && at(x - 1, y - 1) !== k) d |= 1
  if (!(e & 6) && at(x + 1, y - 1) !== k) d |= 2
  if (!(e & 12) && at(x + 1, y + 1) !== k) d |= 4
  if (!(e & 9) && at(x - 1, y + 1) !== k) d |= 8
  switch (e) {
    case 0: return d
    case 1: return 16 + ((d >> 1) & 1) + ((d >> 1) & 2)
    case 2: return 20 + ((d >> 2) & 1) + ((d >> 2) & 2)
    case 4: return 24 + ((d >> 3) & 1) + ((d << 1) & 2)
    case 8: return 28 + (d & 1) + (d & 2)
    case 5: return 32
    case 10: return 33
    case 3: return 34 + ((d >> 2) & 1)
    case 6: return 36 + ((d >> 3) & 1)
    case 12: return 38 + (d & 1)
    case 9: return 40 + ((d >> 1) & 1)
    case 7: return 42
    case 11: return 43
    case 13: return 44
    case 14: return 45
    default: return 46
  }
}

/** The value a baked layer stores per cell: autotile kind + 1, tiles row * 16 + column + 1, 0 = empty. */
export function values(d: Doc, l: Layer): Uint8Array {
  const auto = d.textures[l.texture]?.kind === 'autotile'
  const memo = new Map<string, number>()
  return Uint8Array.from(l.cells, c => {
    let v = memo.get(c)
    if (v === undefined) {
      const e = l.legend[c]
      v = !e ? 0 : auto ? (e.kind! >= 0 && e.kind! <= 31 ? e.kind! + 1 : 0)
        : e.tile && e.tile[0] < 16 && e.tile[1] < 16 ? Math.min(e.tile[1] * 16 + e.tile[0] + 1, 255) : 0
      memo.set(c, v)
    }
    return v
  })
}

function autotile(ctx: CanvasRenderingContext2D, img: Sheet, k: number, sh: number, dx: number, dy: number) {
  const ox = (k % 8) * 2 * TILE, oy = ((k / 8) | 0) * 3 * TILE
  for (let q = 0; q < 4; q++) {
    const code = (TABLE[sh] >> (5 * q)) & 31
    ctx.drawImage(img, ox + (code & 3) * SUB, oy + (code >> 2) * SUB, SUB, SUB, dx + (q & 1) * SUB, dy + (q >> 1) * SUB, SUB, SUB)
  }
}

function tile(ctx: CanvasRenderingContext2D, img: Sheet, v: number, dx: number, dy: number) {
  const cols = Math.max(1, (img.width / TILE) | 0) // tiles.wgsl decodes by the sheet's real width
  ctx.drawImage(img, ((v - 1) % cols) * TILE, (((v - 1) / cols) | 0) * TILE, TILE, TILE, dx, dy, TILE, TILE)
}

function missing(ctx: CanvasRenderingContext2D, label: string, x: number, y: number, w = 1, h = 1) {
  ctx.fillStyle = 'rgba(255,0,170,.35)'
  ctx.fillRect(x * TILE + 2, y * TILE + 2, w * TILE - 4, h * TILE - 4)
  ctx.fillStyle = '#fff'
  ctx.font = '20px monospace'
  ctx.fillText(label, x * TILE + 16, y * TILE + 30)
}

function sprite(ctx: CanvasRenderingContext2D, img: Sheet | undefined, e: Entry, c: string, x: number, y: number) {
  const [col, row] = e.tile!, [sw, sh] = e.size ?? [1, 1]
  if (!img) return missing(ctx, c, x, y + 1 - sh, sw, sh)
  ctx.drawImage(img, col * TILE, row * TILE, sw * TILE, sh * TILE, x * TILE, (y + 1 - sh) * TILE, sw * TILE, sh * TILE)
}

/** The whole level in sheet pixels (a cell is 48 x 48), hidden layers left out. */
export function drawLevel(ctx: CanvasRenderingContext2D, d: Doc, images: Map<string, Sheet>) {
  const { w, h } = d
  const owner = new Uint8Array(w * h) // which structure has each cell, + 1; the later one wins
  d.structures.forEach((s, k) => {
    for (let r = Math.max(0, s.y + 1 - s.size[1]); r <= s.y && r < h; r++)
      for (let c = s.x; c < Math.min(w, s.x + s.size[0]); c++) owner[r * w + c] = k + 1
  })
  const depth = (y: number) => 2 - (h - 1 - y) * 0.01 // tilemap::depth of a bottom row
  const ops: { z: number; f: () => void }[] = []
  let index = 0, over = false
  for (const l of d.layers) {
    const t = d.textures[l.texture]
    if (!t) continue
    const img = images.get(t.name)
    if (t.kind === 'sprites') {
      over = true
      if (l.hidden) continue
      l.cells.forEach((c, i) => {
        const e = l.legend[c]
        if (e?.tile) ops.push({ z: depth((i / w) | 0), f: () => sprite(ctx, img, e, c, i % w, (i / w) | 0) })
      })
      continue
    }
    const li = index++
    if (l.hidden) continue
    const v = values(d, l)
    const at = (x: number, y: number) => v[Math.min(h - 1, Math.max(0, y)) * w + Math.min(w - 1, Math.max(0, x))]
    const cell = (i: number) => {
      const x = i % w, y = (i / w) | 0
      if (!img) missing(ctx, l.cells[i], x, y)
      else if (t.kind === 'autotile') autotile(ctx, img, v[i] - 1, shape(at, x, y, v[i]), x * TILE, y * TILE)
      else tile(ctx, img, v[i], x * TILE, y * TILE)
    }
    ops.push({
      z: li * 0.01 + (over ? 3 : 0), f: () => {
        for (let i = 0; i < v.length; i++) if (v[i] && !(l.structures && owner[i])) cell(i)
      },
    })
    if (l.structures) d.structures.forEach((s, k) => ops.push({
      z: depth(s.y) + li * 0.0001, f: () => {
        for (let i = 0; i < v.length; i++) if (v[i] && owner[i] === k + 1) cell(i)
      },
    }))
  }
  ops.sort((a, b) => a.z - b.z) // stable: spawn order breaks ties, as in draw.rs
  for (const o of ops) o.f()
}

/** One legend entry at cell (x, y), for the brush preview (an autotile as its full interior). */
export function drawEntry(ctx: CanvasRenderingContext2D, d: Doc, l: Layer, e: Entry, x: number, y: number, images: Map<string, Sheet>) {
  const t = d.textures[l.texture]
  const img = t && images.get(t.name)
  if (t?.kind === 'sprites') return sprite(ctx, img, e, '?', x, y)
  if (!img) return missing(ctx, '?', x, y)
  if (t.kind === 'autotile') autotile(ctx, img, e.kind!, 0, x * TILE, y * TILE)
  else tile(ctx, img, e.tile![1] * 16 + e.tile![0] + 1, x * TILE, y * TILE)
}
