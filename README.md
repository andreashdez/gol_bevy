# Conway's Game of Life (Bevy)

Conway's Game of Life with a native desktop GUI built on Bevy.

The full documentation is at <https://andreashdez.github.io/gol_bevy/>.

## Demo

![Game of Life screenshot](docs/media/screenshot.png)

![Game of Life demo](docs/media/demo.gif)

## Download

Each tagged release on the [GitHub Releases page](https://github.com/andreashdez/gol_bevy/releases)
has ready-to-run builds, so no Rust toolchain is needed: unpack the archive for your platform and run `gol-bevy`.

- **Linux (x86_64):** needs glibc 2.39 or newer (Ubuntu 24.04, Debian 13, Fedora 40, or
  later) and a Wayland or X11 desktop.
- **Windows (x86_64):** unzip and run `gol-bevy.exe`.
- **macOS (universal):** the binary is not signed, so macOS blocks it the first time.
  Either allow it under System Settings → Privacy & Security, or run
  `xattr -d com.apple.quarantine gol-bevy` in the unpacked folder.

## Quick start

```bash
cargo run
```

The interface uses native Bevy UI throughout: playback controls, a searchable pattern
palette, editing tools, live status, and import/settings/help/diagnostics dialogs.

At 900 pixels wide and above, the toolbar uses a single 64-pixel row, grouped into
playback, board edits, and workspace controls. Narrower windows use two rows.
Resizing preserves keyboard focus and the camera view relative to the fitted board.
The workspace group has icon toggles for the grid, cell age, and pattern palette, which show
whether they are on, plus Import / Export, a gear for Settings, and a ? for Help, all one click
away. Play / Pause, Step, Undo, and Redo are icon buttons too; tooltips give their names and shortcuts.

The pattern library scrolls with the mouse wheel or its scrollbar; the
stamp preview with its rotate/flip controls, and search stay visible while browsing. Scrolling
over the canvas zooms it. Randomize and Clear are always available in the toolbar.
Draw, Erase, and Stamp live in a separate tool strip above the palette. Hiding the
palette turns that strip into a narrow vertical rail, so editing tools remain available.
A quiet hint at the bottom of the canvas describes the active tool. Stamp hints show
the selected pattern, its rotation and flip state, and the Z/F shortcuts, even with
the palette hidden. The hint lets drawing and stamp clicks pass through.
The board has its own canvas beside the palette and between the toolbar and status
bar. Startup, board-size changes, **Fit board**, and **V** fit the entire board into that space.
The status bar's zoom stepper (**− 100% +**) zooms in steps; 100% means the whole-board
fit, and the corner-brackets button beside it is **Fit board**. Middle-button dragging
pans without editing cells.
Window and layout changes retain the pan position and zoom relative to the fitted
view. Drawing and stamp previews stay inside the canvas, including on Retina displays.
The built-in library covers spaceships, oscillators (blinker, pulsar, pentadecathlon), the
Gosper glider gun, and long-lived starters (R-pentomino, acorn, diehard).
Search patterns by name; built-in and imported patterns have separate groups. The
stamp preview shows the orientation that will be placed, including rotation and
horizontal flip. Filtering the library keeps the current stamp selected.

Speed runs from 1 to 40 gen/s, and its last stop is **∞**: as many generations as fit
in about 8 ms per frame. The chevrons beside the number decrease or increase speed by
1 gen/s; **›** at 40 goes to ∞ and **‹** at ∞ returns to 40, like **+** and **-** on the
keyboard. Drag sideways on the numeric speed field to scrub, past 40 to reach ∞;
hold Shift for finer adjustments. Escape during a drag restores the previous speed.
Click the field to enter a rate from 1–40 gen/s, including decimals, or `∞` (`max`
and `inf` work too): Enter applies it, Escape cancels, and leaving the field applies
a valid edited value. Invalid values leave the simulation speed unchanged. Fixed speeds
use millisecond intervals, so the field shows the closest achievable rate. At ∞, a
small readout under the field shows the rate actually achieved while running.
Hover or focus buttons for shortcut hints. Empty boards show guidance
that disappears after the first cells are placed. Search and speed field focus block
board shortcuts; Escape returns keyboard control to the canvas.

Tab moves between visible, enabled native controls and Enter/Space activates the focused button;
click the canvas to return Space to playback. RLE import, help, diagnostics, and
workspace settings now use native Bevy dialogs. Escape closes a dialog;
typing and scrolling inside dialogs do not affect the board or camera.
Closing a dialog restores focus to its opener. Native fields expose names and values
to accessibility tools, buttons expose their actions and selected states, and inline
validation messages use polite live announcements. Editors support platform clipboard
shortcuts (Cmd on macOS, Ctrl elsewhere) and Shift+Arrow selection.

Open **Settings** to choose a board size preset or enter dimensions, then
apply the size. Invalid values are rejected with an inline message. Import drafts
remain available for the current session.

The rule is chosen in Settings as well: pick Life, HighLife, Day & Night, or Seeds, or
type any rulestring in B/S notation (`B36/S23`, or the older `23/36`). Changing the
rule is an undoable edit. Settings also switches between the Dark, Light, and High contrast
themes, which recolour both the board and the interface.

Palette visibility, board size, rule, boundary mode, speed, the grid, cell age, and the theme are saved between runs
in `settings.toml`, in a `gol-bevy` folder in the platform's settings location:

- Linux: `$XDG_CONFIG_HOME`, or `~/.config` if that is unset
- macOS: `~/Library/Application Support`
- Windows: `%APPDATA%`

`GOL_BEVY_CONFIG_DIR` overrides the folder on every platform. Earlier versions saved
`ui_layout.toml`, at first in `~/.config/gol-bevy` everywhere; an old file is read
until the first save writes `settings.toml`, and is left alone. Entries that are
missing or unreadable keep their defaults.

**Import / Export** also copies the board to the clipboard as RLE: the live cells,
cropped to their bounding box, with the board rule in the header. The result pastes
back into Import, or into other Life tools such as Golly.

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
h: hide/show pattern sidebar
F3: toggle diagnostics
t: toggle grid
/: toggle help
Wheel: zoom camera
Middle drag: pan camera
WASD / Arrows: pan camera
v: reset camera
Left drag: draw or erase with the selected tool
Left click (stamp): place pattern
Right drag: erase
U or Cmd/Ctrl+Z: undo last edit and pause
Cmd/Ctrl+Shift+Z: redo last undone edit
q: quit

Pattern palette:
- Use Import / Export to paste RLE as a stamp or copy the board as RLE
- Select a pattern thumbnail, then click the board to place it
```

Board size is set in Settings: pick a preset (128², 256², 512², 1024²) or type exact dimensions, from 8 up to 2048 per side. The choice is saved between runs. **Fill canvas** instead sizes the board to the canvas and keeps it matched as the window or palette changes. Resizing the board clears undo history, because old snapshots no longer fit; resizing the window does not, unless Fill canvas is on.

The playback toolbar shows speed in generations per second. Draw, Erase, and Stamp tools stay available independently of the pattern palette; rotate and flip controls appear in the palette for stamps. Use the Pattern palette toggle to show or hide the library. Grid and Cell age are available in the toolbar and Settings. Cell age, on by default, brightens newborn cells, deepens long-lived ones, and leaves fading trails where cells died; boundary mode is in Settings. The status bar shows a green Running or neutral Paused pill, the generation, the population with its change over the latest generation (for example `842 alive −12`), the rule, which opens Settings at the rulestring when clicked, and the cell under the pointer, plus board dimensions and boundary mode when space allows. Numbers use tabular digits and keep their widest size until the board is cleared or resized, so readouts stay close without shifting. The status
bar stays a single row at every window size. The active rule preset is highlighted in
Settings. Clear, randomize, undo, and redo show brief feedback, including the undo
shortcut where applicable. Diagnostics open from Settings or F3 and start hidden.

Undo restores the board, generation, and rule before drawing, erasing, placing a stamp, randomizing, clearing, changing the rule, or importing a pattern that changes the rule, and pauses playback. Redo reapplies an undone edit, and making a new edit discards the redo chain. Each drag is one edit. History retains up to 32 edits within an approximate 16 MiB budget (with at least one edit), and clears when the board is resized. Imported patterns remain available for the current session.

## Architecture map

Runtime flow from `src/main.rs`:

```text
main
  -> DefaultPlugins + resources
  -> RenderPlugin (world setup, board texture/grid/wall/preview rendering)
  -> SettingsPlugin (palette visibility, board size, rule, boundary, speed, and grid persistence)
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
  CellAge
  UiInputRegions / UiInputCapture

Core domain:
  app::App      (state machine and user actions)
  board::Board  (cell storage, stepping rules, dirty-cell tracking)
  patterns      (built-ins from assets/patterns/ + RLE parsing)
```

## Pattern system and RLE

Built-in patterns are RLE files in `assets/patterns/`. To add one to the palette, drop a
`.rle` file there and rebuild: `build.rs` embeds every file in the binary, so nothing is
read from disk at runtime. The palette lists them in file-name order, named by each
file's `#N` line, or by the file name if there is none. Keyboard shortcuts find their
pattern by file name (`glider`, `blinker`, `lwss`, `rle-8x4`), so renaming or removing
one of those files unbinds its key; the tests check that each still resolves.

Supported RLE features:

- Header dimensions: `x = <width>, y = <height>`
- Comment lines starting with `#`; the first `#N` line names a built-in pattern
- Optional `rule = B3/S23` in the header, which an import applies to the board
- Run counts (`3o`, `10b`, `2$`)
- Row breaks (`$`) and terminator (`!`)

Example:

```text
x = 8, y = 4
8o$8o$8o$8o!
```

## Performance notes

- Board state is stored in a flat contiguous buffer for better cache locality.
- Board size is independent of the window unless Fill canvas is on. Otherwise resizing the window only changes how much of the board is visible.
- Stepping is adaptive: sparse generations use a frontier update (changed cells plus neighbors), while dense generations fall back to a full scan.
- The frontier path applies flipped cells in place instead of staging a full second buffer, so a sparse generation costs time proportional to the frontier rather than to the board area.
- Cells away from every edge take a neighbour-counting fast path of plain index offsets; only edge cells pay for wrapping or clamping.
- Population is tracked incrementally rather than rescanned, so the status bar does not walk the board every frame.
- Step buffers are recycled between generations, and dirty/frontier flags are cleared by index instead of by full-array fill.
- The dense scan runs in bands of whole rows on Bevy's compute task pool, one band per thread, with each band also marking its own changed cells dirty. A band reads only the current generation and writes only its own rows, so no locking is needed. Boards split only into bands of at least 64K cells: below that, the 15-20 µs it costs to dispatch them outweighs the work they share on a settled board.
- A dense step whose changes exceed the fallback threshold does not keep them as a frontier list, since the next step would take the dense scan anyway.
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

The Rust version is pinned in `rust-toolchain.toml` (currently 1.99.0), and CI uses
the matching `rust:1.99.0-trixie` image, so a new Rust release cannot change what
CI accepts. Inside the project, rustup picks the pinned version; if it is not
installed yet, run `rustup toolchain install` once. To move to a newer Rust, change
the version in both files together.

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

### Releasing

Pushing a tag such as `v0.2.0` runs the usual checks, then builds Linux, Windows,
and universal macOS archives (Windows is cross-compiled with MinGW) and creates a
GitHub release with them attached:

```bash
git tag v0.2.0 && git push origin v0.2.0
```

To try the macOS release build locally, run `just release-macos v0.2.0`.
`scripts/package-release.sh` gives every platform the same archive layout:
the binary, this README, and the licence.
