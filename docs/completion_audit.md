Completion audit

Objective summary

Chelotype needs one real PTY-backed terminal session, one terminal-core state, shell-driven rendering, first-class keyboard/mouse/automation paths, a deliberate terminal-core choice, custom rendering from grid/cell state, deterministic headless testing, container-friendly assumptions, and a path to modern terminal workspaces.

Prompt-to-artifact checklist

- One real PTY per terminal session
  - Evidence: `src/backend.rs` owns one `portable_pty` master/child per `TerminalBackend`; `src/workspace.rs` stores tabs that each own one or more panes, each pane owns its own `TerminalBackend`, and active-tab panes can be snapshotted together for split rendering; the GTK app talks to the active workspace pane.
  - Coverage: `backend_writes_to_single_pty_and_reads_shell_output`, `workspace_tracks_active_tab_identity`, `workspace_keeps_each_tab_terminal_state_isolated`, `workspace_splits_active_tab_into_isolated_panes`, `workspace_snapshots_all_panes_in_active_tab`, and `gtk_e2e_creates_and_switches_terminal_tabs_under_xvfb`.
  - Status: partial; workspace tabs, visible tabs, and real split-pane ownership inside a tab exist, but split panes are not rendered as simultaneous GTK panes yet.
- One terminal-core state / shell-driven source of truth
  - Evidence: `src/backend.rs` owns one `libghostty_vt::Terminal`; PTY reader bytes are fed into that terminal state.
  - Coverage: backend integration tests and headless snapshots.
  - Status: partial; active app rebuilds render frames only when the active terminal or selection changes, but dirty-row incremental rendering is not implemented yet.
- Core choice
  - Evidence: `docs/adr_terminal_core.md`.
  - Status: done for this stage; `libghostty-vt` is selected and implemented in the active backend.
- No hidden terminals / no bridge synchronization / no fake local command fields
  - Evidence: `src/bridge.rs`, `tests/bridge_tests.rs`, and the bridge bench were removed; `Cargo.toml` has no VTE bridge feature.
  - Status: done for the active tree.
- Renderer consumes grid/cell state directly
  - Evidence: `src/ghostty_snapshot.rs` exports `TerminalContent`; `src/render.rs` renders typed cells/runs from that model; `src/canvas.rs` paints structured runs by grid column and draws the cursor from grid coordinates.
  - Status: partial; canvas paints individual terminal cells at fixed grid columns and has nonblank screenshot, pixel-level truecolor e2e, and pixel-level cursor position assertions, but does not yet have incremental dirty-row paint.
- History, scrollback, current input
  - Evidence: `RenderRegion::{History, Input}`, `RenderFrame.lines`, `TerminalBackend::scroll_display`, `display_offset` snapshots, Ghostty row metadata (`wrapped`, `wrap_continuation`, semantic prompt) exported through `TerminalContent`, and semantic prompt rows converted to `RenderFrame.command_blocks`.
  - Coverage: `backend_exposes_scrollback_display_offset`, `headless_mode_replays_scroll_event`, `ghostty_snapshot::tests::snapshot_exports_soft_wrap_metadata`, `render::tests::renderer_marks_soft_wrapped_cursor_line_as_single_input_region`, `render::tests::renderer_marks_semantic_prompt_continuations_as_input_region`, `renderer_exports_prompt_delimited_command_blocks`, and `headless_mode_exports_osc133_command_blocks`.
  - Status: partial; soft-wrapped and semantic prompt continuation input stays in one input region, prompt-delimited command-block metadata exists, and OSC 133 command blocks are exported in render dumps, but no smooth scroll model or command-block UI yet.
- Keyboard input
  - Evidence: `src/input.rs`.
  - Coverage: unit tests plus headless keyboard, Backspace, Enter, Ctrl-D, arrow byte e2e, `tests/gtk_e2e_tests.rs` real-window Xvfb smoke, and Xvfb+xdotool keyboard input into the actual window.
  - Status: partial; real keyboard input reaches the shell through GTK and the headless clean zsh fixture, with more IME/layout scenarios still needed.
- Mouse as first-class input
  - Evidence: `src/mouse.rs`, `src/interaction.rs`, Ghostty mouse-mode state from `src/ghostty_snapshot.rs`.
  - Coverage: local drag selection tests, drag-release stop test, SGR mouse reporting tests, headless mouse drag selection e2e, headless mouse-click cursor movement through zsh line editing, headless soft-wrapped input click-to-cursor e2e, semantic prompt continuation cursor movement unit tests, real Xvfb+xdotool GTK drag-selection e2e that verifies exported `selected_text`, real Xvfb+xdotool released-selection copy e2e, real Xvfb+xdotool click-to-cursor e2e, real Xvfb+xdotool click-to-cursor with zsh autosuggestions visible, and real Xvfb+xdotool terminal mouse-reporting e2e that verifies SGR click bytes reach the PTY.
  - Status: partial; PRIMARY/CLIPBOARD export, current-row shell cursor placement, soft-wrapped input cursor placement, and semantic prompt continuation row cursor placement are covered, but richer multi-command cursor placement is still primitive.
- Resize/reflow
  - Evidence: `TerminalBackend::resize` updates PTY winsize and Ghostty terminal dimensions.
  - Coverage: backend resize integration, headless resize event e2e, and real Xvfb+xdotool GTK window resize e2e that verifies changed snapshot rows.
  - Status: partial; scrollback reflow edge cases are weak.
- Colors/styles/zsh prompt fidelity
  - Evidence: Ghostty cell fg/bg/style snapshots and renderer exports.
  - Coverage: ANSI color backend test, headless Unicode/style JSON test, headless clean-zsh colored prompt cell test, Xvfb real-window smoke snapshot, GTK color e2e that verifies ANSI-colored output cells in JSON, pixel-level truecolor screenshot e2e, and a real zsh-autosuggestions fixture that asserts grey suggestion cells stay on exact grid columns across the styled space.
  - Status: partial; truecolor, deterministic zsh prompt color, and zsh autosuggestion styling reach snapshots/render dumps, but broader user theme/plugin fidelity is not deeply asserted.
- Cursor position/shape/visibility
  - Evidence: cursor fields in snapshots, canvas caret drawing.
  - Coverage: backend cursor integration, HTML snapshot cursor-placement unit test, canvas blink reset unit test, real Xvfb screenshot e2e that asserts the cursor-colored pixels form a narrow vertical caret at the snapshot cursor row/column, and real Xvfb screenshot e2e that verifies cursor blink-off plus visible reset after input.
  - Status: partial; visual cursor shape, position, and blink/reset are covered; cursor behavior under IME/composition is still missing.
- Unicode/graphemes
  - Evidence: Ghostty grapheme extraction, shared `src/cell_text.rs`, structured cell JSON with wide-cell flags.
  - Coverage: render/snapshot/selection unit tests preserve combining marks and skip wide-cell spacers; headless Unicode/style JSON test checks combining marks and wide-cell metadata; headless emoji/ZWJ/ambiguous-width test verifies ZWJ emoji cells and single-width ambiguous characters through snapshots and render dumps.
  - Status: partial; emoji ZWJ and ambiguous-width coverage exists, but IME/composition scenarios are still needed.
- Perf gates
  - Evidence: `benches/pipeline.rs`, `src/perf_trace.rs`, `src/process_metrics.rs`, `tests/gtk_e2e_tests.rs`, `docs/benchmarks.md`.
  - Coverage: active Ghostty app-frame render benchmark, 4 ms latency guard, explicit 60/120 Hz frame-budget gates, Criterion held-key scenario, backend dirty-snapshot regression test, canvas frame equality skip, and real Xvfb 10-second held-key e2e that records and gates `input_to_render`, `gtk_render`, and `gtk_paint` p95/p99.
  - Status: partial; active frame construction, GTK input-to-render latency, GTK render, GTK paint, per-render allocations, and 10-second held-key RSS growth are gated, but allocation attribution per keystroke is still missing.
- Container-friendly runtime assumptions
  - Evidence: `scripts/with-zig.sh`, binary-level headless mode, clean `zsh -f` diagnostics fixture, README setup/run commands, and Xvfb-based GTK e2e test.
  - Status: partial; no CI container image yet.
- Workspaces, tabs/splits, command blocks, smooth scrolling
  - Evidence: `src/workspace.rs` provides active-tab routing, multi-tab ownership, active-pane routing inside each tab, and active-tab multi-pane render snapshots; `src/workspace_render.rs` builds one structured render frame per pane with deterministic pane geometry; `src/app.rs` routes keyboard, mouse, scroll, resize, scripted input, and snapshots through the active workspace pane; the header exposes tab buttons and a new-tab button; `src/command_blocks.rs` builds semantic-prompt command blocks; `src/diagnostics.rs` exports command-block metadata and headless split-pane workspace render dumps.
  - Coverage: workspace unit tests prove tab identity switching, isolated terminal state across two top-level tabs, isolated terminal state across two real panes inside one tab, and simultaneous snapshot extraction for all panes in the active tab; workspace render unit tests prove deterministic column layout for split panes; input unit tests cover tab shortcuts; GTK e2e creates a second terminal tab, verifies active tab isolation, and switches back; `headless_mode_exports_workspace_split_render_dump` proves split-pane render-state export from two real PTYs; command-block unit tests, renderer tests, and OSC 133 headless e2e prove prompt/continuation/output row boundaries.
  - Status: partial; visible tabs and split-pane workspace ownership/headless render dumps exist, but GTK split-pane layout, smooth scroll model, and command-block UI are not implemented yet.

Current green commands

- `bash scripts/with-zig.sh cargo fmt -- --check`
- `bash scripts/with-zig.sh cargo check`
- `bash scripts/with-zig.sh cargo test`
- `bash scripts/with-zig.sh cargo bench --bench pipeline -- --sample-size 10`

Not done

The milestone is not complete. The next highest-value gaps are advanced grapheme/IME tests, split-pane UI, command-block UI, smooth scrolling, and memory/allocation perf gates.
