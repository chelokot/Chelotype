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

1. Add richer non-Latin IM/preedit layout and styling coverage.
2. Add richer command-block interactions on top of OSC 133 metadata.
3. Strengthen selection preservation across resize/reflow and smooth-scroll states.
4. Add deeper allocation/source attribution beyond per-input and per-render counters.
5. Keep replacing broad full-frame work with smaller dirty-region render and paint paths.
