# tile-level-editor

Browser editor for the level JSON that `tilemap::build` bakes (see `crates/tilemap/src/lib.rs`).
Rendering, draw order, collision and validation are a TS port of the tilemap crate (`src/render.ts`, `src/level.ts`).

```bash
bun install
bun run dev      # open, drop apps/tilemap-example/assets/town.json + assets/*.png
bun run check    # typecheck + round-trip the game's levels
```

In Chromium, Open/Save write straight back to the JSON file (Ctrl+S, then `cargo run`); elsewhere Save downloads.
Keys: B brush, R rect, F fill, E erase, I pick (or Alt+click), S structures, V events, 1–9 layer, G grid,
C collision, O overlays, 0 fit, Space/middle-drag pan, wheel zoom, right-drag erase, Ctrl+Z / Ctrl+Shift+Z.
