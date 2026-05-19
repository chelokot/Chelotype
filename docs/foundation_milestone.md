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
- Partial: `src/diagnostics.rs`, `src/snapshot.rs`, and `src/workspace_render.rs` provide structured snapshot exports with rows/cols, cursor, mouse mode, plain text, per-cell text/color/style records, and multi-pane workspace render frames.
- Partial: `tests/backend_integration_tests.rs` verifies real PTY write/read, ANSI color preservation, cursor tracking, resize, mouse modes, and scrollback against the active backend.
- Partial: `tests/headless_diagnostics_tests.rs` launches the binary in headless mode against a clean interactive `zsh -f` fixture, verifies JSON/HTML/text/markup snapshot files, checks expected terminal content, covers deterministic zsh prompt color cells and emoji/ZWJ/ambiguous-width cells, supports scripted action/expectation env overrides, and fails on stderr warnings/errors.
- Partial: `tests/gtk_e2e_tests.rs` verifies real GTK launch snapshots, ANSI-colored cell export, nonblank real-window screenshots, pixel-level truecolor output, pixel-level narrow cursor shape and position, cursor blink/reset, real Xvfb keyboard input, click-to-cursor movement on the current input row, zsh autosuggestion grid/click behavior, terminal mouse-reporting bytes forwarded to the PTY, real mouse drag selection, stable released-selection copy, PRIMARY/CLIPBOARD selection export, real GTK held-key render/paint latency, and real GTK window resize without GTK warnings.
- Partial: headless diagnostics support event-level automation for raw/text writes, key events, resize, scroll, mouse press/drag/release, click-to-cursor movement, selection export, split-pane creation, pane cycling, and multi-pane workspace render export.
- Partial: `TerminalBackend::resize` updates both the PTY winsize and Ghostty terminal dimensions; `app.rs` derives terminal size from the GTK viewport.
- Partial: `TerminalBackend::scroll_display` and `scroll_to_bottom` expose scrollback viewport movement from the terminal core, and snapshots export `display_offset`.
- Partial: `src/workspace.rs` owns a typed tab/session model where every tab owns one or more panes and every pane owns one `TerminalBackend`; active-tab panes can be snapshotted and resized together for split rendering; `src/workspace_render.rs` exports each pane as a structured render frame with deterministic geometry; `app.rs` routes keyboard, mouse, scroll, resize, scripted input, visible GTK split-pane rendering, and snapshots through the active pane/tab; the header exposes tab buttons and a new-tab button; unit, headless, and GTK e2e tests prove tab switching, isolated terminal state across two top-level tabs, isolated terminal state across two panes inside one tab, simultaneous render-state extraction for both panes in the active tab, deterministic split-pane layout metadata, visible GTK split-pane render dumps, and split-pane render export from two real PTYs.
- Partial: `src/command_blocks.rs` converts semantic-prompt rows from the terminal core into prompt/output block metadata, `RenderFrame.command_blocks` exports it for renderer/diagnostic consumers, and `.render.json` headless dumps now include OSC 133-derived command blocks.
- Partial: `src/input.rs` defines a testable keyboard-to-terminal-byte mapping used by the active GTK app.
- Partial: `src/mouse.rs` defines testable SGR mouse event encoding, `src/interaction.rs` owns the pure pointer state machine, Ghostty exposes terminal mouse mode state, and `app.rs` forwards click press/release/drag events to the PTY only when terminal mouse reporting is enabled.
- Partial: `src/selection.rs` defines a typed half-open grid selection model with extraction tests, `src/render.rs` can highlight selected cells from terminal grid coordinates, and `app.rs` starts local drag selection when terminal mouse reporting is not active. GTK e2e now verifies real Xvfb mouse drag selection and exported `selected_text`.
- Partial: `src/canvas.rs` provides a GTK `DrawingArea` render surface that draws terminal cells at fixed grid columns and draws the caret itself from `RenderFrame`; it is still full-frame rather than dirty-row incremental.
- Partial: `benches/pipeline.rs` measures the active Ghostty parser/render-state/snapshot/render path; `src/perf_trace.rs` and the GTK held-key e2e measure real `gtk_render` and `gtk_paint` durations under a 10-second key hold; the app runtime uses dirty snapshots and skips identical canvas frames.
- Weak: resize is covered at backend, headless, and GTK window-event level, but scrollback reflow edge cases are still weak.
- Weak: first-class mouse support has mode-aware click press/release/drag plumbing, terminal mouse-reporting e2e, basic shell cursor placement on the current input row in both headless and GTK paths, headless soft-wrapped input cursor placement, semantic prompt continuation row cursor placement in unit coverage, a local grid selection model whose drag state stops on release, real GTK drag-selection e2e, and PRIMARY/CLIPBOARD export; richer multi-command cursor placement is still primitive.
- Weak: screenshots now prove the GTK surface is nonblank, truecolor text reaches pixels, and the cursor is rendered as a blinking narrow caret that resets visible after input; GTK held-key e2e now gates visible render/paint p95/p99 plus per-frame allocations and RSS growth, but allocation attribution per keystroke is still missing.

Next implementation order

1. Add advanced grapheme/IME tests.
2. Add smooth scrolling and command-block UI.
3. Add richer split-pane interaction on top of the typed tab/session model and workspace render frames.
4. Add allocation tracking.
