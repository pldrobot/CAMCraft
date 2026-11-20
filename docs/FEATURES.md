# CAMCraft features and roadmap

Use this file as the working checklist in VS Code. Tick items as you finish them.

## Setup in VS Code

- Install rustup, then the extensions `rust-analyzer` and `CodeLLDB`.
- Open the folder `E:\PLDRobot\Github\CAMCraft`.
- Run: `cargo run --release`. Tests: `cargo test`.
- First build is slow (about 2 to 5 minutes), later builds are fast.
- Built and tested on Linux only. The Windows build has not been tried yet.

## Code map

| File | Purpose |
|---|---|
| `src/main.rs` | App window, screens (Project, Machines), file dialogs, actions |
| `src/model.rs` | GcodeStyle, Machine, Project, Part, PathData, library storage (JSON) |
| `src/dxf_import.rs` | DXF to paths: flatten arcs and splines, join touching entities |
| `src/gcode.rs` | Machine-driven G-code generator (has unit tests) |
| `src/p2d/canvas.rs` | Bed view: grid, zoom, pan, drag part, click to sequence |
| `src/theme.rs` | AutoCAD-style dark colours |

## Done in v0.1

- [x] Machine management: pick, add, clone, rename, delete. Flavors were removed; all G-code settings live on the machine.
- [x] Machine G-code settings (Scripts and Speed sections): pre and post script, dwell template (`{ms}` or `{s}`), rapid and linear commands, F on rapid, decimals, comments, default job settings.
- [x] Machine management: bed size, axis travel, which axes move the end effector or the bed, end effector type (Motor, Laser, Glue, Other) and control (Binary on/off commands, or PWM with min and max).
- [x] Library saved automatically in the user config folder (`CAMCraft/library.json`).
- [x] New Project: choose a machine, the bed appears.
- [x] Insert DXF: LINE, LWPOLYLINE, POLYLINE (bulges), ARC, CIRCLE, SPLINE.
- [x] Touching entities are joined into one path.
- [x] Drag a part on the bed, or type its X/Y.
- [x] Sequence tool: click paths in order, reorder, reverse direction, remove, add all.
- [x] Direction arrows, order numbers and dashed travel lines on the canvas.
- [x] Job settings: dispense and travel feed, Z on/off, safe Z, dispense Z, pre-flow and post-flow waits.
- [x] Generate G-code, preview, save, copy.
- [x] Warnings: path outside bed, Z above machine height.
- [x] Save and open project files (`.camcraft`, JSON).
- [x] Valve is switched off at the start and the end of every job.

## Known gaps in v0.1

- [ ] Canvas drag and click behaviour not yet tested by hand. Check it first.
- [ ] Output uses G1 lines only, no G2/G3 arcs.
- [ ] DXF units assumed mm. No unit detection.
- [ ] No undo or redo.
- [ ] Machine names are checked for duplicates only in Add, Clone and Rename.
- [ ] Generated G-code never run on a real machine.

## Next features (suggested order)

### Priority 1: make it dependable
- [ ] Hand test on Windows: insert a real DXF, drag, sequence, generate.
- [ ] Undo and redo for move, sequence and delete.
- [ ] Keyboard shortcuts: Delete (remove part), Ctrl+S, Ctrl+O, Ctrl+N, Z (zoom fit), Esc.
- [ ] Rotate and mirror a part. Snap to grid when dragging.
- [ ] DXF unit detection (`$INSUNITS`) with a manual override.
- [ ] Duplicate-name checks and rename handling for machines.

### Priority 2: better gluing
- [ ] Auto-sequence: nearest-neighbour order starting from the origin, with a manual override.
- [ ] Per-path settings: feed, Z height, skip valve (travel only).
- [ ] Start offset and overlap on closed paths, lead-in and lead-out.
- [ ] Corner slowdown.
- [ ] Bead start and stop delay compensation (valve lag).
- [ ] Multiple copies of a part: grid array for batches.
- [ ] Z follows the path (needs a 3D source, ties to the open Z-axis question).

### Priority 3: output and machines
- [ ] G2/G3 arc output, per flavor switch.
- [ ] Extra flavor fields: tool change or pause command, M-code before and after each path, line numbers.
- [ ] Flavor import and export as JSON to share between PCs.
- [ ] Machine origin option (bottom-left, centre) and work offset.
- [ ] G-code preview with simulated travel.
- [ ] Send G-code over serial or WiFi to the controller.

### Priority 4: inputs
- [ ] STEP import: pick edges from a 3D model (needs OpenCascade bindings, large effort).
- [ ] SVG import.
- [ ] Ellipse, hatch and block (INSERT) support in DXF.

## Safety notes for G-code testing

- First runs: valve disconnected or the pneumatic supply off, no glue, no heat.
- Check every flavor command against your firmware. Dwell units differ: GRBL `G4 P` is seconds, Marlin `G4 P` is milliseconds.
- Marlin: `M106`/`M107` can be blocked on some fan pins, and `M42` may be disabled. Verify before relying on it.
- FluidNC and GRBL: `M8`/`M9` map to the coolant output. Confirm the pin in the board config.
- Confirm the machine travel against the Machine settings. The X 125 mm and Y 250 mm travel values in the project notes were still marked "confirm".

## Prompts to paste into Claude Code (VS Code)

- "Read FEATURES.md and src/p2d/canvas.rs. Add undo and redo for part move, sequence changes and part delete."
- "Add G2/G3 arc output as a flavor option. Keep the existing G1 output as the default and add tests."
- "Add auto-sequence (nearest neighbour from the origin) as a button next to Add all, with a test."
- "Add DXF unit detection from $INSUNITS with a manual override in the Insert DXF flow."

- [x] Project tabs: Prepare, Path selection, Preview. Preview is a 3D toolpath simulation (orbit, pan, zoom, play, speed). Enabled after G-code is generated.
- [x] Machine size = bed + travel on each axis. Workable area = part of the bed the end effector can reach (never larger than the bed or the travel). Work zero is the lowest reachable point on the bed; G-code Z 0 is the bed top.
