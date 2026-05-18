Completion audit

Objective summary

Chelotype needs one real PTY-backed terminal session, one terminal-core state, shell-driven rendering, first-class keyboard/mouse/automation paths, a deliberate terminal-core choice, custom rendering from grid/cell state, deterministic headless testing, container-friendly assumptions, and a path to modern terminal workspaces.

Prompt-to-artifact checklist

- One real PTY per terminal session
  - Evidence: `src/backend.rs` owns one `portable_pty` master/child and one `alacritty_terminal::Term`.
  - Coverage: `tests/backend_integration_tests.rs::backend_writes_to_single_pty_and_reads_shell_output`.
  - Status: partial; no workspace/pane manager exists yet.
- One terminal-core state / shell-driven source of truth
  - Evidence: `RenderableContentOwned::from_renderable` snapshots `alacritty_terminal` cells/cursor/mouse mode.
  - Coverage: backend integration tests and headless snapshots.
  - Status: partial; the active app still rebuilds full render frames every tick.
- Core choice: `libghostty-vt` vs `alacritty_terminal`
  - Evidence: `docs/adr_terminal_core.md`, `docs/libghostty_spike_findings.md`.
  - Status: partial; `alacritty_terminal` is the current implementation, `libghostty-vt` remains blocked by missing Zig 0.15.x and has not been probed locally.
- No hidden terminals / no bridge synchronization / no fake local command fields
  - Evidence: active app uses `TerminalBackend`; legacy VTE bridge is feature-gated behind `legacy-vte-bridge`.
  - Coverage: default tests run with zero bridge tests; `cargo test --features legacy-vte-bridge --test bridge_tests` keeps the old reference isolated.
  - Status: partial; old bridge code still exists as a feature-gated reference.
- Renderer consumes grid/cell state directly
  - Evidence: `src/render.rs` converts `RenderableContentOwned` lines/cells into `RenderOutput`, `RenderFrame`, and typed `RenderRun` records with start columns and style fields.
  - Evidence: `src/canvas.rs` draws active app frames on a GTK `DrawingArea`.
  - Status: partial; the app has typed render runs but the canvas still paints line markup rather than direct per-run draw batches.
- History, scrollback, current input
  - Evidence: `RenderRegion::{History, Input}`, `RenderFrame.lines`, `TerminalBackend::scroll_display`, `display_offset` snapshots.
  - Coverage: `backend_exposes_scrollback_display_offset`, `headless_mode_replays_scroll_event`.
  - Status: partial; no smooth scroll model and no command-block model.
- Keyboard input
  - Evidence: `src/input.rs`.
  - Coverage: unit tests plus headless keyboard, Backspace, Enter, Ctrl-D, and arrow byte e2e.
  - Status: partial; no GTK window event injection e2e.
- Mouse as first-class input
  - Evidence: `src/mouse.rs`, `src/interaction.rs`.
  - Coverage: local drag selection tests, SGR mouse reporting tests, headless mouse drag selection e2e.
  - Status: partial; no GTK drag event injection e2e, no clipboard export, no shell cursor placement integration.
- Resize/reflow
  - Evidence: `TerminalBackend::resize`.
  - Coverage: backend resize integration and headless resize event e2e.
  - Status: partial; no GTK resize e2e and scrollback reflow edge cases are weak.
- Colors/styles/zsh prompt fidelity
  - Evidence: cell fg/bg/style snapshots and Pango markup renderer.
  - Coverage: ANSI color backend test and headless Unicode/style JSON test.
  - Status: partial; no screenshot/pixel-level app renderer verification.
- Cursor position/shape/visibility
  - Evidence: cursor fields in snapshots, canvas caret drawing.
  - Coverage: backend cursor integration and canvas byte-index unit test.
  - Status: partial; no visual blink/shape e2e after the canvas switch.
- Unicode/graphemes
  - Evidence: Unicode headless snapshot and byte-index unit test.
  - Status: weak; grapheme clusters and wide/zero-width cell behavior need dedicated tests.
- Perf gates
  - Evidence: `benches/pipeline.rs`, `docs/benchmarks.md`.
  - Coverage: active app-frame render benchmark, latency guard, held-key scenario.
  - Status: partial; active frame construction is gated, but not the full GTK/Cairo/Pango paint path, and there is no memory/allocation gate.
- Container-friendly runtime assumptions
  - Evidence: binary-level headless mode works without opening the GTK app.
  - Status: weak; no documented container image, no CI container recipe, no deterministic shell fixture.
- Workspaces, tabs/splits, command blocks, smooth scrolling
  - Evidence: none beyond current docs and data-shape direction.
  - Status: missing.

Current green commands

- `cargo fmt --check`
- `CARGO_BUILD_JOBS=1 cargo check`
- `CARGO_BUILD_JOBS=1 cargo test`
- `CARGO_BUILD_JOBS=1 cargo test --features legacy-vte-bridge --test bridge_tests`
- `CARGO_BUILD_JOBS=1 cargo bench --bench pipeline -- --sample-size 10`

Not done

The milestone is not complete. The next highest-value gaps are GTK window event e2e, per-cell/incremental drawing, shell cursor placement strategy, grapheme/wide-cell tests, clipboard selection, workspace/pane model, and a real `libghostty-vt` probe once Zig 0.15.x is available.
