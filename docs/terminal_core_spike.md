Terminal core spike

Question

Should Chelotype's terminal core be based on `libghostty-vt` or `alacritty_terminal`?

Decision constraints

- The core must support one PTY and one terminal state as the only source of truth.
- The core must expose enough grid/cell state for a custom renderer.
- The core must preserve shell-driven prompt colors, syntax highlighting, cursor state, scrollback, resize/reflow, Unicode/graphemes, and terminal mouse modes.
- The integration must be testable headlessly and viable for container/Flatpak-style packaging.
- The API and build system must be stable enough for daily development.

Spike scenarios

For each candidate, implement or prototype the same scenarios:

- Spawn a shell through PTY and feed output into the terminal core.
- Type `echo hello`, read echoed input through core state, press Enter, and verify output.
- Render zsh prompt cells with fg/bg colors and style flags preserved.
- Move cursor with arrows and verify cursor column after each key.
- Resize the terminal and verify PTY size plus core reflow behavior.
- Feed a Unicode/grapheme sample and verify cell width and cursor movement.
- Feed mouse reporting escape sequences or enable a mouse-aware app and verify event support.
- Simulate a 10-second held-key stream and record p95/p99 input-to-render latency.
- Export deterministic JSON snapshots of cells, cursor, scrollback, and viewport.

`alacritty_terminal` pass criteria

- Current Rust integration remains simple and dependency-contained.
- Renderable content exposes all required cell/cursor/style data without private API hacks.
- Resize and scrollback can be wired cleanly.
- Unicode/grapheme and mouse support are good enough or can be extended without forking.
- Benchmarks show stable p95/p99 latency under sustained input.

`libghostty-vt` pass criteria

- The library can be built reproducibly in the local dev environment and packaging target.
- Rust can call it through a small, maintainable wrapper without spreading unsafe code through the app.
- The API exposes grid/cell/cursor/scrollback/dirty-region state needed by the renderer.
- Keyboard and mouse protocol coverage is materially better than the current `alacritty_terminal` path.
- The API instability risk is acceptable compared with the correctness and feature gains.

Decision output

- Create `docs/adr_terminal_core.md`.
- Record chosen core, rejected alternative, evidence from spike scenarios, known risks, and migration plan.
- Remove or isolate code that belongs to the rejected product path.
