# CAMCraft

Desktop tool (Rust, egui) that turns DXF drawings into G-code for glue dispensing machines.

## Run

    cargo run --release

Needs the Rust toolchain (rustup). Tested to build on Linux only. Windows build not yet tried.

## Workflow

1. Machines: pick a machine from the list (Add, Clone, Rename, Delete). Sections: Machine (construction, positions, home, Z heights), Speed, End-Effector (type, Binary or PWM control, delays) and Scripts (pre and post script, G-code format).
3. File > New Project, pick a machine. The bed appears.
4. Insert DXF, then drag it on the bed (Move part tool) or type X/Y in the left panel.
5. Sequence paths tool: click paths in the order to glue. Use ^ v to reorder, rev to flip direction.
6. Generate G-code, then Save.

Mouse: wheel zoom, right or middle drag pan.

Data: machines are stored in the user config folder (CAMCraft/library.json). Projects are saved as .camcraft (JSON).

## Supported DXF entities

LINE, LWPOLYLINE, POLYLINE (with bulges), ARC, CIRCLE, SPLINE. Arcs and splines are flattened to short line segments (0.02 mm tolerance), so the output uses G1 only. Touching open entities are joined into one path. Units are assumed to be mm. Other entities are skipped and reported in the status line.

## Status

Version 0.1. Unit tests cover DXF chaining, circles, bulge arcs, splines and G-code output. The GUI starts and renders under a virtual display, but canvas interaction (drag, click-to-sequence) and generated G-code have not been run on a real machine. Dry-run every file with the valve disconnected first.
