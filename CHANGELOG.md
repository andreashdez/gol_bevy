# Changelog

All notable changes to gol-bevy. Versions follow [Semantic Versioning](https://semver.org).
Add a line under Unreleased in the same pull request as the change; see
[Releasing](https://andreashdez.github.io/gol_bevy/releasing/) for how a release uses it.

## [Unreleased]

### Added

- **Web version** on the docs site: try gol-bevy in a browser without installing it. It
  has no file dialogs or saved settings, runs on one thread, and has no screen reader
  support; see [Try it in your browser](https://andreashdez.github.io/gol_bevy/try-online/).

### Changed

- The dialogs code is split into smaller files for file dialogs, controls, the import
  preview, help, and tests. Nothing changes for players.
- Resizing the board, or the window with Fill canvas on, keeps undo and redo history.
  Undoing past a resize brings back the cells that fit the new size.
- With Fill canvas on, shrinking the window no longer deletes cells near the right or top
  edge: the board stays large enough to hold them, and the view zooms out.
- The stamp preview is drawn as one image, so a large pasted stamp no longer slows every
  frame.

### Fixed

- Trackpad and browser scroll zoom one step at a time, instead of doubling or halving with
  every event.
- On keyboard layouts where `/` is not on its own key, such as German, `/` closes Help as
  well as opening it.
- On Cyrillic, Greek, Hebrew, and Arabic layouts, letter shortcuts and `Cmd/Ctrl` chords
  work by the key's position on a US layout.
- When `Cmd/Ctrl+S` from the board cannot save, or a file dialog is already open, a notice
  over the board says so.
- RLE files that start with a UTF-8 byte-order mark open, as do Golly files whose rule has a
  bounded-grid suffix such as `B3/S23:T64,64`, which is ignored.
- Undo history stays within its memory budget when snapshots of different sizes are mixed.
- An unbounded board frees the working memory a busy pattern used once it calms down or is
  cleared.

## [0.2.0] - 2026-11-17

### Added

- **Select tool** (`4`, a dashed square in the toolbar): drag to select a rectangle of cells.
  `Cmd/Ctrl+A` selects every live cell, `Cmd/Ctrl+C` and `Cmd/Ctrl+X` copy or cut the
  selection as RLE, `Delete` clears it, and `Esc` deselects. Clearing and cutting can be undone.
- **Paste RLE as a stamp** (`Cmd/Ctrl+V`): RLE on the clipboard, from gol-bevy or another
  Life program, follows the pointer and places a copy with each click.
- Release downloads include this changelog.

### Changed

- Undo keeps its full 32 steps on large finite boards. A 2048 × 2048 board used to keep only 4.
- Finite boards store a bit per cell instead of a byte, and step on the same engine as
  unbounded boards.
- Cell age is cheaper to draw, so maximum speed is faster with it on.
- The README is now a short landing page; the full guide is on the
  [documentation site](https://andreashdez.github.io/gol_bevy/).

## [0.1.0] - 2026-11-03

First release, with finite and unbounded boards, a pattern library with stamps, RLE import
and export, saved workspaces, and builds for Linux, Windows, and macOS.

[Unreleased]: https://github.com/andreashdez/gol_bevy/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/andreashdez/gol_bevy/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/andreashdez/gol_bevy/releases/tag/v0.1.0
