Goal
- Drop the dual-VTE bridge and render everything ourselves from a single PTY.
- Keep one PTY/session (any shell), parse output into a screen grid, render history ourselves, and render the current input line in our custom overlay field (no duplication, no hidden terminal).
- This plan is now governed by `docs/foundation_milestone.md`, `docs/terminal_core_spike.md`, and `docs/adr_terminal_core.md`.

Architecture plan
1) PTY backend
   - Use `portable-pty` to spawn the user’s shell with a single PTY.
   - Expose async read loop that pushes incoming bytes to the parser.
   - Forward key/input bytes from our UI directly into the PTY (same as now, but single sink).

2) Terminal emulation (state machine)
   - Choose between `alacritty_terminal` and `libghostty-vt` through the spike, then keep only one product path.
   - Maintain Screen/Scrollback state in memory; expose:
     - `grid_rows(): Vec<Line>` for history + current screen.
     - `cursor_position()` and cell attributes (fg/bg, bold, underline, etc.).
   - Configure tab width, wrap, and scrollback size similar to current VTE.

3) Rendering
   - History: render the parsed grid lines into a GTK container (e.g., custom DrawingArea or existing Label overlay) with our markup pipeline.
   - Input line: render the current line (from parser state) into our custom field; caret is ours (already implemented overlay cursor).
   - No duplication: we never show the terminal’s raw prompt/line; we draw it ourselves from the grid state.
   - Scrollback: map screen+scrollback from alacritty_terminal to our GTK scrollable area.

4) Input handling
   - Map keys to escape sequences (reuse current key mapping, expand as needed).
   - Send bytes directly to PTY.
   - When we type, the parser also receives echo from the PTY; we keep rendering from parser state, not from GTK input widgets.

5) Sync model
   - PTY read loop -> parser (screen update) -> schedule render (throttled to frame budget, e.g., 120 Hz).
   - UI input -> PTY write immediately; parser will reflect once echo arrives (single source of truth).
   - Use idle/timeout coalescing for renders to avoid per-byte redraws.

6) Bench/test impact
   - Update benches to include parser + render pipeline (alacritty_terminal + our markup) as “full frame” benchmarks.
   - Keep jitter/frame-budget detectors to fail on render stalls.
   - Keep dual-terminal bridge tests behind the `legacy-vte-bridge` feature until migration is complete; default checks must target the active single-PTY architecture.

7) Migration steps
   - Add `portable-pty` and `alacritty_terminal` dependencies.
   - Implement PTY spawn + read loop in a new module (e.g., `term_backend.rs`).
   - Implement parser/screen wrapper to expose lines and cursor attributes.
   - Replace VTE shadow/output usage in `ui.rs` with the new backend; reuse overlay cursor field.
   - Keep current UI layout; history rendered via our data, input via our overlay field.
   - Gradually delete dual-terminal bridge code once parity is reached.
