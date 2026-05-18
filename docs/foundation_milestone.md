Foundation milestone

Build Chelotype's real terminal foundation: one real PTY, one terminal-core state, one shell-driven source of truth, and a custom renderer where keyboard, mouse, and automation are first-class input paths. Choose the core deliberately through a focused `libghostty-vt` vs `alacritty_terminal` spike, then eliminate hidden terminals, bridge synchronization, duplicated input, fake local command fields, and VTE HTML scraping. The architecture must be e2e-testable by design, with deterministic snapshots, structured render-state exports, headless interaction tests, container-friendly runtime assumptions, and a clean path to workspaces, tabs/splits, smooth scrolling, selection, command blocks, and other modern terminal features. The renderer must consume terminal grid/cell state directly and prove that history, scrollback, current input, cursor placement, text selection, mouse-driven cursor movement, terminal mouse reporting, resize/reflow, zsh prompt fidelity, syntax highlighting colors, cursor position/shape/visibility, Unicode/graphemes, and sustained held-key performance all work from the same state model.

Success criteria

- The app uses one real PTY per terminal session.
- The shell and terminal core are the only source of truth for visible terminal state.
- Terminal output is parsed into a grid/cell model with text, fg/bg colors, style flags, cursor state, scrollback, and resize/reflow behavior.
- The UI renderer consumes the grid/cell model directly.
- History, scrollback, and current input can be rendered in different UI regions without hidden terminals or duplicated input.
- Keyboard input, mouse clicks, drag selection, mouse-driven cursor placement, terminal mouse reporting, and automation all enter through explicit, testable input paths.
- The chosen terminal core is documented in an ADR after a focused `libghostty-vt` vs `alacritty_terminal` spike.
- The old VTE bridge is removed from the product path or isolated as a debug/reference backend.
- Headless e2e tests cover typing, Enter, Backspace, arrows, mouse clicks, drag selection, cursor movement, resize, colors/styles, and warnings/errors.
- Performance gates cover p95/p99 input-to-render latency, 60/120 FPS frame budgets, and 10-second held-key scenarios.

Current state audit

- Partial: `src/backend.rs` uses one PTY and `alacritty_terminal` for parser/state.
- Partial: `src/render.rs` consumes terminal cells and emits GTK markup.
- Partial: `src/diagnostics.rs` and `src/snapshot.rs` provide structured snapshot exports with rows/cols, cursor, mouse mode, plain text, and per-cell text/color/style records.
- Partial: `tests/backend_integration_tests.rs` verifies real PTY write/read, ANSI color preservation, and cursor tracking against the active backend.
- Partial: `tests/headless_diagnostics_tests.rs` launches the binary in headless mode, verifies JSON/HTML/text/markup snapshot files, checks expected terminal content, supports scripted action/expectation env overrides, and fails on stderr warnings/errors.
- Partial: headless diagnostics support event-level automation for raw/text writes, key events, resize, scroll, mouse press/drag/release, and selection export.
- Partial: headless diagnostics verify Unicode text and styled cells (bold, italic, underline, ANSI color) in structured JSON snapshots.
- Partial: `TerminalBackend::resize` updates both the PTY winsize and `alacritty_terminal` state; `app.rs` derives terminal size from the GTK viewport.
- Partial: `TerminalBackend::scroll_display` and `scroll_to_bottom` expose scrollback viewport movement from the terminal core, and snapshots export `display_offset`.
- Partial: `src/input.rs` defines a testable keyboard-to-terminal-byte mapping used by the active GTK app.
- Partial: `src/input.rs` maps Shift+PageUp/PageDown to app scrollback actions instead of shell bytes, and `app.rs` also routes wheel scrolling through `TerminalBackend::scroll_display`.
- Partial: `src/mouse.rs` defines testable SGR mouse event encoding, `src/interaction.rs` owns the pure pointer state machine, `backend.rs` exposes terminal mouse mode state, and `app.rs` forwards click press/release/drag events from the GTK overlay to the PTY only when the corresponding SGR mouse reporting mode is enabled.
- Partial: `src/selection.rs` defines a typed half-open grid selection model with extraction tests, `src/render.rs` can highlight selected cells from terminal grid coordinates, and `app.rs` starts local drag selection when terminal mouse reporting is not active.
- Partial: `libghostty-vt` research, ADR evidence, and local Ghostling configure attempt are recorded in `docs/libghostty_spike_findings.md` and `docs/adr_terminal_core.md`; build is blocked by missing Zig 0.15.x.
- Partial: `src/canvas.rs` provides a GTK `DrawingArea` render surface that draws terminal markup and caret itself from `RenderOutput`; this replaces GTK labels in the active app path, but still uses full-frame Pango markup rather than an incremental cell renderer.
- Weak: resize is covered at backend level, but not yet by headless GTK e2e.
- Weak: first-class mouse support has mode-aware click press/release/drag plumbing and a local grid selection model, but selection is not yet exported to clipboard and still lacks headless GTK drag e2e.
- Partial: active binary-level headless diagnostics exercise backend/input/interaction/render paths, but GTK window event injection e2e is still missing.
- Partial: old VTE bridge is isolated behind the `legacy-vte-bridge` Cargo feature; default builds/tests no longer include it, while `cargo test --features legacy-vte-bridge --test bridge_tests` keeps the reference path checked.
- Weak: benchmarks exist, but they do not yet prove UI-visible held-key smoothness.
- Blocked: local verification is currently unreliable while the filesystem has almost no free space.

Next implementation order

1. Stabilize dev gates by freeing enough disk space for `cargo check`, `cargo test`, and benches to run reliably.
2. Register and run all existing benches, including the full pipeline bench.
3. Add an ADR template and a `libghostty-vt` spike plan with explicit pass/fail criteria.
4. Extract the active app path from old bridge code so tests and benches target the current architecture.
5. Add headless e2e for the active app: launch, type, press Enter, assert cells/cursor/colors, fail on warnings.
6. Add resize and mouse e2e before implementing major UI features.
7. Replace label markup rendering with a direct drawing renderer.
