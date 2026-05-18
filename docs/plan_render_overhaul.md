Render overhaul plan

Goal

- Keep one PTY/session and one `libghostty-vt` terminal state.
- Render all visible terminal state from Chelotype's typed `TerminalContent` model.
- Do not use hidden terminals, duplicated input fields, VTE HTML scraping, or bridge synchronization.

Current architecture

1. `src/backend.rs` owns the PTY, writes UI input to the PTY, reads PTY bytes on a background thread, and feeds those bytes into one `libghostty_vt::Terminal` on the owning thread.
2. `src/ghostty_snapshot.rs` turns Ghostty render-state rows/cells/cursor/colors/mouse modes into `TerminalContent`.
3. `src/render.rs` converts `TerminalContent` into typed render lines/runs plus GTK markup.
4. `src/canvas.rs` draws the active frame on a GTK `DrawingArea`.
5. `src/diagnostics.rs` drives headless event scenarios against the same backend/input/interaction/render path.

Next implementation order

1. Replace Pango-line markup drawing with direct per-run or per-cell drawing batches.
2. Add GTK window event e2e for launch, typing, Enter, arrows, resize, clicks, drag selection, and warnings/errors.
3. Add clipboard-backed selection export.
4. Add shell cursor placement strategy for mouse-driven command editing.
5. Add smooth scrolling and command-block metadata.
6. Add workspace/tab/split session model where each pane owns exactly one PTY and one `libghostty-vt` terminal.
7. Add GTK paint-path perf gates and allocation tracking.
