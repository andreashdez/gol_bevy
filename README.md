# Conway's Game of Life (Bevy)

Conway's Game of Life with a native desktop GUI built on Bevy.

## Demo

![Game of Life screenshot](docs/media/screenshot.png)

![Game of Life demo](docs/media/demo.gif)

## Quick start

```bash
cargo run
```

The interface uses native Bevy UI throughout: playback controls, a searchable pattern
palette, editing tools, live status, and import/settings/help/diagnostics dialogs.

The native palette scrolls with the mouse wheel; scrolling over the canvas zooms it.
The board has its own canvas beside the palette and between the toolbar and status
bar. Startup, board-size changes, and **V** fit the entire board into that space.
Window and layout changes retain the pan position and zoom relative to the fitted
view. Drawing and stamp previews stay inside the canvas, including on Retina displays.
Search patterns by name; builtin and imported patterns have separate groups. The
stamp preview shows the orientation that will be placed, including rotation and
horizontal flip. Filtering the library keeps the current stamp selected.

Drag the native speed slider or focus it and use Left/Right (Home/End for the
limits). The notch past 40 gen/s is **Max**: each frame runs as many generations
as fit in about 8 ms, and the label shows the rate actually achieved. **+** at
40 gen/s also selects Max, and **-** returns to 40 gen/s. Hover or focus buttons for shortcut hints. Empty boards show guidance
that disappears after the first cells are placed. Search and slider focus block
board shortcuts; Escape returns keyboard control to the canvas.

Tab moves between visible, enabled native controls and Enter/Space activates the focused button;
click the canvas to return Space to playback. RLE import, help, diagnostics, and
workspace settings now use native Bevy dialogs. Escape closes a dialog without
quitting; typing and scrolling inside dialogs do not affect the board or camera.
Closing a dialog restores focus to its opener. Native fields expose names and values
to accessibility tools, buttons expose their actions and selected states, and inline
validation messages use polite live announcements. Editors support platform clipboard
shortcuts (Cmd on macOS, Ctrl elsewhere) and Shift+Arrow selection.

Open **Settings** or **Board size…** to choose a preset or enter dimensions, then
apply the size. Invalid values are rejected with an inline message. Full uses a
wider palette, Compact a narrower one, and Minimal hides it. Layout and board
size are saved between runs. Import drafts remain available for the current session.

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
- Choose Full, Compact, or Minimal from Settings
- Press F2 to cycle Full -> Compact -> Minimal
- Layout preset is saved between runs
```

Board size is set in Settings: pick a preset (128², 256², 512², 1024²) or type exact dimensions, from 8 up to 2048 per side. The choice is saved between runs. Resizing the board clears undo history, because old snapshots no longer fit; resizing the window does not.

The playback toolbar shows speed in generations per second. Draw, Erase, and Stamp tools share a visual pattern palette; rotate and flip controls appear for stamps. Use Palette to show or hide the editing tools and board actions. Grid is available in the toolbar and Settings; diagnostics open from Settings or F3 and start hidden.

Undo restores the board, generation, and rule before drawing, erasing, placing a stamp, randomizing, clearing, or importing a pattern that changes the rule, and pauses playback. Redo reapplies an undone edit, and making a new edit discards the redo chain. Each drag is one edit. History retains up to 32 edits within an approximate 16 MiB budget (with at least one edit), and clears when the board is resized. Imported patterns remain available for the current session.

## Architecture map

Runtime flow from `src/main.rs`:

```text
main
  -> DefaultPlugins + resources
  -> RenderPlugin (world setup, board texture/grid/wall/preview rendering)
  -> SettingsPlugin (layout and board-size persistence)
  -> UiInputPlugin (UI regions, modal capture, and keyboard focus)
  -> CanvasViewPlugin (canvas viewport and board framing)
  -> NativeUiPlugin (native toolbar + palette + import/settings/help/diagnostics dialogs)
  -> InputPlugin (keyboard, mouse, camera controls)
  -> SimulationPlugin (tick advancement, redraw collection, quit)

Shared resources:
  Simulation { app, accumulator, avg_step_ms, step_samples }
  BoardSettings { cols, rows }
  RedrawRequested
  DiagnosticsOverlay
  GridOverlay
  UiInputRegions / UiInputCapture

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
- Dense fallback threshold defaults to 5% candidate coverage and can be overridden with `GOL_FRONTIER_DENSE_THRESHOLD_PERCENT`. The dense scan is vectorizable while the frontier step pays per-candidate bookkeeping, so the scan wins early: around 2% for young soup and 5-6% for settled debris on small boards. `cargo bench --bench dense_crossover` measures both, and is worth re-running after changing either path. A frontier that is already over the threshold skips straight to the scan, and candidate collection stops as soon as it crosses it.
- Rendering is incremental: the board is drawn as a single texture with one texel per cell, and only changed cells are rewritten. Cell count no longer costs one sprite entity each.
- The grid overlay is drawn only for the lines that intersect the viewport, and is skipped entirely below a few pixels per cell, so its cost tracks screen area rather than board area.
- A frame runs at most 8 generations to catch up after a stall and then drops the backlog, so a slow frame cannot snowball into a frozen app. Max speed instead spends a fixed time budget per frame, capped at 1024 generations.
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

Benchmark targets:

- `benches/board_step.rs`: `board::step` + dirty-cell extraction for sparse glider and deterministic noise scenarios across 256/512/1024 boards, plus a dense group that steps a freshly seeded 512/1024/2048 board each iteration so it always measures the dense scan.
- `benches/dense_crossover.rs`: times the frontier step against the dense scan on soup covering a growing share of the board, to tune the dense fallback threshold.

## Development

Dev builds optimize dependencies (`[profile.dev.package."*"] opt-level = 3`) so
Bevy stays responsive under `cargo run`; the first build is slower as a result.
For faster rebuilds while iterating, link Bevy dynamically:

```bash
cargo run --features bevy/dynamic_linking
```

Useful commands:

```bash
just fmt-check
just clippy
just test
just ci-local
```
