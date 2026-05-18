libghostty-vt spike findings

Scope

This spike checks whether `libghostty-vt` is a realistic terminal-core candidate for Chelotype before wiring it into the main Rust build.

Sources checked

- Ghostty About docs: `libghostty` is the shared core used by the macOS and Linux Ghostty GUIs, but the standalone API is not stable yet.
- Ghostty GitHub README: `libghostty` is a C/Zig library for terminal emulators; `libghostty-vt` is usable today, but API signatures are still in flux.
- Ghostling README: `libghostty-vt` handles VT parsing, terminal state, cursor position, styles, reflow, scrollback, render-state management, Unicode/graphemes, keyboard/mouse protocols, Kitty keyboard, Kitty graphics, and mouse reporting formats.
- Ghostling source: the C API exposes terminal creation, VT writes, resize, render-state update, row/cell iteration, colors, scrollbar data, mouse tracking, key events, mouse events, and effect callbacks.

Local checks

- Cloned Ghostling into `/tmp/chelotype-ghostling-spike`.
- Ghostling commit: `32c2dd5cd691c6bf226535be5ff7ec099ddd3360`.
- Ghostling pins Ghostty commit `fdb6e3d2c8543e2e756b7e07f44372efbc0fba4b`.
- Local tools present: `cmake`, `ninja`, C compiler.
- Local blocker: `zig` is not installed, and Ghostling/Ghostty CMake fails at `find_program(ZIG_EXECUTABLE)`.

Build result

Command:

```sh
cmake -S /tmp/chelotype-ghostling-spike -B /tmp/chelotype-ghostling-spike/build -G Ninja -DCMAKE_BUILD_TYPE=Release
```

Result:

```text
CMake Error at build/_deps/ghostty-src/CMakeLists.txt:68 (find_program):
  Could not find ZIG_EXECUTABLE using the following names: zig
```

Observed C API shape from Ghostling

- PTY output enters core with `ghostty_terminal_vt_write`.
- Terminal is created with `ghostty_terminal_new`.
- Terminal resize uses `ghostty_terminal_resize`.
- Render state is created with `ghostty_render_state_new`.
- Render state updates from terminal state with `ghostty_render_state_update`.
- Rendering iterates rows/cells through `GhosttyRenderStateRowIterator` and `GhosttyRenderStateRowCells`.
- Colors are accessed through `ghostty_render_state_colors_get`.
- Mouse tracking is exposed through `ghostty_terminal_get(... GHOSTTY_TERMINAL_DATA_MOUSE_TRACKING ...)`.
- Input events can be encoded through Ghostty key/mouse event APIs rather than manually emitting terminal byte sequences.

Fit for Chelotype

Strong fit:

- Single terminal state feeding a custom renderer.
- Renderer-owned UI with terminal-grid/cell iteration.
- Scrollback, reflow, styles, cursor state, Unicode/graphemes, keyboard and mouse protocols.
- Matches Chelotype's desired architecture better than VTE widget scraping.

Risks:

- Requires Zig 0.15.x in the build environment.
- C API signatures are still in flux.
- Rust integration requires a small FFI crate or generated bindings.
- Packaging needs a clear plan for building or vendoring `ghostty-vt`.
- The repo previously had a working parser baseline, so the switch had to be justified by measured correctness/features.

Decision status

Switched.

The active Chelotype backend now uses the safe Rust `libghostty-vt` bindings. Zig 0.15.2 is provided locally through `scripts/with-zig.sh`, and the product tree no longer depends on the old VTE bridge or the previous parser baseline.

Pass criteria met

- `libghostty-vt` builds locally through the Rust binding.
- The active backend creates a terminal, feeds PTY bytes, resizes, updates render state, and dumps cells/cursor/colors.
- The probe became the product path in `src/backend.rs` and `src/ghostty_snapshot.rs`.
- The backend reproduces Chelotype's integration tests: text echo, ANSI colors, cursor state, resize, scrollback, and mouse mode state.
