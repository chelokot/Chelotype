ADR: terminal core

Status

Accepted and implemented for the active backend: `libghostty-vt`.

Context

Chelotype needs one PTY, one terminal-core state, and a custom renderer that can display history, scrollback, and current input from the same shell-driven source of truth. The previous VTE bridge approach caused hidden terminal state, duplicated input, HTML scraping, and weak testability.

Decision

Use `libghostty-vt` as the product terminal core. It is built around VT parsing, terminal state, render-state extraction, scrollback/reflow, Unicode/graphemes, keyboard/mouse protocol support, and renderer-owned UI. The old VTE bridge code and the previous parser baseline have been removed from the product tree.

Evidence

- `Cargo.toml` depends on `libghostty-vt` and no longer depends on `alacritty_terminal`, `vte4`, or bridge-only parsing helpers.
- `src/backend.rs` owns one `portable_pty` session and one `libghostty_vt::Terminal`.
- `src/ghostty_snapshot.rs` converts Ghostty render-state rows/cells into Chelotype's typed `TerminalContent`.
- `src/render.rs`, `src/snapshot.rs`, and `src/selection.rs` consume Chelotype's own terminal grid model, not backend-specific cell types.
- `tests/backend_integration_tests.rs` proves PTY write/read, ANSI colors, cursor tracking, resize, mouse modes, and scrollback on the active `libghostty-vt` backend.
- `tests/headless_diagnostics_tests.rs` proves binary-level automation, warnings/errors checks, keyboard events, mouse drag selection, resize, scroll, Unicode, wide cells, and style exports.
- `benches/pipeline.rs` measures the active `libghostty-vt` parser/render-state/snapshot/renderer path.

Risks

- `libghostty-vt` API signatures are still young, so keep the integration behind small Chelotype-owned modules.
- The vendored native build needs Zig 0.15.2. `scripts/with-zig.sh` installs a local ignored Zig toolchain before running Cargo commands.
- Ghostty VT objects are `!Send + !Sync`; Chelotype keeps terminal state on the owning thread and moves PTY bytes across a channel.

Migration Outcome

The active product path has been migrated. Remaining work is no longer "choose the core"; it is improving the renderer, GTK e2e automation, clipboard/selection, workspaces, tabs, split panes, command blocks, smooth scrolling, and stronger perf/memory gates on top of the chosen core.
