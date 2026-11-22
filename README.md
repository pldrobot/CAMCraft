# CAMCraft

Desktop tool (Rust, egui) that turns DXF drawings (2D projects) and STEP models (3D projects) into G-code for generic CNC machines.

## Machine types

- Milling
- Laser
- Dispensing
- Pick and place (planned)

Machines are defined in the machine library (axes, travel, end effector, G-code format), so the output follows the machine, not a fixed controller.

## Run

    cargo run --release

Needs the Rust toolchain (rustup). Or use `scripts/build.ps1` (Windows) or `scripts/build.sh`.

## Installers (release builds)

- Windows MSI: `scripts/build.ps1 -Installer` (needs WiX Toolset v3 and `cargo install cargo-wix`). Output in `target/wix`.
- Linux .deb: `scripts/build.sh --deb` (needs `cargo install cargo-deb`). Output in `target/debian`.

Branding assets are in `assets/`. Release builds on Windows open without a console window.

## Workflow

1. Machines: pick a machine from the list (Add, Clone, Rename, Delete). Sections: Machine (construction, positions, home, Z heights), Speed, End-Effector (type, Binary or PWM control, delays) and Scripts (pre and post script, G-code format).
2. File > New Project, pick a machine and a project type:
   - **2D (DXF):** the bed appears. Insert DXF, drag it on the bed (Move part tool) or type X/Y in the left panel. In Path selection, click paths in the order to run. Use ^ v to reorder, rev to flip direction.
   - **3D (STEP):** Insert STEP, orient the parts, then add sequence steps. For each step pick the edge or surface, the start and end, and the offsets. Optionally add a lead in between steps.
3. Generate G-code, check it in Preview, then Save.

Mouse: wheel zoom, right or middle drag pan.

Data: machines are stored in the user config folder (CAMCraft/library.json). Projects are saved as .camcraft (JSON).

## Supported DXF entities

LINE, LWPOLYLINE, POLYLINE (with bulges), ARC, CIRCLE, SPLINE. Splines are flattened to short line segments (0.02 mm tolerance). Arcs, circles and bulges are written as G2/G3 when the machine has arcs on, otherwise as G1 lines. Touching open entities are joined into one path. Units are assumed to be mm. Other entities are skipped and reported in the status line.

## Features

2D project (DXF)
- Machine library: bed, travel, axes, end effector (binary or PWM), speeds, pre/post scripts, G-code format. Saved in the user config folder.
- Insert DXF, drag or type X/Y, snapping, locked DXFs.
- Editable path sequence: reorder, reverse, remove, add all, direction arrows and travel lines.
- Job settings: feeds, Z heights, pre-flow and post-flow waits.
- Tabs: Prepare, Path selection, Preview (3D toolpath simulation with play and speed).
- Warnings for paths outside the bed or Z above machine height.
- G2/G3 arcs when the machine has arcs on.

3D project (STEP)
- Insert STEP assemblies, rotate parts, pick edges and faces on the model.
- Sequence of steps with edge, surface, start and end picks, offsets and per-step overrides.
- Lead in between steps that follows picked surfaces and goes round obstacles.
- G2/G3 arcs (helical when Z changes) when the machine has arcs on.
- Stable edge and face numbering when a project is reopened.

Both: G-code preview, save, copy. Projects are saved as .camcraft (JSON).

## Todo

- [ ] Hand test on a real machine (tool or end effector disabled first), and on Windows.
- [ ] Undo and redo.
- [ ] Global shortcuts (Ctrl+S, Ctrl+O, Ctrl+N, zoom fit).
- [ ] Rotate and mirror parts in 2D, snap to grid, grid array of copies.
- [ ] DXF unit detection (`$INSUNITS`), ellipse, hatch and INSERT support.
- [ ] Auto-sequence (nearest neighbour).
- [ ] Lead-in and lead-out for 2D, lead-out for 3D, start offset and overlap on closed paths.
- [ ] Corner slowdown, end effector on/off lag compensation.
- [ ] Machine origin option and work offset, machine import/export.
- [ ] Send G-code over serial or WiFi.
- [ ] SVG import.
- [ ] Pick and place machine type.
- [ ] Milling machine type and path planning: profile (inside, outside, on line), pocket, drilling, facing, engraving, multiple depth passes, tool compensation and tabs.

## Status

Version 0.1. Unit tests cover DXF chaining, circles, bulge arcs, splines and G-code output. Generated G-code has not been run on a real machine. Dry-run every file with the tool, laser or dispenser disabled first.

## Source layout

    src/app      top-level window, menus, New/Open/Save
    src/common   machines, toolpath moves, preview, icons, theme (shared by 2D and 3D)
    src/p2d      2D project: DXF import, canvas, sequence, G-code
    src/p3d      3D project: STEP import, view, G-code

Build scripts are in scripts/.
