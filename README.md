# Conway's Game of Life (Bevy)

Conway's Game of Life with a native desktop GUI built on Bevy.

## Demo

![Game of Life screenshot](docs/media/screenshot.png)

![Game of Life demo](docs/media/demo.gif)

## Quick start

```bash
cargo run
```

Local CI-equivalent checks:

```bash
just ci-local
```

## Controls

```text
Controls

Space: run/pause
j: step
r: randomize
x: clear
0: reset epoch
+: speed up
-: slow down
g: select glider stamp
b: select blinker stamp
l: select LWSS stamp
p: select RLE 8x4 stamp
z: rotate stamp
f: flip stamp horizontally
c: clear active stamp
m: toggle boundary mode
F2: cycle layout preset
h: hide/show pattern sidebar
F3: toggle diagnostics
t: toggle grid
/: toggle help
Wheel: zoom camera
WASD / Arrows: pan camera
v: reset camera
Left drag: draw or erase with the selected tool
Left click (stamp): place pattern
Right drag: erase
U or Cmd/Ctrl+Z: undo last edit and pause
Cmd/Ctrl+Shift+Z: redo last undone edit
q / Esc: quit

Pattern palette:
- Use Import pattern to paste RLE and add a stamp
- Select a pattern thumbnail, then click the board to place it

Layout presets:
- Choose Full, Compact, or Minimal from the View menu
- Press F2 to cycle Full -> Compact -> Minimal
- Layout preset is saved between runs
```

Board size is set in the sidebar under Board: pick a preset (128², 256², 512², 1024²) or type exact dimensions, from 8 up to 2048 per side. The choice is saved between runs. Resizing the board clears undo history, because old snapshots no longer fit; resizing the window does not.

The playback toolbar shows speed in generations per second. Draw, Erase, and Stamp tools share a visual pattern palette; rotate and flip controls appear for stamps. In Minimal mode, open Tools to access the palette and board actions. Grid and diagnostics are available in View; diagnostics start hidden.

Undo restores the board and generation before drawing, erasing, placing a stamp, randomizing, or clearing, and pauses playback. Redo reapplies an undone edit, and making a new edit discards the redo chain. Each drag is one edit. History retains up to 32 edits within an approximate 16 MiB budget (with at least one edit), and clears when the board is resized. Imported patterns remain available for the current session.

## Architecture map

Runtime flow from `src/main.rs`:

```text
main
  -> DefaultPlugins + resources
  -> RenderPlugin (world setup, board texture/grid/wall/preview rendering)
  -> UiPlugin (playback toolbar + pattern palette + layout presets + RLE import)
  -> InputPlugin (keyboard, mouse, camera controls)
  -> SimulationPlugin (tick advancement, redraw collection, quit)

Shared resources:
  Simulation { app, accumulator, avg_step_ms, step_samples }
  BoardSettings { cols, rows }
  RedrawRequested
  DiagnosticsOverlay
  GridOverlay

Core domain:
  app::App      (state machine and user actions)
  board::Board  (cell storage, stepping rules, dirty-cell tracking)
  patterns      (builtin patterns + RLE parsing)
```

## Pattern system and RLE

Patterns are data-driven (`src/patterns.rs`) and include builtins plus RLE-derived layouts.

Supported RLE features:

- Header dimensions: `x = <width>, y = <height>`
- Comment lines starting with `#`
- Run counts (`3o`, `10b`, `2$`)
- Row breaks (`$`) and terminator (`!`)

Example:

```text
x = 8, y = 4
8o$8o$8o$8o!
```

## Performance notes

- Board state is stored in a flat contiguous buffer for better cache locality.
- Board size is independent of the window. Resizing the window only changes how much of the board is visible.
- Stepping is adaptive: sparse generations use a frontier update (changed cells plus neighbors), while dense generations fall back to a full scan.
- The frontier path applies flipped cells in place instead of staging a full second buffer, so a sparse generation costs time proportional to the frontier rather than to the board area.
- Cells away from every edge take a neighbour-counting fast path of plain index offsets; only edge cells pay for wrapping or clamping.
- Population is tracked incrementally rather than rescanned, so the status bar does not walk the board every frame.
- Step buffers are recycled between generations, and dirty/frontier flags are cleared by index instead of by full-array fill.
- Dense fallback threshold defaults to 60% candidate coverage and can be overridden with `GOL_FRONTIER_DENSE_THRESHOLD_PERCENT`.
- Rendering is incremental: the board is drawn as a single texture with one texel per cell, and only changed cells are rewritten. Cell count no longer costs one sprite entity each.
- The grid overlay is drawn only for the lines that intersect the viewport, and is skipped entirely below a few pixels per cell, so its cost tracks screen area rather than board area.
- Full redraw is still used as a safe fallback for operations like resize, randomize, and clear.

## Profiling and benchmarking

Baseline runs:

```bash
cargo run --release
cargo test --all-targets --all-features --locked
cargo bench --bench board_step --locked
```

CPU profiling options:

- macOS: use Instruments (`cargo run --release`, attach Time Profiler).
- Linux: use `perf` or `cargo flamegraph` (if installed).

Current benchmark target:

- `benches/board_step.rs`: `board::step` + dirty-cell extraction for sparse glider and deterministic noise scenarios across 256/512/1024 boards.

## Development

Useful commands:

```bash
just fmt-check
just clippy
just test
just ci-local
```
