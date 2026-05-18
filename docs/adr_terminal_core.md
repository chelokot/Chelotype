ADR: terminal core

Status

Pending buildable `libghostty-vt` probe.

Context

Chelotype needs one PTY, one terminal-core state, and a custom renderer that can display history, scrollback, and current input from the same shell-driven source of truth. The previous VTE bridge approach caused hidden terminal state, duplicated input, HTML scraping, and weak testability. The current `alacritty_terminal` prototype proves that a Rust terminal core can be embedded, but it has not yet been compared against `libghostty-vt`.

Decision

Pending.

Candidates

- `alacritty_terminal`
- `libghostty-vt`

Required evidence

- PTY spawn and write path.
- Parser/grid correctness for zsh prompt, command echo, command output, and ANSI colors.
- Cursor position, visibility, and shape.
- Scrollback and viewport access.
- Resize/reflow behavior.
- Unicode/grapheme behavior.
- Keyboard and mouse protocol support.
- Snapshot/export support for e2e tests.
- Sustained held-key latency and frame budget measurements.
- Build, packaging, and container compatibility.

Current evidence

- `alacritty_terminal` is already integrated in `src/backend.rs`.
- `alacritty_terminal` currently passes backend integration tests for PTY write/read, ANSI colors, cursor tracking, resize, and mouse mode tracking.
- `libghostty-vt` was researched through Ghostty docs, Ghostty README, Ghostling README, and Ghostling source.
- Ghostling was cloned locally and inspected at commit `32c2dd5cd691c6bf226535be5ff7ec099ddd3360`.
- Ghostling pins Ghostty commit `fdb6e3d2c8543e2e756b7e07f44372efbc0fba4b`.
- Local Ghostling CMake configure currently fails because `zig` is not installed.
- Detailed findings are recorded in `docs/libghostty_spike_findings.md`.

Decision criteria

- Prefer the candidate that gives the strongest terminal correctness while keeping the build and API maintainable.
- Reject any candidate that forces hidden terminals, duplicated shell state, HTML scraping, or untestable UI behavior.
- Reject any candidate that cannot expose enough render-state data for a custom renderer.
- Treat unstable APIs as acceptable only if the feature/correctness gain is large and isolated behind a small adapter.

Outcome

Pending. Do not switch away from `alacritty_terminal` until `libghostty-vt` builds locally and a minimal render-state probe reproduces the active backend tests.

Migration plan

- Keep the `alacritty_terminal` path as the working product path for now.
- Add a separate `libghostty-vt` probe only after Zig 0.15.x is available in the dev/build environment.
- Keep any unsafe/FFI code behind a small adapter crate or module.
- Compare both cores against the same backend integration scenarios before choosing.
