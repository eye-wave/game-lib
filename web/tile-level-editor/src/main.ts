import './style.css'
import {
  type Doc, type Entry, type Event, type Kind, type Layer, blank, charFor, format, fromSource, newLayer,
  scriptToText, solidMap, textToScript, toSource, validate,
} from './level.ts'
import { TILE, type Sheet, drawEntry, drawLevel } from './render.ts'

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T
const esc = (s: string) => s.replace(/[&<>"']/g, c => `&#${c.charCodeAt(0)};`)
const mod = (a: number, n: number) => ((a % n) + n) % n
const canvas = $<HTMLCanvasElement>('canvas'), ctx = canvas.getContext('2d')!
const pal = $<HTMLCanvasElement>('palette'), pctx = pal.getContext('2d')!

// ---------------------------------------------------------------- state
type Tool = 'brush' | 'rect' | 'fill' | 'erase' | 'pick' | 'structure' | 'event'
/** Sheet cell (col, row) and size in cells; for an autotile the kind is row * 8 + col. */
interface Brush { col: number; row: number; w: number; h: number; solid: boolean }
type Cell = [number, number]
type Drag =
  | { t: 'pan'; sx: number; sy: number; vx: number; vy: number }
  | { t: 'paint'; erase: boolean; last: Cell; start: Cell; done: Set<string> }
  | { t: 'rect'; erase: boolean; a: Cell; b: Cell }
  | { t: 'newstruct'; a: Cell; b: Cell }
  | { t: 'move'; what: 'event' | 'structure'; i: number; last: Cell; moved: boolean }

let doc: Doc = blank()
let fileName = 'level.json'
let handle: any = null // FileSystemFileHandle (Chromium): Save writes straight back to the opened file
let active = 0
let tool: Tool = 'brush'
let brushes: Record<number, Brush> = {} // per texture index
let sel: { what: 'event' | 'structure'; i: number } | null = null
let hover: Cell | null = null
let drag: Drag | null = null
let space = false, dirty = false
let undos: string[] = [], redos: string[] = []
const images = new Map<string, Sheet>()
const view = { zoom: 1, x: 0, y: 0 }

const layer = (): Layer | undefined => doc.layers[active]
const kindOf = (l?: Layer): Kind | undefined => (l ? doc.textures[l.texture]?.kind : undefined)
const brush = (): Brush => (brushes[layer()?.texture ?? -1] ??= { col: 0, row: 0, w: 1, h: 1, solid: false })
const list = (what: 'event' | 'structure') => (what === 'event' ? doc.events : doc.structures)
const selected = () => sel && list(sel.what)[sel.i]
const gridOn = () => $<HTMLInputElement>('grid').checked
const inMap = (x: number, y: number) => x >= 0 && y >= 0 && x < doc.w && y < doc.h

// ---------------------------------------------------------------- persistence: level in localStorage, images + file handle in IndexedDB
const db = new Promise<IDBDatabase>((ok, fail) => {
  const r = indexedDB.open('tile-level-editor', 1)
  r.onupgradeneeded = () => r.result.createObjectStore('files')
  r.onsuccess = () => ok(r.result)
  r.onerror = () => fail(r.error)
})
async function idb<T>(mode: IDBTransactionMode, f: (s: IDBObjectStore) => IDBRequest): Promise<T> {
  const d = await db
  return new Promise((ok, fail) => {
    const q = f(d.transaction('files', mode).objectStore('files'))
    q.onsuccess = () => ok(q.result)
    q.onerror = () => fail(q.error)
  })
}
const HANDLE = '\0handle'

let saveTimer = 0
function autosave() {
  clearTimeout(saveTimer)
  saveTimer = setTimeout(() => {
    localStorage.setItem('tle', JSON.stringify({ doc, fileName, dirty }))
    renderProblems()
  }, 250)
}

/** Alpha thresholded at 0.5, like the shaders' `discard`, so the canvas shows what the game draws. */
async function toSheet(blob: Blob): Promise<Sheet> {
  const bmp = await createImageBitmap(blob, { premultiplyAlpha: 'none', colorSpaceConversion: 'none' })
  const c = document.createElement('canvas')
  c.width = bmp.width
  c.height = bmp.height
  const x = c.getContext('2d')!
  x.drawImage(bmp, 0, 0)
  const d = x.getImageData(0, 0, c.width, c.height)
  for (let i = 3; i < d.data.length; i += 4) d.data[i] = d.data[i] < 128 ? 0 : 255
  x.putImageData(d, 0, 0)
  return c
}

async function addImage(name: string, blob: Blob, asTexture: boolean) {
  let sheet: Sheet
  try { sheet = await toSheet(blob) } catch { return toast(`${name}: not an image`) }
  images.set(name, sheet)
  await idb('readwrite', s => s.put(blob, name))
  if (asTexture && !doc.textures.some(t => t.name === name)) {
    commit()
    // an A2 sheet is 768 x 576; anything else is most likely tiles
    doc.textures.push({ name, kind: sheet.width === 768 && sheet.height === 576 ? 'autotile' : 'tiles' })
    if (!doc.layers.length) doc.layers.push(newLayer(doc, doc.textures.length - 1))
  }
  renderAll()
}

async function addFiles(files: Iterable<File>) {
  for (const f of files) {
    if (f.name.endsWith('.json')) await openText(await f.text(), f.name, null)
    else await addImage(f.name, f, true)
  }
}

// ---------------------------------------------------------------- undo
function commit() {
  undos.push(JSON.stringify(doc))
  if (undos.length > 200) undos.shift()
  redos = []
  dirty = true
}
function undo(back = true) {
  const from = back ? undos : redos, to = back ? redos : undos
  const s = from.pop()
  if (!s) return
  to.push(JSON.stringify(doc))
  doc = JSON.parse(s)
  active = Math.min(active, Math.max(0, doc.layers.length - 1))
  if (!selected()) sel = null
  renderAll()
}

// ---------------------------------------------------------------- editing primitives
function put(l: Layer, x: number, y: number, e: Entry | null) {
  if (inMap(x, y)) l.cells[y * doc.w + x] = e ? charFor(l, e) : l.empty
}

/** Anchor cells (bottom left) of the sprites on `l` whose footprint covers (x, y). */
function spritesAt(l: Layer, x: number, y: number): number[] {
  const out: number[] = []
  l.cells.forEach((c, i) => {
    const e = l.legend[c]
    if (!e?.tile) return
    const [sw, sh] = e.size ?? [1, 1], ax = i % doc.w, ay = (i / doc.w) | 0
    if (x >= ax && x < ax + sw && y <= ay && y > ay - sh) out.push(i)
  })
  return out
}

function tileEntry(k: Kind, b: Brush, dx: number, dy: number): Entry | null {
  const solid = b.solid ? { solid: true } : {}
  if (k === 'autotile') return { kind: b.row * 8 + b.col, ...solid }
  const c = b.col + mod(dx, b.w), r = b.row + mod(dy, b.h)
  return c === 15 && r === 15 ? null : { tile: [c, r], ...solid } // the last cell of a sheet is unusable
}

function spriteEntry(b: Brush): Entry {
  return { tile: [b.col, b.row], size: [b.w, b.h], ...(b.solid && { solid: true }) }
}

const spriteFits = (b: Brush, x: number, y: number) => x >= 0 && x + b.w <= doc.w && y < doc.h && y + 1 >= b.h

/** The whole brush with its anchor at (x, y): tiles go right / down, a sprite stands on its bottom left cell. */
function stamp(l: Layer, x: number, y: number) {
  const k = kindOf(l)!, b = brush()
  if (k === 'sprites') {
    if (spriteFits(b, x, y)) put(l, x, y, spriteEntry(b))
    return
  }
  for (let dy = 0; dy < b.h; dy++) for (let dx = 0; dx < b.w; dx++) put(l, x + dx, y + dy, tileEntry(k, b, dx, dy))
}

function eraseAt(l: Layer, x: number, y: number) {
  if (kindOf(l) === 'sprites') for (const i of spritesAt(l, x, y)) l.cells[i] = l.empty
  else put(l, x, y, null)
}

function rectFill(l: Layer, [ax, ay]: Cell, [bx, by]: Cell, erase: boolean) {
  const [x0, x1, y0, y1] = [Math.min(ax, bx), Math.max(ax, bx), Math.min(ay, by), Math.max(ay, by)]
  const k = kindOf(l)!, b = brush()
  if (erase) {
    for (let y = y0; y <= y1; y++) for (let x = x0; x <= x1; x++) put(l, x, y, null)
  } else if (k === 'sprites') { // as many whole sprites as fit, from the bottom left
    for (let y = y1; y - b.h + 1 >= y0; y -= b.h) for (let x = x0; x + b.w - 1 <= x1; x += b.w) stamp(l, x, y)
  } else {
    for (let y = y0; y <= y1; y++) for (let x = x0; x <= x1; x++) put(l, x, y, tileEntry(k, b, x - x0, y - y0))
  }
}

function flood(l: Layer, sx: number, sy: number, erase: boolean) {
  const k = kindOf(l)!
  if (k === 'sprites') return toast('Fill works on tile layers')
  const b = brush(), target = l.cells[sy * doc.w + sx], seen = new Uint8Array(l.cells.length)
  const todo: Cell[] = [[sx, sy]]
  while (todo.length) {
    const [x, y] = todo.pop()!
    const i = y * doc.w + x
    if (!inMap(x, y) || seen[i] || l.cells[i] !== target) continue
    seen[i] = 1
    put(l, x, y, erase ? null : tileEntry(k, b, x - sx, y - sy))
    todo.push([x + 1, y], [x - 1, y], [x, y + 1], [x, y - 1])
  }
}

/** Picks what is drawn at (x, y) on the topmost visible layer: that layer and that brush. */
function pick(x: number, y: number) {
  for (let li = doc.layers.length - 1; li >= 0; li--) {
    const l = doc.layers[li]
    if (l.hidden) continue
    const k = kindOf(l)
    const i = k === 'sprites' ? spritesAt(l, x, y).pop() : y * doc.w + x
    const e = i === undefined ? undefined : l.legend[l.cells[i]]
    if (!e) continue
    active = li
    brushes[l.texture] = k === 'autotile'
      ? { col: e.kind! % 8, row: (e.kind! / 8) | 0, w: 1, h: 1, solid: !!e.solid }
      : { col: e.tile![0], row: e.tile![1], w: e.size?.[0] ?? 1, h: e.size?.[1] ?? 1, solid: !!e.solid }
    if (tool === 'pick' || tool === 'erase') setTool('brush')
    return renderAll()
  }
}

function resize(w: number, h: number) {
  if (!(w >= 1 && h >= 1) || (w === doc.w && h === doc.h)) return
  commit()
  for (const l of doc.layers) {
    const cells: string[] = []
    for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) cells.push(x < doc.w && y < doc.h ? l.cells[y * doc.w + x] : l.empty)
    l.cells = cells
  }
  doc.w = w
  doc.h = h // events and structures that fall off show up as problems
  renderAll()
}

// ---------------------------------------------------------------- canvas input
function cellAt(e: PointerEvent | MouseEvent): Cell {
  const r = canvas.getBoundingClientRect()
  return [Math.floor((e.clientX - r.left - view.x) / view.zoom / TILE), Math.floor((e.clientY - r.top - view.y) / view.zoom / TILE)]
}
const structureAt = (x: number, y: number) =>
  doc.structures.findLastIndex(s => x >= s.x && x < s.x + s.size[0] && y <= s.y && y > s.y - s.size[1])
const eventAt = (x: number, y: number) => doc.events.findLastIndex(e => e.x === x && e.y === y)

/** Paints the cells on the line from the last cell to (x, y), with the brush snapped to a grid of its own size
 * from where the stroke began, so stamps tile seamlessly. */
function paintTo(d: Extract<Drag, { t: 'paint' }>, [x, y]: Cell) {
  const l = layer()!, b = brush()
  let [cx, cy] = d.last
  const dx = Math.abs(x - cx), dy = -Math.abs(y - cy), sx = cx < x ? 1 : -1, sy = cy < y ? 1 : -1
  let err = dx + dy
  for (;;) {
    if (d.erase) eraseAt(l, cx, cy)
    else {
      const ax = d.start[0] + Math.floor((cx - d.start[0]) / b.w) * b.w
      const ay = d.start[1] + Math.floor((cy - d.start[1]) / b.h) * b.h
      const key = ax + ',' + ay
      if (!d.done.has(key)) { d.done.add(key); stamp(l, ax, ay) }
    }
    if (cx === x && cy === y) break
    const e2 = 2 * err
    if (e2 >= dy) { err += dy; cx += sx }
    if (e2 <= dx) { err += dx; cy += sy }
  }
  d.last = [x, y]
}

canvas.addEventListener('contextmenu', e => e.preventDefault())
canvas.addEventListener('pointerdown', e => {
  e.preventDefault() // no focus change: a new event's script box keeps the focus
  ;(document.activeElement as HTMLElement)?.blur?.()
  canvas.setPointerCapture(e.pointerId)
  const c = cellAt(e), [x, y] = c, right = e.button === 2
  if (e.button === 1 || space) return void (drag = { t: 'pan', sx: e.clientX, sy: e.clientY, vx: view.x, vy: view.y })
  if (e.button !== 0 && !right) return
  if (e.altKey) return pick(x, y)
  if (tool === 'structure' || tool === 'event') {
    const what = tool, i = what === 'event' ? eventAt(x, y) : structureAt(x, y)
    if (right) {
      if (i >= 0) { sel = { what, i }; deleteSelection() }
      return
    }
    if (i >= 0) {
      sel = { what, i }
      drag = { t: 'move', what, i, last: c, moved: false }
    } else if (what === 'structure') {
      if (inMap(x, y)) drag = { t: 'newstruct', a: c, b: c }
    } else if (inMap(x, y)) {
      commit()
      doc.events.push({ x, y, script: [{ say: 'Hello!' }] })
      sel = { what, i: doc.events.length - 1 }
      renderInspector()
      $<HTMLTextAreaElement>('script')?.select()
    }
    renderInspector()
    return changed()
  }
  const l = layer()
  if (!l || !kindOf(l)) return toast('Add a layer first')
  if (l.hidden) return toast('This layer is hidden')
  if (tool === 'pick') return pick(x, y)
  if (!inMap(x, y)) return
  const erase = right || tool === 'erase'
  commit()
  if (tool === 'fill') flood(l, x, y, erase)
  else if (tool === 'rect') drag = { t: 'rect', erase, a: c, b: c }
  else paintTo(drag = { t: 'paint', erase, last: c, start: c, done: new Set() }, c)
  changed()
})

canvas.addEventListener('pointermove', e => {
  const c = cellAt(e)
  hover = inMap(...c) ? c : null
  renderStatus()
  if (!drag) return requestRender()
  if (drag.t === 'pan') {
    view.x = drag.vx + e.clientX - drag.sx
    view.y = drag.vy + e.clientY - drag.sy
  } else if (drag.t === 'paint') paintTo(drag, c)
  else if (drag.t === 'rect' || drag.t === 'newstruct') drag.b = [Math.max(0, Math.min(doc.w - 1, c[0])), Math.max(0, Math.min(doc.h - 1, c[1]))]
  else if (drag.t === 'move') {
    const dx = c[0] - drag.last[0], dy = c[1] - drag.last[1]
    const o = list(drag.what)[drag.i]
    const lo = drag.what === 'structure' ? (o as any).size[1] - 1 : 0, wd = drag.what === 'structure' ? (o as any).size[0] : 1
    if ((dx || dy) && o.x + dx >= 0 && o.x + dx + wd <= doc.w && o.y + dy >= lo && o.y + dy < doc.h) {
      if (!drag.moved) { commit(); drag.moved = true }
      o.x += dx
      o.y += dy
      drag.last = c
      renderInspector()
    }
  }
  changed()
})

canvas.addEventListener('pointerup', () => {
  const d = drag
  drag = null
  if (d?.t === 'rect') rectFill(layer()!, d.a, d.b, d.erase)
  if (d?.t === 'newstruct') {
    commit()
    const x0 = Math.min(d.a[0], d.b[0]), y1 = Math.max(d.a[1], d.b[1])
    doc.structures.push({ x: x0, y: y1, size: [Math.abs(d.a[0] - d.b[0]) + 1, Math.abs(d.a[1] - d.b[1]) + 1] })
    sel = { what: 'structure', i: doc.structures.length - 1 }
    renderInspector()
  }
  changed()
})
canvas.addEventListener('pointerleave', () => { hover = null; requestRender(); renderStatus() })

canvas.addEventListener('wheel', e => {
  e.preventDefault()
  const r = canvas.getBoundingClientRect(), px = e.clientX - r.left, py = e.clientY - r.top
  const z = Math.min(8, Math.max(0.1, view.zoom * Math.exp(-e.deltaY * 0.0015)))
  view.x = px - (px - view.x) * (z / view.zoom)
  view.y = py - (py - view.y) * (z / view.zoom)
  view.zoom = z
  requestRender()
  renderStatus()
}, { passive: false })

function fit() {
  const W = canvas.clientWidth, H = canvas.clientHeight
  view.zoom = Math.min(4, 0.95 * Math.min(W / (doc.w * TILE), H / (doc.h * TILE)))
  view.x = (W - doc.w * TILE * view.zoom) / 2
  view.y = (H - doc.h * TILE * view.zoom) / 2
  requestRender()
}

// ---------------------------------------------------------------- canvas drawing
let frame = 0
function requestRender() { frame ||= requestAnimationFrame(render) }
function changed() { requestRender(); autosave() }

function render() {
  frame = 0
  const dpr = devicePixelRatio, W = canvas.clientWidth, H = canvas.clientHeight
  if (canvas.width !== Math.round(W * dpr) || canvas.height !== Math.round(H * dpr)) {
    canvas.width = Math.round(W * dpr)
    canvas.height = Math.round(H * dpr)
  }
  ctx.setTransform(1, 0, 0, 1, 0, 0)
  ctx.clearRect(0, 0, canvas.width, canvas.height)
  const z = view.zoom, px = 1 / z // one screen pixel in sheet pixels
  ctx.setTransform(dpr * z, 0, 0, dpr * z, dpr * view.x, dpr * view.y)
  ctx.imageSmoothingEnabled = false
  const mw = doc.w * TILE, mh = doc.h * TILE
  ctx.fillStyle = '#16181d'
  ctx.fillRect(0, 0, mw, mh)
  drawLevel(ctx, doc, images)

  if ($<HTMLInputElement>('collision').checked) {
    ctx.fillStyle = 'rgba(255,60,60,.4)'
    solidMap(doc).forEach((s, i) => s && ctx.fillRect((i % doc.w) * TILE, ((i / doc.w) | 0) * TILE, TILE, TILE))
  }
  if (gridOn()) {
    ctx.beginPath()
    for (let x = 0; x <= doc.w; x++) { ctx.moveTo(x * TILE, 0); ctx.lineTo(x * TILE, mh) }
    for (let y = 0; y <= doc.h; y++) { ctx.moveTo(0, y * TILE); ctx.lineTo(mw, y * TILE) }
    ctx.strokeStyle = 'rgba(255,255,255,.12)'
    ctx.lineWidth = px
    ctx.stroke()
    ctx.strokeStyle = 'rgba(255,255,255,.5)'
    ctx.strokeRect(0, 0, mw, mh)
  }

  const box = (x: number, y: number, w: number, h: number, color: string, width = 2, fill?: string) => {
    if (fill) { ctx.fillStyle = fill; ctx.fillRect(x * TILE, y * TILE, w * TILE, h * TILE) }
    ctx.strokeStyle = color
    ctx.lineWidth = width * px
    ctx.strokeRect(x * TILE, y * TILE, w * TILE, h * TILE)
  }
  const overlays = $<HTMLInputElement>('overlays').checked || tool === 'structure' || tool === 'event'
  if (overlays) {
    ctx.font = `bold ${Math.max(12, 13 * px)}px system-ui`
    doc.structures.forEach((s, i) => {
      const on = sel?.what === 'structure' && sel.i === i
      box(s.x, s.y + 1 - s.size[1], s.size[0], s.size[1], on ? '#ffd23f' : '#3fd0ff', on ? 3 : 2, on ? 'rgba(255,210,63,.08)' : undefined)
      ctx.fillStyle = on ? '#ffd23f' : '#3fd0ff'
      ctx.fillText(`⌂${i + 1}${s.solid ? ' solid' : ''}`, s.x * TILE + 4, (s.y + 1 - s.size[1]) * TILE + 16 * Math.max(1, px))
    })
    doc.events.forEach((e, i) => {
      const on = sel?.what === 'event' && sel.i === i, go = !!e.goto
      box(e.x + 0.1, e.y + 0.1, 0.8, 0.8, on ? '#ffd23f' : '#fff', on ? 3 : 1.5, go ? 'rgba(255,140,0,.55)' : 'rgba(150,80,255,.55)')
      ctx.fillStyle = '#fff'
      ctx.fillText(go ? '→' : '…', e.x * TILE + 18, e.y * TILE + 30)
    })
  }

  const l = layer(), k = kindOf(l), b = brush()
  if (drag?.t === 'rect' || drag?.t === 'newstruct') {
    const [a, c] = [drag.a, drag.b]
    const color = drag.t === 'newstruct' ? '#3fd0ff' : drag.erase ? '#ff5050' : '#ffd23f'
    box(Math.min(a[0], c[0]), Math.min(a[1], c[1]), Math.abs(a[0] - c[0]) + 1, Math.abs(a[1] - c[1]) + 1, color, 2, 'rgba(255,255,255,.08)')
  } else if (hover && l && k && !drag?.t.startsWith('pan')) {
    const [x, y] = hover
    if (tool === 'brush' && !l.hidden) { // ghost of what a click would put down
      ctx.globalAlpha = 0.6
      if (k === 'sprites') drawEntry(ctx, doc, l, spriteEntry(b), x, y, images)
      else for (let dy = 0; dy < b.h; dy++) for (let dx = 0; dx < b.w; dx++) {
        const e = tileEntry(k, b, dx, dy)
        if (e && inMap(x + dx, y + dy)) drawEntry(ctx, doc, l, e, x + dx, y + dy, images)
      }
      ctx.globalAlpha = 1
      const fits = k !== 'sprites' || spriteFits(b, x, y)
      box(x, k === 'sprites' ? y + 1 - b.h : y, b.w, b.h, fits ? '#ffd23f' : '#ff5050')
    } else if (tool !== 'structure' && tool !== 'event') box(x, y, 1, 1, tool === 'erase' ? '#ff5050' : '#fff')
    else box(x, y, 1, 1, 'rgba(255,255,255,.6)', 1)
  }
}

// ---------------------------------------------------------------- panels
function renderAll() {
  renderLayers()
  renderTextures()
  renderPalette()
  renderInspector()
  renderStatus()
  $<HTMLInputElement>('mw').value = String(doc.w)
  $<HTMLInputElement>('mh').value = String(doc.h)
  $('filename').textContent = (handle ? '📄 ' : '') + fileName + (dirty ? ' •' : '')
  $('empty').hidden = doc.textures.length > 0
  document.querySelectorAll<HTMLButtonElement>('#tools button').forEach(b => b.classList.toggle('on', b.dataset.tool === tool))
  changed()
}

function renderLayers() {
  $('layers').innerHTML = doc.layers.map((l, i) => {
    const t = doc.textures[l.texture]
    return `<li data-i="${i}" class="${i === active ? 'on' : ''} ${l.hidden ? 'off' : ''}">
      <button class="eye" data-eye="${i}" title="Show / hide">${l.hidden ? '◌' : '●'}</button>
      <span class="num">${i + 1}</span><span class="name">${esc(t?.name ?? '(no texture)')}</span>
      <span class="kind ${t?.kind}">${t?.kind ?? '?'}</span>${l.structures ? '<span class="badge" title="cells inside structures belong to them">⌂</span>' : ''}
    </li>`
  }).reverse().join('')
  const l = layer()
  $('layerprops').innerHTML = !l ? '' : `
    <label>Texture <select id="ltex">${doc.textures.map((t, i) =>
      `<option value="${i}" ${i === l.texture ? 'selected' : ''}>${esc(t.name)} · ${t.kind}</option>`).join('')}</select></label>
    <label title="Cells of this layer inside a structure sort and fade with it"><input type="checkbox" id="lstruct"
      ${l.structures ? 'checked' : ''} ${kindOf(l) === 'sprites' ? 'disabled' : ''}> Structures</label>
    <div class="row"><button id="lup" title="Move up">↑</button><button id="ldown" title="Move down">↓</button>
      <button id="ldup">Duplicate</button><button id="ldel" class="danger">Delete</button></div>`
}

function moveLayer(to: number) {
  if (to < 0 || to >= doc.layers.length) return
  commit()
  const [l] = doc.layers.splice(active, 1)
  doc.layers.splice(to, 0, l)
  active = to
  renderAll()
}

$('layers').addEventListener('click', e => {
  const t = e.target as HTMLElement
  const eye = t.closest<HTMLElement>('[data-eye]')
  if (eye) {
    const l = doc.layers[+eye.dataset.eye!]
    l.hidden = !l.hidden
    return renderAll()
  }
  const li = t.closest<HTMLElement>('li')
  if (li) { active = +li.dataset.i!; renderAll() }
})
$('layerprops').addEventListener('change', e => {
  const t = e.target as HTMLInputElement, l = layer()!
  commit()
  if (t.id === 'ltex') l.texture = +t.value
  if (t.id === 'lstruct') l.structures = t.checked
  renderAll()
})
$('layerprops').addEventListener('click', e => {
  const id = (e.target as HTMLElement).id
  if (id === 'lup') moveLayer(active + 1)
  if (id === 'ldown') moveLayer(active - 1)
  if (id === 'ldup') {
    commit()
    doc.layers.splice(active + 1, 0, JSON.parse(JSON.stringify(layer())))
    active++
    renderAll()
  }
  if (id === 'ldel' && confirm(`Delete layer ${active + 1}?`)) {
    commit()
    doc.layers.splice(active, 1)
    active = Math.max(0, active - 1)
    renderAll()
  }
})
$('addlayer').onclick = () => {
  if (!doc.textures.length) return toast('Upload a texture first')
  commit()
  doc.layers.splice(active + 1, 0, newLayer(doc, layer()?.texture ?? 0))
  active = doc.layers.length === 1 ? 0 : active + 1
  renderAll()
}

let uploadFor: string | null = null // the texture name a picked image is stored as, null = its own file name
function renderTextures() {
  $('textures').innerHTML = doc.textures.map((t, i) => {
    const ok = images.has(t.name)
    return `<li data-i="${i}">
      <button class="status ${ok ? 'ok' : 'missing'}" data-upload="${esc(t.name)}" title="${ok ? 'Replace image' : 'Image missing: upload it'}">${ok ? '✓' : '⚠'}</button>
      <span class="name" title="${esc(t.name)}">${esc(t.name)}</span>
      <select data-kind="${i}">${['autotile', 'tiles', 'sprites'].map(k => `<option ${k === t.kind ? 'selected' : ''}>${k}</option>`).join('')}</select>
      <button data-dup="${i}" title="Use this image again with another kind">⧉</button>
      <button data-del="${i}" title="Remove">×</button></li>`
  }).join('')
}
$('textures').addEventListener('click', e => {
  const t = e.target as HTMLElement
  if (t.dataset.upload) { uploadFor = t.dataset.upload; pickFile('image/*', false) }
  if (t.dataset.dup) {
    const s = doc.textures[+t.dataset.dup]
    commit()
    doc.textures.push({ name: s.name, kind: s.kind === 'sprites' ? 'tiles' : 'sprites' })
    renderAll()
  }
  if (t.dataset.del) {
    const i = +t.dataset.del, user = doc.layers.findIndex(l => l.texture === i)
    if (user >= 0) return toast(`Layer ${user + 1} uses it`)
    commit()
    doc.textures.splice(i, 1)
    for (const l of doc.layers) if (l.texture > i) l.texture--
    brushes = {}
    renderAll()
  }
})
$('textures').addEventListener('change', e => {
  const t = e.target as HTMLSelectElement
  commit()
  doc.textures[+t.dataset.kind!].kind = t.value as Kind
  renderAll()
})
$('addtex').onclick = () => { uploadFor = null; pickFile('image/*', true) }

// palette: the active layer's sheet; autotiles pick a kind (2 x 3 tiles), tiles and sprites a rectangle of cells
const palGrid = (k: Kind) => (k === 'autotile' ? [2 * TILE, 3 * TILE] : [TILE, TILE])
let palDrag: Cell | null = null
function renderPalette() {
  const l = layer(), k = kindOf(l), t = l && doc.textures[l.texture], img = t && images.get(t.name), b = brush()
  $<HTMLInputElement>('brushsolid').checked = b.solid
  const info = $('brushinfo')
  pal.hidden = !img
  if (!l || !k || !img) {
    info.innerHTML = !l ? 'Add a layer to paint.' : `<button id="palupload">Upload ${esc(t?.name ?? '')}</button> to paint with it.`
    return
  }
  info.textContent = k === 'autotile' ? `Autotile kind ${b.row * 8 + b.col}. Edges follow the 8 neighbours.`
    : k === 'tiles' ? `Tile [${b.col}, ${b.row}]${b.w * b.h > 1 ? `, ${b.w}×${b.h} stamp` : ''}. Drag to select several.`
      : `Sprite [${b.col}, ${b.row}], ${b.w}×${b.h}. Drag to size it; it stands on its bottom left cell.`
  pal.width = img.width
  pal.height = img.height
  pctx.drawImage(img, 0, 0)
  const [gw, gh] = palGrid(k)
  if (gridOn()) {
    pctx.fillStyle = 'rgba(255,255,255,.08)'
    for (let x = gw; x < img.width; x += gw) pctx.fillRect(x, 0, 1, img.height)
    for (let y = gh; y < img.height; y += gh) pctx.fillRect(0, y, img.width, 1)
  }
  pctx.strokeStyle = '#ffd23f'
  pctx.lineWidth = Math.max(2, img.width / 200)
  pctx.strokeRect(b.col * gw, b.row * gh, b.w * gw, b.h * gh)
}
function palCell(e: PointerEvent): Cell {
  const k = kindOf(layer())!, [gw, gh] = palGrid(k), r = pal.getBoundingClientRect()
  const cols = k === 'autotile' ? 8 : 16, rows = k === 'autotile' ? 4 : 16
  const x = Math.floor(((e.clientX - r.left) / r.width) * pal.width / gw), y = Math.floor(((e.clientY - r.top) / r.height) * pal.height / gh)
  return [Math.max(0, Math.min(cols - 1, x)), Math.max(0, Math.min(rows - 1, y))]
}
function palSelect(e: PointerEvent) {
  const [x, y] = palCell(e), b = brush(), multi = kindOf(layer()) !== 'autotile'
  const [ax, ay] = palDrag && multi ? palDrag : [x, y]
  Object.assign(b, { col: Math.min(ax, x), row: Math.min(ay, y), w: Math.abs(ax - x) + 1, h: Math.abs(ay - y) + 1 })
  renderPalette()
}
pal.addEventListener('pointerdown', e => {
  pal.setPointerCapture(e.pointerId)
  palDrag = palCell(e)
  palSelect(e)
  if (tool !== 'rect' && tool !== 'fill') setTool('brush')
})
pal.addEventListener('pointermove', e => palDrag && palSelect(e))
pal.addEventListener('pointerup', () => { palDrag = null })
$('brushsolid').onchange = e => { brush().solid = (e.target as HTMLInputElement).checked }
$('brushinfo').addEventListener('click', e => {
  if ((e.target as HTMLElement).id === 'palupload') { uploadFor = doc.textures[layer()!.texture].name; pickFile('image/*', false) }
})

// inspector: the selected event or structure, else help for the tool
const HELP: Record<Tool, string> = {
  brush: 'Click or drag to paint. Right-drag erases, Alt+click picks from the map. Drag in the palette for a multi-tile stamp; it repeats seamlessly while you drag.',
  rect: 'Drag a rectangle to fill it with the brush (stamps repeat). Right-drag clears it.',
  fill: 'Click to flood-fill the connected area of the same tile. Right-click clears it.',
  erase: 'Click or drag to erase. On a sprite layer it removes the sprite under the cursor.',
  pick: 'Click the map to take its topmost tile or sprite as the brush (and switch to its layer).',
  structure: 'Drag to make a structure (a building): the cells of layers marked ⌂ inside it sort and fade with it as one sprite. Drag one to move it, right-click to delete.',
  event: 'Click an empty cell to add a sign the hero talks to with Z; switch it to a door to change map when stepped on. Drag to move, right-click or Delete to remove.',
}
let editing = false // first change since focusing an inspector field: one undo step per field edit
function renderInspector() {
  const box = $('inspector')
  if (box.contains(document.activeElement) && sel) return // don't rebuild what is being typed in
  const s = selected()
  if (!sel || !s) {
    box.innerHTML = `<h2>${tool[0].toUpperCase() + tool.slice(1)}</h2><p class="hint">${HELP[tool]}</p>`
    return
  }
  if (sel.what === 'structure') {
    const b = s as Doc['structures'][number]
    box.innerHTML = `<h2>Structure ${sel.i + 1} <button id="seldel" class="danger">Delete</button></h2>
      <div class="row"><label>x <input type="number" data-f="x" value="${b.x}"></label><label>bottom row <input type="number" data-f="y" value="${b.y}"></label></div>
      <div class="row"><label>w <input type="number" data-f="w" min="1" value="${b.size[0]}"></label><label>h <input type="number" data-f="h" min="1" value="${b.size[1]}"></label></div>
      <label><input type="checkbox" data-f="solid" ${b.solid ? 'checked' : ''}> Solid bottom row</label>`
    return
  }
  const ev = s as Event
  box.innerHTML = `<h2>Event ${sel.i + 1} <span class="hint">(${ev.x}, ${ev.y})</span> <button id="seldel" class="danger">Delete</button></h2>
    <div class="row seg"><label><input type="radio" name="etype" value="script" ${ev.goto ? '' : 'checked'}> Sign (Z talks)</label>
    <label><input type="radio" name="etype" value="goto" ${ev.goto ? 'checked' : ''}> Door (step on)</label></div>
    ${ev.goto ? `<label>Map <input data-f="map" value="${esc(ev.goto.map)}" placeholder="garden.level"></label>
      <div class="row"><label>x <input type="number" data-f="gx" min="0" value="${ev.goto.x}"></label><label>y <input type="number" data-f="gy" min="0" value="${ev.goto.y}"></label></div>`
    : `<textarea id="script" spellcheck="true" rows="8">${esc(scriptToText(ev.script ?? []))}</textarea>
      <p id="scripterr" class="err"></p>
      <p class="hint">One line per message. <code>? question</code> asks; indent <code>- answer</code> lines under it, and each answer's lines under the answer. Answers without lines carry on after the question.</p>`}`
}
$('inspector').addEventListener('focusin', () => { editing = false })
$('inspector').addEventListener('input', e => {
  const t = e.target as HTMLInputElement, s = selected()
  if (!s) return
  const num = Math.max(0, Math.floor(+t.value || 0))
  const edit = (f: () => void) => { if (!editing) { commit(); editing = true } f(); changed() }
  if (t.name === 'etype') {
    const ev = s as Event
    commit()
    if (t.value === 'goto') { ev.goto = { map: 'garden.level', x: 0, y: 0 }; delete ev.script }
    else { ev.script = [{ say: 'Hello!' }]; delete ev.goto }
    ;(document.activeElement as HTMLElement)?.blur()
    renderInspector()
    return changed()
  }
  if (t.id === 'script') {
    try {
      const steps = textToScript(t.value)
      $('scripterr').textContent = ''
      edit(() => { (s as Event).script = steps })
    } catch (err) { $('scripterr').textContent = (err as Error).message }
    return
  }
  const f = t.dataset.f
  if (!f) return
  edit(() => {
    if ('size' in s) {
      if (f === 'x' || f === 'y') s[f] = num
      if (f === 'w') s.size[0] = Math.max(1, num)
      if (f === 'h') s.size[1] = Math.max(1, num)
      if (f === 'solid') s.solid = t.checked || undefined
    } else if (s.goto) {
      if (f === 'map') s.goto.map = t.value
      if (f === 'gx') s.goto.x = num
      if (f === 'gy') s.goto.y = num
    }
  })
})
$('inspector').addEventListener('click', e => { if ((e.target as HTMLElement).id === 'seldel') deleteSelection() })

function deleteSelection() {
  if (!sel) return
  commit()
  list(sel.what).splice(sel.i, 1)
  sel = null
  ;(document.activeElement as HTMLElement)?.blur()
  renderInspector()
  changed()
}

function renderProblems() {
  const p = validate(doc)
  $('probcount').textContent = p.length ? `⚠ ${p.length}` : '✓'
  $('probcount').className = p.length ? 'err' : 'ok'
  $('problems').innerHTML = p.slice(0, 50).map(m => `<li>${esc(m)}</li>`).join('')
}

function renderStatus() {
  const l = layer(), parts: string[] = []
  if (hover) {
    const [x, y] = hover, c = l?.cells[y * doc.w + x]
    parts.push(`column ${x}, row ${y}`)
    if (l && c !== undefined && c !== l.empty) parts.push(`'${c}' ${JSON.stringify(l.legend[c] ?? '?')}`)
    const ev = doc.events[eventAt(x, y)]
    if (ev) parts.push(ev.goto ? `door → ${ev.goto.map} (${ev.goto.x}, ${ev.goto.y})` : `“${scriptToText(ev.script ?? []).split('\n')[0]}”`)
  }
  parts.push(`${Math.round(view.zoom * 100)}%`)
  $('status').textContent = parts.join('  ·  ')
}

let toastTimer = 0
function toast(msg: string) {
  const t = $('toast')
  t.textContent = msg
  t.classList.add('show')
  clearTimeout(toastTimer)
  toastTimer = setTimeout(() => t.classList.remove('show'), 2500)
}

function setTool(t: Tool) {
  tool = t
  if (t !== 'structure' && t !== 'event') sel = null
  renderAll()
}

// ---------------------------------------------------------------- files
const picker = window as any // File System Access API, Chromium only; elsewhere open = file input, save = download
const JSON_TYPE = [{ description: 'Level JSON', accept: { 'application/json': ['.json'] } }]

function pickFile(accept: string, multiple: boolean) {
  const f = $<HTMLInputElement>('file')
  f.accept = accept
  f.multiple = multiple
  f.value = ''
  f.click()
}
$('file').onchange = async e => {
  const files = [...(e.target as HTMLInputElement).files!]
  if (uploadFor && files[0]) await addImage(uploadFor, files[0], false)
  else await addFiles(files)
  uploadFor = null
}

async function openText(text: string, name: string, h: any) {
  let d: Doc
  try { d = fromSource(JSON.parse(text)) } catch (err) { return toast(`${name}: ${(err as Error).message}`) }
  if (dirty && !confirm('Discard unsaved changes?')) return
  doc = d
  fileName = name
  handle = h
  idb('readwrite', s => (h ? s.put(h, HANDLE) : s.delete(HANDLE)))
  undos = []
  redos = []
  dirty = false
  active = 0
  sel = null
  brushes = {}
  fit()
  renderAll()
  const lost = [...new Set(doc.textures.map(t => t.name).filter(n => !images.has(n)))]
  if (lost.length) toast(`Drop or upload: ${lost.join(', ')}`)
}

async function open() {
  if (!picker.showOpenFilePicker) return pickFile('.json,application/json', false)
  let h
  try { [h] = await picker.showOpenFilePicker({ types: JSON_TYPE }) } catch { return }
  await openText(await (await h.getFile()).text(), h.name, h)
}

async function save(as: boolean) {
  const text = format(toSource(doc))
  if ((as || !handle) && picker.showSaveFilePicker) {
    try { handle = await picker.showSaveFilePicker({ suggestedName: fileName, types: JSON_TYPE }) } catch { return }
    idb('readwrite', s => s.put(handle, HANDLE))
  }
  try {
    if (handle) {
      if ((await handle.queryPermission?.({ mode: 'readwrite' })) === 'prompt') await handle.requestPermission({ mode: 'readwrite' })
      const w = await handle.createWritable()
      await w.write(text)
      await w.close()
      fileName = handle.name
    } else {
      const a = document.createElement('a')
      a.href = URL.createObjectURL(new Blob([text], { type: 'application/json' }))
      a.download = fileName
      a.click()
      URL.revokeObjectURL(a.href)
    }
  } catch (err) { return toast(`Not saved: ${(err as Error).message}`) }
  dirty = false
  const n = validate(doc).length
  toast(n ? `Saved ${fileName}, but it has ${n} problem(s): the build will fail` : `Saved ${fileName}`)
  renderAll()
}

$('new').onclick = () => {
  if (dirty && !confirm('Discard unsaved changes?')) return
  const textures = doc.textures
  doc = blank(doc.w, doc.h)
  doc.textures = textures
  if (textures.length) doc.layers.push(newLayer(doc, Math.max(0, textures.findIndex(t => t.kind === 'autotile'))))
  fileName = 'level.json'
  handle = null
  idb('readwrite', s => s.delete(HANDLE))
  undos = []
  redos = []
  dirty = false
  active = 0
  sel = null
  fit()
  renderAll()
}
$('open').onclick = open
$('save').onclick = () => save(false)
$('saveas').onclick = () => save(true)
$('undo').onclick = () => undo()
$('redo').onclick = () => undo(false)
document.querySelectorAll<HTMLButtonElement>('#tools button').forEach(b => { b.onclick = () => setTool(b.dataset.tool as Tool) })
const redraw = () => { renderPalette(); requestRender() }
for (const id of ['grid', 'collision', 'overlays']) $(id).onchange = redraw
$('mw').onchange = $('mh').onchange = () => resize(+$<HTMLInputElement>('mw').value, +$<HTMLInputElement>('mh').value)

addEventListener('dragover', e => { e.preventDefault(); document.body.classList.add('dropping') })
addEventListener('dragleave', e => { if (!e.relatedTarget) document.body.classList.remove('dropping') })
addEventListener('drop', e => {
  e.preventDefault()
  document.body.classList.remove('dropping')
  addFiles(e.dataTransfer?.files ?? [])
})
addEventListener('resize', requestRender)

// ---------------------------------------------------------------- keyboard
const TOOL_KEYS: Record<string, Tool> = { b: 'brush', r: 'rect', f: 'fill', e: 'erase', i: 'pick', s: 'structure', v: 'event' }
const toggle = (id: string) => { const c = $<HTMLInputElement>(id); c.checked = !c.checked; redraw() }
addEventListener('keydown', e => {
  const k = e.key.toLowerCase(), ctrl = e.ctrlKey || e.metaKey
  if (ctrl && k === 's') { e.preventDefault(); return void save(e.shiftKey) }
  if (ctrl && k === 'o') { e.preventDefault(); return void open() }
  const typing = (e.target as HTMLElement).closest('input, textarea, select')
  if (typing) { if (k === 'escape') (e.target as HTMLElement).blur(); return }
  if (ctrl && (k === 'z' || k === 'y')) { e.preventDefault(); return undo(k === 'z' && !e.shiftKey) }
  if (ctrl) return
  if (k === ' ') { e.preventDefault(); space = true; canvas.style.cursor = 'grab'; return }
  if (TOOL_KEYS[k]) return setTool(TOOL_KEYS[k])
  if (k === 'g') return toggle('grid')
  if (k === 'c') return toggle('collision')
  if (k === 'o') return toggle('overlays')
  if (k === '0') return fit()
  if (k >= '1' && k <= '9' && +k <= doc.layers.length) { active = +k - 1; return renderAll() }
  if (k === 'delete' || k === 'backspace') return deleteSelection()
  if (k === 'escape') { sel = null; renderInspector(); return requestRender() }
})
addEventListener('keyup', e => { if (e.key === ' ') { space = false; canvas.style.cursor = '' } })

// ---------------------------------------------------------------- start: last session's level, every stored image
const saved = JSON.parse(localStorage.getItem('tle') ?? 'null')
if (saved) ({ doc, fileName, dirty } = saved)
renderAll()
requestAnimationFrame(fit)
;(async () => {
  const [keys, vals] = await Promise.all([idb<IDBValidKey[]>('readonly', s => s.getAllKeys()), idb<any[]>('readonly', s => s.getAll())])
  for (let i = 0; i < keys.length; i++) {
    if (keys[i] === HANDLE) handle = vals[i]
    else try { images.set(keys[i] as string, await toSheet(vals[i])) } catch { /* not an image any more */ }
  }
  renderAll()
})()
