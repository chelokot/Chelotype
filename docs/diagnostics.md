Headless snapshots
- Run with `CHELOTYPE_HEADLESS=1` to spawn the shell, feed a short scripted session, and dump snapshots.
- Snapshot files go to `/tmp/chelotype_snapshots` by default or `CHELOTYPE_SNAPSHOT_DIR`.
- Each run produces `.json`, `.html`, `.txt`, and `markup.html` captures with colors, cursor, and rendered markup.
- Selection runs also produce `.render.json` with history/input markup, selected text, and the selected grid range.
- JSON snapshots include `rows`, `cols`, cursor fields, mouse mode fields, plain `text`, and per-cell text/color/style records, including zero-width combining marks and wide-cell spacer metadata.
- Headless mode runs a clean interactive `zsh -f` fixture with an empty prompt so shell line editing, arrow keys, and mouse-click cursor movement are exercised against the same kind of editable shell input path used by the product target.
- Override actions with `CHELOTYPE_HEADLESS_ACTIONS`; split actions with `|`, and use `\n`, `\r`, `\t`, `\e`, and `\\` escapes.
- Prefer `CHELOTYPE_HEADLESS_EVENTS` for input-path e2e. Supported events: `raw:<bytes>`, `text:<text>`, `key:<name>`, `resize:<cols>x<rows>`, `scroll:<lines>`, `mouse:press:<button>:<col>,<row>`, `mouse:drag:<col>,<row>`, and `mouse:release:<col>,<row>`.
- Override expected terminal text with `CHELOTYPE_HEADLESS_EXPECT`; split expected substrings with `|`.
- Override delay between actions with `CHELOTYPE_HEADLESS_STEP_MS`.
- Override selected cells with `CHELOTYPE_HEADLESS_SELECTION=row,column:row,column`.
- This runner is a binary-level snapshot/automation gate. It is not a replacement for GTK event e2e; cooked shell line-editing sequences such as Backspace and arrows should be verified through input mapping tests plus GTK interaction tests.

Live snapshots
- Set `CHELOTYPE_SNAPSHOT=1` while running the UI; one snapshot per second is written to the snapshot directory.
- Use `CHELOTYPE_SNAPSHOT_DIR` to override the dump location.
- Product runs prefer `fish` when it is installed. Set `CHELOTYPE_SHELL=/path/to/shell` when a test or manual run must force zsh/bash.
- Set `CHELOTYPE_RENDER_SNAPSHOT=1` with live snapshots to also emit `.render.json`/`.render.html` files for the active rendered frame. These include transient UI overlays such as IM preedit text at the terminal cursor, which are not part of the PTY-backed terminal grid until committed.
- Set `CHELOTYPE_GEOMETRY_TRACE=/path/to/file.env` to write canvas geometry, grid size, cell width, and line height for real-window automation.
- Set `CHELOTYPE_SCROLL_TRACE=/path/to/file.tsv` to write GTK wheel-scroll enqueue/frame/idle/limit events for smooth-scroll automation.
- Set `CHELOTYPE_CLIPBOARD_TRACE=/path/to/file.tsv` to write PRIMARY/CLIPBOARD selection exports.
- Set `CHELOTYPE_PERF_TRACE=/path/to/file.tsv` to write `input_to_render`, per-input allocation-to-render, GTK render/paint, real paint cadence, row-surface cache counters, per-render allocation, and RSS samples.
- Run `scripts/profile-240hz.sh --scenario held-key|scroll|scroll-burst|scroll-sustain|idle|frame-baseline|timer-baseline` to collect and summarize those perf samples from a nested run. `scroll-burst` sends wheel events without spacing so accumulated target distance must accelerate; `scroll-sustain` repeats that app-side burst to profile longer smooth-scroll cache pressure. `frame-baseline` keeps a minimal animation loop active and isolates compositor/GDK frame-callback cadence from terminal snapshot/render work; `timer-baseline` compares a 240 Hz GLib timeout with GDK frame-callback cadence. Use `--strict` when you want the command to fail if p50 frame interval, real paint interval, scroll-frame interval, p50 paint time, or p95 CPU work misses the 4.166 ms 240 Hz budget.
- The `frame_buckets` line groups `gtk_frame_interval` samples by 240 Hz budget multiples (`<=1x`, `<=2x`, `<=3x`, `>3x`) to distinguish a steady below-target cadence from sporadic skipped presentations.
- For Weston-headless diagnostics, use `CHELOTYPE_WESTON_RENDERER=gl|pixman` and `CHELOTYPE_WESTON_REPAINT_WINDOW=<ms>` to compare compositor settings. The script prints both values in the scenario header.
- Use `CHELOTYPE_GSK_RENDERER=gl|ngl|vulkan|cairo` to compare GTK scene renderers; the selected GSK renderer is printed in the profile header.
- Run `scripts/record-wheel-profile.sh`, scroll naturally over the opened `wev` window, then press Ctrl+C to save a real wheel timing profile to `~/.config/chelotype/wheel-profile.tsv`. The Appearance settings preview replays this profile through the same smooth-scroll integrator used by the terminal.

Usage tips
- Inspect `markup.html` for GTK markup fidelity and color mismatches.
- Compare `.txt` with `.html` to spot alignment or spacing drift.
- When debugging input lag, pair snapshots with benches in `benches/` to reproduce render cost without the UI.
