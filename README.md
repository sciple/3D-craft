# 3D-craft

A lightweight native Windows CAD tool for modeling small parts to 3D print, inspired by
SketchUp's workflow. Built with Rust + Tauri v2 + three.js. Sketch a face, push/pull it into a
solid, refine, and export a print-ready STL (millimeters, Z-up).

## Install

Prerequisites:

- [Rust](https://www.rust-lang.org/tools/install) (stable) with the MSVC toolchain
- [Node.js](https://nodejs.org/) 18+
- Windows with the [Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/) (WebView2 +
  the "Desktop development with C++" build tools)

Then, from the repo root:

```sh
npm install
```

## Run

```sh
npm run tauri dev
```

This launches the full app in a native window (Vite dev server + Rust backend), with hot-reload on
frontend changes and automatic backend recompiles.

To produce a distributable build:

```sh
npm run tauri build
```

## Operation

Pick a tool from the left toolbar (or press its shortcut). Most tools work on the clicked face, or
on the whole selection if the clicked face is already selected. While dragging, **type a number**
to set an exact value (mm, or degrees for Rotate) and press **Enter** to commit; **Esc** cancels.

| Tool | Key | What it does |
| --- | --- | --- |
| Select | `S` | Click a face to select. `Ctrl+D` duplicates the selection. |
| Rectangle | `R` | Draw a rectangle on the ground or on an existing face. |
| Circle | `C` | Draw a circle. |
| Arc | `A` | Click center, click to set radius, click (or type degrees) to set the sweep. Closes with a straight chord - a 180° arc is a half-pipe cross-section. |
| Polygon | `L` | Click points to draw a closed polygon. |
| Push/Pull | `P` | Drag a face along its normal to extrude a solid (or carve inward). |
| Inset | `I` | Offset a face's border inward. |
| Scale | `G` | Scale the selection about its center. |
| Move | `M` | Drag to reposition. Hold `Shift` for vertical; press `X`/`Y`/`Z` to lock to an axis. |
| Rotate | `T` | Drag to rotate. Press `X`/`Y`/`Z` to choose the spin axis (defaults to Z). |
| Measure | `E` | Click two points to read the straight-line distance plus X/Y/Z deltas. Snaps to corners/midpoints/edges, or free points on faces/ground; `Esc` cancels. Creates no geometry. |

Other controls:

- **Camera**: orbit, pan, and zoom with the mouse (SketchUp-style), Z-up.
- **Undo / Redo**: `Ctrl+Z` / `Ctrl+Y` (or `Ctrl+Shift+Z`).
- **Outliner** (right panel): manage groups and mirror a copy of the selection across X/Y/Z.
- **Parts Catalog** (collapsible dropdown, top right): a guided build manifest of spacecraft subsystems (propulsion,
  reactors, crew cabins, solar sails, …) with 3D-print tips. A part's dot turns **green** once
  it's modeled — i.e. once a group of that name exists. Click a green part's **Select** to
  reselect its geometry; with a selection active, click an unmodeled part's **Tag** to mark that
  selection as the part. Click a part's name to expand its description, print tip, and material.
  Edit `src/ui/parts-catalog-data.ts` to grow or change the list.
  - **Mass estimate**: each modeled part shows an estimated mass = enclosed solid volume ×
    scale³ × material density. Pick a **material** in the part's expanded row, and set the global
    **scale** ("1 unit = _ m", default `0.001`, i.e. model drawn true-size in mm) — the header
    shows the running **total dry mass**. Volume is computed from the model geometry; hollow parts
    (thin-walled hulls, tanks) are treated as solid and so read heavy. Material and scale choices
    persist between sessions (stored locally, not in the project file).
- **File menu**: save/open a project, export STL (blocked if the model isn't a watertight,
  printable solid), and **Arrange for Print** (moves every printable solid onto a floor-aligned,
  non-overlapping grid, ready to slice; flat sketches are left alone).

## Development

- `npm run tauri dev` — run the app (primary dev loop)
- `npx tsc --noEmit` — type-check the frontend
- `cd src-tauri && cargo test` — run the Rust test suite

See [CLAUDE.md](CLAUDE.md) for architecture and contributor notes.
