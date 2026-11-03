// The level source format of `crates/tilemap/src/bake.rs` (what `tilemap::build` reads), as the editor holds it:
// rows become a flat cell array of legend characters. `validate` and `solidMap` mirror `bake`'s checks and its
// collision rules, so what passes here bakes.

export type Kind = 'autotile' | 'tiles' | 'sprites'
export interface Entry { kind?: number; tile?: [number, number]; size?: [number, number]; solid?: boolean }
export interface Texture { name: string; kind: Kind }
export interface Structure { x: number; y: number; size: [number, number]; solid?: boolean }
export interface Answer { text: string; then?: Step[] }
export type Step = { say: string } | { ask: { text: string; options: Answer[] } }
export type Goto = { map: string; x: number; y: number }
export type Event = { x: number; y: number; script?: Step[]; goto?: Goto }
export interface Layer {
  texture: number
  legend: Record<string, Entry>
  cells: string[] // w * h characters, row 0 = top
  empty: string
  structures: boolean
  hidden?: boolean // editor only, not exported
}
export interface Doc { w: number; h: number; textures: Texture[]; layers: Layer[]; structures: Structure[]; events: Event[] }

export const blank = (w = 20, h = 15): Doc => ({ w, h, textures: [], layers: [], structures: [], events: [] })

export const newLayer = (d: Doc, texture: number): Layer =>
  ({ texture, legend: {}, cells: Array(d.w * d.h).fill('.'), empty: '.', structures: false })

/** Lenient: rows that are too short are padded with the empty character (and so get fixed on save). */
export function fromSource(s: any): Doc {
  const src: any[] = s.layers ?? []
  const h = src[0]?.rows?.length ?? 15
  const w = src[0] ? [...(src[0].rows[0] ?? '')].length : 20
  const layers = src.map((l): Layer => {
    const empty = l.empty ?? '.'
    const cells: string[] = []
    for (let y = 0; y < h; y++) {
      const row = [...(l.rows?.[y] ?? '')] // chars, not UTF-16 units: Rust counts chars
      for (let x = 0; x < w; x++) cells.push(row[x] ?? empty)
    }
    return { texture: l.texture, legend: l.legend ?? {}, cells, empty, structures: !!l.structures }
  })
  return { w, h, textures: s.textures ?? [], layers, structures: s.structures ?? [], events: s.events ?? [] }
}

export function toSource(d: Doc) {
  const out: any = {
    textures: d.textures.map(t => ({ name: t.name, kind: t.kind })),
    layers: d.layers.map(l => ({
      texture: l.texture,
      ...(l.empty !== '.' && { empty: l.empty }),
      ...(l.structures && { structures: true }),
      legend: l.legend,
      rows: Array.from({ length: d.h }, (_, y) => l.cells.slice(y * d.w, (y + 1) * d.w).join('')),
    })),
  }
  if (d.structures.length) out.structures = d.structures
  if (d.events.length) out.events = d.events
  return out
}

/** Pretty JSON with `[col, row]` pairs kept on one line, like the hand-written levels. */
export const format = (s: unknown) =>
  JSON.stringify(s, null, 2).replace(/\[\s+(-?\d+),\s+(-?\d+)\s+\]/g, '[$1, $2]') + '\n'

const sameEntry = (a: Entry, b: Entry) =>
  a.kind === b.kind && !!a.solid === !!b.solid && String(a.tile) === String(b.tile) &&
  String(a.tile && (a.size ?? [1, 1])) === String(b.tile && (b.size ?? [1, 1]))

const POOL = [...'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789#$%&*+=@!?~^<>/|;:',
  ...Array.from({ length: 0x250 - 0xc0 }, (_, i) => String.fromCodePoint(0xc0 + i))]

/** The legend character for `e` on `l`, adding one if no entry matches. */
export function charFor(l: Layer, e: Entry): string {
  for (const [c, x] of Object.entries(l.legend)) if (sameEntry(x, e)) return c
  const c = POOL.find(c => c !== l.empty && !(c in l.legend))
  if (!c) throw new Error('legend is full')
  l.legend[c] = e
  return c
}

const fitsMap = (d: Doc, x: number, y: number, w: number, h: number) =>
  w > 0 && h > 0 && x + w <= d.w && y < d.h && y + 1 >= h

/** `bake`'s checks, one message per mistake (not per cell). */
export function validate(d: Doc): string[] {
  const err = new Set<string>()
  if (d.layers.length < 1 || d.layers.length > 100) err.add('needs 1 to 100 layers')
  d.layers.forEach((l, li) => {
    const name = `layer ${li + 1}`
    const t = d.textures[l.texture]
    if (!t) return void err.add(`${name}: there is no texture ${l.texture}`)
    if (l.structures && t.kind === 'sprites') err.add(`${name}: "structures" is for tile layers`)
    l.cells.forEach((c, i) => {
      const e = l.legend[c]
      const at = `${name}: \`${c}\``
      if (!e) return void (c !== l.empty && err.add(`${at} is not in the legend`))
      if (t.kind === 'autotile') {
        if (!Number.isInteger(e.kind) || e.kind! < 0 || e.kind! > 31) err.add(`${at} needs kind 0..31 (autotile)`)
        if (e.tile || e.size) err.add(`${at}: tile and size are for tiles and sprites`)
      } else if (!e.tile) err.add(`${at} needs tile (${t.kind})`)
      else if (t.kind === 'tiles') {
        const [col, row] = e.tile
        if (col >= 16 || row >= 16 || row * 16 + col > 254 || e.kind != null || e.size)
          err.add(`${at}: needs tile inside a 16 x 16 sheet (the last cell is unusable), no kind, no size`)
      } else {
        const [col, row] = e.tile, [sw, sh] = e.size ?? [1, 1]
        if (e.kind != null || sw <= 0 || sh <= 0 || col + sw > 16 || row + sh > 16)
          err.add(`${at}: needs tile and size inside the 16 x 16 sheet, no kind`)
        else if (!fitsMap(d, i % d.w, (i / d.w) | 0, sw, sh))
          err.add(`${at} at column ${i % d.w}, row ${(i / d.w) | 0}: a ${sw} x ${sh} sprite does not fit on the map`)
      }
    })
  })
  if (d.structures.length > 255) err.add('at most 255 structures')
  d.structures.forEach((s, i) => {
    if (!fitsMap(d, s.x, s.y, s.size[0], s.size[1])) err.add(`structure ${i + 1} does not fit on the map`)
  })
  const asks = (steps: Step[]): boolean =>
    steps.every(s => 'say' in s || (s.ask.options.length >= 1 && s.ask.options.length <= 4 && s.ask.options.every(o => asks(o.then ?? []))))
  d.events.forEach((e, i) => {
    if (e.x >= d.w || e.y >= d.h) err.add(`event ${i + 1}: (${e.x}, ${e.y}) is outside the map`)
    if (e.script && !asks(e.script)) err.add(`event ${i + 1}: a question needs 1 to 4 answers`)
  })
  return [...err]
}

/** Which cells block, as `bake` computes it: tiles' `solid`, a sprite's or solid structure's bottom row. */
export function solidMap(d: Doc): Uint8Array {
  const s = new Uint8Array(d.w * d.h)
  for (const l of d.layers) {
    const sprites = d.textures[l.texture]?.kind === 'sprites'
    l.cells.forEach((c, i) => {
      const e = l.legend[c]
      if (!e?.solid) return
      if (!sprites) return void (s[i] = 1)
      const sw = e.size?.[0] ?? 1
      if (i % d.w + sw <= d.w) s.fill(1, i, i + sw)
    })
  }
  for (const b of d.structures) if (b.solid && fitsMap(d, b.x, b.y, b.size[0], b.size[1])) s.fill(1, b.y * d.w + b.x, b.y * d.w + b.x + b.size[0])
  return s
}

// Scripts as indented text:   a line says it;  "? question" asks, its "- answer" lines indented under it, and
// each answer's own steps indented under the answer. A line that has to start with ? - or \ gets a \ in front.
export function scriptToText(steps: Step[], ind = ''): string {
  return steps.map(s => {
    if ('say' in s) {
      const t = s.say.replace(/\s*\n\s*/g, ' ')
      return ind + (/^[?\-\\]/.test(t) ? '\\' : '') + t
    }
    return [ind + '? ' + s.ask.text, ...s.ask.options.map(o =>
      ind + '  - ' + o.text + (o.then?.length ? '\n' + scriptToText(o.then, ind + '    ') : ''))].join('\n')
  }).join('\n')
}

export function textToScript(src: string): Step[] {
  const lines = src.split('\n').map((l, n) => ({ n: n + 1, l: l.replace(/\t/g, '  ') }))
    .filter(x => x.l.trim()).map(({ n, l }) => ({ n, ind: l.length - l.trimStart().length, t: l.trim() }))
  let i = 0
  const block = (ind: number): Step[] => {
    const out: Step[] = []
    while (i < lines.length && lines[i].ind === ind) {
      const { t, n } = lines[i++]
      if (t.startsWith('-')) throw new Error(`line ${n}: an answer goes under a "? question", indented`)
      if (!t.startsWith('?')) { out.push({ say: t.startsWith('\\') ? t.slice(1) : t }); continue }
      const options: Answer[] = []
      if (i < lines.length && lines[i].ind > ind) {
        const oi = lines[i].ind
        while (i < lines.length && lines[i].ind === oi) {
          const o = lines[i++]
          if (!o.t.startsWith('-')) throw new Error(`line ${o.n}: answers start with "-"`)
          const a: Answer = { text: o.t.slice(1).trim() }
          if (i < lines.length && lines[i].ind > oi) a.then = block(lines[i].ind)
          options.push(a)
        }
      }
      out.push({ ask: { text: t.slice(1).trim(), options } })
    }
    return out
  }
  const steps = lines.length ? block(lines[0].ind) : []
  if (i < lines.length) throw new Error(`line ${lines[i].n}: unexpected indentation`)
  return steps
}
