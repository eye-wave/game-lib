// `bun src/check.ts`: the game's levels survive a load -> save round trip unchanged and pass `validate`, scripts
// survive the text form, and the autotile shape matches autotile.wgsl on known cases.
import { readFileSync } from 'node:fs'
import { fromSource, scriptToText, textToScript, toSource, validate } from './level.ts'
import { shape } from './render.ts'

const canon = (v: unknown) => JSON.stringify(v, (_, x) =>
  x && typeof x === 'object' && !Array.isArray(x) ? Object.fromEntries(Object.entries(x).sort()) : x)
const assert = (ok: boolean, msg: string) => { if (!ok) throw new Error(msg) }

for (const name of ['town', 'garden']) {
  const src = JSON.parse(readFileSync(new URL(`../../../apps/tilemap-example/assets/${name}.json`, import.meta.url), 'utf8'))
  const doc = fromSource(src)
  assert(canon(toSource(doc)) === canon(src), `${name}: round trip changed the level`)
  assert(validate(doc).length === 0, `${name}: ${validate(doc)}`)
  for (const e of doc.events) if (e.script)
    assert(canon(textToScript(scriptToText(e.script))) === canon(e.script), `${name}: script round trip`)
  // a broken copy is caught
  doc.layers[0].cells[0] = '☃'
  doc.events.push({ x: doc.w, y: 0, goto: { map: 'x', x: 0, y: 0 } })
  assert(validate(doc).length === 2, `${name}: validate misses mistakes`)
}

const steps = textToScript('Hi\n? Like it?\n  - Yes\n    Me too.\n  - No\n\\- dash\nBye.')
assert(canon(steps) === canon([{ say: 'Hi' }, { ask: { text: 'Like it?', options: [{ text: 'Yes', then: [{ say: 'Me too.' }] }, { text: 'No' }] } }, { say: '- dash' }, { say: 'Bye.' }]), 'script parse')

// 3 x 3 map, kind 1 in the middle (and at the left edge for the last case)
const grid = (cells: number[]) => (x: number, y: number) => cells[Math.min(2, Math.max(0, y)) * 3 + Math.min(2, Math.max(0, x))]
assert(shape(grid([0, 0, 0, 0, 1, 0, 0, 0, 0]), 1, 1, 1) === 46, 'alone')
assert(shape(grid([1, 1, 1, 1, 1, 1, 1, 1, 1]), 1, 1, 1) === 0, 'surrounded')
assert(shape(grid([1, 1, 1, 0, 1, 1, 1, 1, 1]), 1, 1, 1) === 16, 'left edge')
assert(shape(grid([0, 1, 1, 1, 1, 1, 1, 1, 1]), 1, 1, 1) === 1, 'top-left corner')
console.log('ok')
