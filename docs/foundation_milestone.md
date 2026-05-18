Foundation milestone

Build Chelotype's real terminal foundation: one real PTY, one terminal-core state, one shell-driven source of truth, and a custom renderer where keyboard, mouse, and automation are first-class input paths. `libghostty-vt` is the chosen core. Hidden terminals, bridge synchronization, duplicated input, fake local command fields, and VTE HTML scraping have been removed from the active tree.

Success criteria

- The app uses one real PTY per terminal session.
- The shell and terminal core are the only source of truth for visible terminal state.
- Terminal output is parsed into a grid/cell model with text, fg/bg colors, style flags, cursor state, scrollback, and resize/reflow behavior.
- The UI renderer consumes the grid/cell model directly.
- History, scrollback, and current input can be rendered in different UI regions without hidden terminals or duplicated input.
- Keyboard input, mouse clicks, drag selection, mouse-driven cursor placement, terminal mouse reporting, and automation all enter through explicit, testable input paths.
- Headless e2e tests cover typing, Enter, Backspace, arrows, mouse clicks, drag selection, cursor movement, resize, colors/styles, and warnings/errors.
- Performance gates cover p95/p99 input-to-render latency, 60/120 FPS frame budgets, and 10-second held-key scenarios.

Current state audit

- Done: `src/backend.rs` uses one PTY and one `libghostty_vt::Terminal` for parser/state.
- Done: `src/bridge.rs`, `tests/bridge_tests.rs`, and the bridge benchmark were removed.
- Done: `Cargo.toml` no longer depends on `alacritty_terminal`, `vte4`, or bridge-only regex parsing.
- Partial: `src/ghostty_snapshot.rs` exports typed terminal lines/cells/cursor/colors/mouse modes from Ghostty render state.
- Partial: `src/render.rs` consumes terminal cells and emits structured render lines/runs plus GTK markup.
- Partial: `src/diagnostics.rs` and `src/snapshot.rs` provide structured snapshot exports with rows/cols, cursor, mouse mode, plain text, and per-cell text/color/style records.
- Partial: `tests/backend_integration_tests.rs` verifies real PTY write/read, ANSI color preservation, cursor tracking, resize, mouse modes, and scrollback against the active backend.
- Partial: `tests/headless_diagnostics_tests.rs` launches the binary in headless mode, verifies JSON/HTML/text/markup snapshot files, checks expected terminal content, supports scripted action/expectation env overrides, and fails on stderr warnings/errors.
- Partial: headless diagnostics support event-level automation for raw/text writes, key events, resize, scroll, mouse press/drag/release, and selection export.
- Partial: `TerminalBackend::resize` updates both the PTY winsize and Ghostty terminal dimensions; `app.rs` derives terminal size from the GTK viewport.
- Partial: `TerminalBackend::scroll_display` and `scroll_to_bottom` expose scrollback viewport movement from the terminal core, and snapshots export `display_offset`.
- Partial: `src/input.rs` defines a testable keyboard-to-terminal-byte mapping used by the active GTK app.
- Partial: `src/mouse.rs` defines testable SGR mouse event encoding, `src/interaction.rs` owns the pure pointer state machine, Ghostty exposes terminal mouse mode state, and `app.rs` forwards click press/release/drag events to the PTY only when terminal mouse reporting is enabled.
- Partial: `src/selection.rs` defines a typed half-open grid selection model with extraction tests, `src/render.rs` can highlight selected cells from terminal grid coordinates, and `app.rs` starts local drag selection when terminal mouse reporting is not active.
- Partial: `src/canvas.rs` provides a GTK `DrawingArea` render surface that draws terminal markup and caret itself from `RenderFrame`; it still uses full-frame Pango markup rather than an incremental cell renderer.
- Partial: `benches/pipeline.rs` measures the active Ghostty parser/render-state/snapshot/render path.
- Weak: resize is covered at backend/headless level, but not yet by GTK event e2e.
- Weak: first-class mouse support has mode-aware click press/release/drag plumbing and a local grid selection model, but selection is not yet exported to clipboard and still lacks headless GTK drag e2e.
- Weak: benchmarks do not yet prove UI-visible GTK paint smoothness.

Next implementation order

1. Add GTK window event e2e for launch, type, press Enter, resize, click, drag, and assert cells/cursor/colors without warnings.
2. Replace line-level Pango markup rendering with direct per-run/per-cell drawing.
3. Add clipboard selection export.
4. Design shell cursor placement for mouse-driven command editing.
5. Add smooth scrolling and command blocks.
6. Add workspace/tab/split model.
7. Add GTK paint-path perf gates and allocation tracking.
