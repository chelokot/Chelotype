Terminal core spike

Question

Should Chelotype's terminal core be based on `libghostty-vt` or `alacritty_terminal`?

Decision

Use `libghostty-vt`.

Why

- It exposes a terminal state plus render-state API intended for custom renderers.
- It covers scrollback, reflow, cursor state, styles, Unicode/graphemes, keyboard protocols, and mouse reporting in the core.
- Safe Rust bindings are available through `libghostty-vt`.
- The active Chelotype backend now passes the existing PTY, color, cursor, resize, mouse-mode, scrollback, headless interaction, Unicode/style, and pipeline benchmark gates on `libghostty-vt`.

Implementation Evidence

- `src/backend.rs`
- `src/ghostty_snapshot.rs`
- `src/terminal_grid.rs`
- `tests/backend_integration_tests.rs`
- `tests/headless_diagnostics_tests.rs`
- `benches/pipeline.rs`
- `docs/adr_terminal_core.md`

Remaining Risk

The Rust binding and C/Zig API are still young. Keep all direct `libghostty-vt` API usage contained behind Chelotype-owned modules so API churn does not spread through UI, diagnostics, or renderer code.
