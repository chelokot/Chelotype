Completion audit

Objective summary

Chelotype needs one real PTY-backed terminal session, one terminal-core state, shell-driven rendering, first-class keyboard/mouse/automation paths, a deliberate terminal-core choice, custom rendering from grid/cell state, deterministic headless testing, container-friendly assumptions, and a path to modern terminal workspaces.

Prompt-to-artifact checklist

- One real PTY per terminal session
  - Evidence: `src/backend.rs` owns one `portable_pty` master/child per `TerminalBackend`.
  - Coverage: `backend_writes_to_single_pty_and_reads_shell_output`.
  - Status: partial; no workspace/pane manager exists yet.
- One terminal-core state / shell-driven source of truth
  - Evidence: `src/backend.rs` owns one `libghostty_vt::Terminal`; PTY reader bytes are fed into that terminal state.
  - Coverage: backend integration tests and headless snapshots.
  - Status: partial; active app still rebuilds full render frames every tick.
- Core choice
  - Evidence: `docs/adr_terminal_core.md`.
  - Status: done for this stage; `libghostty-vt` is selected and implemented in the active backend.
- No hidden terminals / no bridge synchronization / no fake local command fields
  - Evidence: `src/bridge.rs`, `tests/bridge_tests.rs`, and the bridge bench were removed; `Cargo.toml` has no VTE bridge feature.
  - Status: done for the active tree.
- Renderer consumes grid/cell state directly
  - Evidence: `src/ghostty_snapshot.rs` exports `TerminalContent`; `src/render.rs` renders typed cells/runs from that model; `src/canvas.rs` paints structured runs by grid column and draws the cursor from grid coordinates.
  - Status: partial; canvas no longer paints whole-line markup and has nonblank screenshot plus pixel-level truecolor e2e, but still uses per-run Pango markup for text attributes and does not yet have incremental dirty-row paint or pixel-level cursor assertions.
- History, scrollback, current input
  - Evidence: `RenderRegion::{History, Input}`, `RenderFrame.lines`, `TerminalBackend::scroll_display`, `display_offset` snapshots, and Ghostty row metadata (`wrapped`, `wrap_continuation`, semantic prompt) exported through `TerminalContent`.
  - Coverage: `backend_exposes_scrollback_display_offset`, `headless_mode_replays_scroll_event`, `ghostty_snapshot::tests::snapshot_exports_soft_wrap_metadata`, `render::tests::renderer_marks_soft_wrapped_cursor_line_as_single_input_region`.
  - Status: partial; soft-wrapped cursor input now stays in one input region, but no smooth scroll model and no command-block model.
- Keyboard input
  - Evidence: `src/input.rs`.
  - Coverage: unit tests plus headless keyboard, Backspace, Enter, Ctrl-D, arrow byte e2e, `tests/gtk_e2e_tests.rs` real-window Xvfb smoke, and Xvfb+xdotool keyboard input into the actual window.
  - Status: partial; real keyboard input reaches the shell through GTK, with more IME/layout scenarios still needed.
- Mouse as first-class input
  - Evidence: `src/mouse.rs`, `src/interaction.rs`, Ghostty mouse-mode state from `src/ghostty_snapshot.rs`.
  - Coverage: local drag selection tests, SGR mouse reporting tests, headless mouse drag selection e2e, real Xvfb+xdotool GTK drag-selection e2e that verifies exported `selected_text`, and real Xvfb+xdotool terminal mouse-reporting e2e that verifies SGR click bytes reach the PTY.
  - Status: partial; PRIMARY/CLIPBOARD export is covered, but no shell cursor placement integration.
- Resize/reflow
  - Evidence: `TerminalBackend::resize` updates PTY winsize and Ghostty terminal dimensions.
  - Coverage: backend resize integration, headless resize event e2e, and real Xvfb+xdotool GTK window resize e2e that verifies changed snapshot rows.
  - Status: partial; scrollback reflow edge cases are weak.
- Colors/styles/zsh prompt fidelity
  - Evidence: Ghostty cell fg/bg/style snapshots and renderer exports.
  - Coverage: ANSI color backend test, headless Unicode/style JSON test, Xvfb real-window smoke snapshot, GTK color e2e that verifies ANSI-colored output cells in JSON, and pixel-level truecolor screenshot e2e.
  - Status: partial; truecolor reaches pixels, but zsh prompt/theme fidelity is still not deeply asserted.
- Cursor position/shape/visibility
  - Evidence: cursor fields in snapshots, canvas caret drawing.
  - Coverage: backend cursor integration and canvas byte-index unit test.
  - Status: partial; no visual blink/shape e2e after the canvas switch.
- Unicode/graphemes
  - Evidence: Ghostty grapheme extraction, shared `src/cell_text.rs`, structured cell JSON with wide-cell flags.
  - Coverage: render/snapshot/selection unit tests preserve combining marks and skip wide-cell spacers; headless Unicode/style JSON test checks combining marks and wide-cell metadata.
  - Status: partial; emoji ZWJ, ambiguous-width, and IME/composition scenarios are still needed.
- Perf gates
  - Evidence: `benches/pipeline.rs`, `docs/benchmarks.md`.
  - Coverage: active Ghostty app-frame render benchmark, 4 ms latency guard, explicit 60/120 Hz frame-budget gates, held-key scenario, backend dirty-snapshot regression test, and canvas frame equality skip.
  - Status: partial; active frame construction is gated and idle repaint churn is reduced, but not the full GTK/Cairo/Pango paint path, and there is no memory/allocation gate.
- Container-friendly runtime assumptions
  - Evidence: `scripts/with-zig.sh`, binary-level headless mode, README setup/run commands, and Xvfb-based GTK e2e test.
  - Status: partial; no CI container image or deterministic shell fixture beyond the current headless `/bin/sh` diagnostics.
- Workspaces, tabs/splits, command blocks, smooth scrolling
  - Evidence: none beyond current data-shape direction.
  - Status: missing.

Current green commands

- `bash scripts/with-zig.sh cargo fmt -- --check`
- `bash scripts/with-zig.sh cargo check`
- `bash scripts/with-zig.sh cargo test`
- `bash scripts/with-zig.sh cargo bench --bench pipeline -- --sample-size 10`

Not done

The milestone is not complete. The next highest-value gaps are stronger GTK cursor visual assertions, direct per-cell/incremental drawing, shell cursor placement strategy, advanced grapheme/IME tests, workspace/pane model, command blocks, smooth scrolling, and full GTK paint-path perf gates.
