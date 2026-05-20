Benchmark suite overview

- Full pipeline: `benches/pipeline.rs`
  - `full_pipeline_markup_keyrepeat`: 128 input frames through `libghostty-vt` VT parsing, render-state snapshot extraction, and compact markup rendering.
  - `full_pipeline_frame_keyrepeat`: 128 input frames through `libghostty-vt`, `TerminalContent`, structured render runs, and app-frame rendering.
  - `full_pipeline_frame_latency_guard`: per-frame guard for the active app-frame renderer; fails if a single parser/snapshot/render frame exceeds 4 ms.
  - `frame_budget_60hz_gate`: parser/snapshot/render frame must fit the 16.67 ms 60 Hz budget.
  - `frame_budget_120hz_gate`: parser/snapshot/render frame must fit the 8.33 ms 120 Hz budget.
  - `frame_budget_240hz_gate`: parser/snapshot/render frame must fit the 4.166 ms 240 Hz budget.
  - `held_key_10s_latency_gate`: sustained held-key latency histogram; defaults to 1s per Criterion sample for local speed, set `CHELOTYPE_HELD_KEY_SECONDS=10` for the full scenario.
- GTK runtime path: the app asks the backend for dirty snapshots and queues canvas draws only when terminal, resize/scroll, or selection state changed.
- GTK paint path: `CHELOTYPE_PERF_TRACE=/path/to/perf.tsv` records `input_to_render`, `input_allocs_to_render`, `input_alloc_bytes_to_render`, `gtk_render`, `gtk_paint_interval`, `gtk_paint`, `gtk_paint_rows`, Pango layout cache hit/miss counters, row-surface cache hit/miss counters, per-render allocations, and `process_rss_kib` from the real app. `gtk_e2e_tracks_held_key_render_and_paint_latency_under_xvfb` holds `a` for 10 seconds under Xvfb, verifies sustained typed input, and gates p95/p99 input-to-render, per-input allocation-to-render, render, paint, per-render allocation, and process-memory behavior.
- GTK smooth scroll path: `gtk_e2e_scroll_wheel_drains_through_smooth_steps_under_xvfb` records scroll frame deltas, remaining pixels, visual offsets, frame durations, GTK frame intervals, render samples, and paint samples. It verifies frame-clock-driven pixel-only frames, monotonic scroll convergence, sparse full-frame renders, and paint cadence under Xvfb.
- Nested profile: `xvfb-run -a scripts/profile-240hz.sh --scenario held-key|scroll|scroll-burst|scroll-sustain|idle|frame-baseline|timer-baseline` runs the real app in a nested X server, drives x11 scenarios with `xdotool`, and summarizes `gtk_frame_interval`, real wall-clock tick cadence, real paint cadence, effective p50 FPS, 240 Hz frame buckets, `gdk_refresh_interval`, `gdk_monitor_refresh_millihz`, GTK tick-work breakdown, paint/render samples, allocation counters, and scroll-frame timing. `scroll` spaces wheel events out; `scroll-burst` sends them back-to-back to verify accumulated-target acceleration and summarizes perf samples from the app-side burst marker onward so shell seeding does not pollute the burst window. `scroll-sustain` repeats the same burst to exercise longer smooth-scroll row-surface cache pressure. `frame-baseline` keeps a nearly empty animation loop active and is the compositor/GDK baseline; `timer-baseline` adds a 240 Hz GLib timeout so main-loop timer cadence can be compared against frame callbacks. Add `--release` for release-mode profiling and `--strict` to fail when p50 frame interval, real paint interval, scroll-frame interval, p50 paint time, or p95 CPU work exceeds the 4.166 ms 240 Hz budget. Direct xdotool live-desktop profiling is guarded and requires `--allow-live`.
- Native Wayland profile: `scripts/profile-240hz.sh --display-backend native-wayland --scenario scroll-burst --release --strict` opens the app on the current Wayland session and uses app-side scroll automation only. It does not move the pointer, focus windows, send keys, or use `xdotool`, so it is the preferred live-compositor gate for proving real 240 Hz frame cadence.

Perf signals to track

- Parser/render-state/snapshot/render should stay below the 4 ms single-frame guard.
- Parser/render-state/snapshot/render must stay within explicit 60 Hz, 120 Hz, and 240 Hz frame budgets.
- Held-key p95 must stay below 4 ms, p99 below 8 ms, and worst frame below 16 ms.
- The GTK held-key e2e gate keeps `input_to_render` p95 under 16 ms and p99 under 33 ms, keeps `gtk_render` p95 under 8 ms and p99 under 16 ms, and keeps `gtk_paint` p95 under 16 ms and p99 under 33 ms.
- The GTK smooth-scroll e2e keeps scroll frame and paint p50 under the Xvfb frame-clock budget, and fails if pixel-only scroll frames trigger full terminal renders. `scripts/profile-240hz.sh --strict` also gates scroll-frame p50 and p95 CPU work for paint, render, snapshot, tick-work, and smooth-scroll integration. Use `scripts/profile-240hz.sh --display-backend native-wayland --scenario scroll-burst --release --strict` for a live-compositor 240 Hz gate without desktop input automation.
- `scripts/profile-240hz.sh --display-backend weston-headless --scenario frame-baseline --release` is a diagnostic lower bound for the nested backend, not a product 240 Hz proof. On the current machine it reports 240 Hz monitor refresh but about 195 Hz GTK tick and real paint cadence with `gtk_tick_work` around 9 us, so missed 240 Hz cadence there is compositor/GDK pacing rather than terminal render cost. `timer-baseline` in the same backend shows the GLib timeout can tick around the 240 Hz budget (`glib_timeout_interval` p50 about 4.1 ms) while GDK frame callbacks and real paint intervals remain around 195 Hz, narrowing the nested-backend blocker to GDK/Wayland presentation scheduling rather than main-loop timer precision.
- Weston-headless renderer and repaint-window checks: `CHELOTYPE_WESTON_RENDERER=pixman` is slower than the default GL renderer, and `CHELOTYPE_WESTON_REPAINT_WINDOW=0` or `1` drops frame callbacks to about 120-140 Hz. Repaint windows at `4` ms and above stay in the same about-195 Hz plateau, so the nested-backend miss is not fixed by a smaller repaint window.
- GTK scene renderer checks use `CHELOTYPE_GSK_RENDERER=gl|ngl|vulkan|cairo`; the selected renderer is printed in the profile header so frame-cadence results can be compared without editing the script. On GTK 4.20 in the nested Weston baseline, `gl`, `ngl`, and `vulkan` stay in the same about-196 Hz plateau. `cairo` can lower `gtk_frame_interval` p50 but worsens wall-clock cadence and frame buckets, so it is not a stable 240 Hz fix.
- The same GTK held-key e2e keeps per-input allocation-to-render and per-render allocations bounded, and fails if process RSS grows by more than 96 MiB during the 10-second key hold.
- These benches measure the active Ghostty core path, and the GTK e2e perf trace now measures the visible key-event-to-render and Cairo/Pango paint paths under held-key input. The runtime no longer repaints identical frames on every 16 ms tick, the canvas draws each terminal cell at its grid column instead of trusting full-run text layout, and pixel-only scroll paints reuse canvas-local Pango layouts plus cached row surfaces for unchanged render frames.

Next additions

- Allocation tracking per keystroke, not only per rendered frame.
- Optional 60/120 Hz coalescing path toggled by env to compare repaint throttling.

Run

```sh
bash scripts/with-zig.sh cargo bench --bench pipeline -- --sample-size 10
CHELOTYPE_HELD_KEY_SECONDS=10 bash scripts/with-zig.sh cargo bench --bench pipeline -- held_key_10s_latency_gate --sample-size 10
xvfb-run -a scripts/profile-240hz.sh --scenario held-key --duration 10 --release
xvfb-run -a scripts/profile-240hz.sh --scenario scroll --duration 6 --release
xvfb-run -a scripts/profile-240hz.sh --scenario scroll-burst --duration 1 --release
scripts/profile-240hz.sh --display-backend weston-headless --scenario scroll-sustain --duration 1 --release
scripts/profile-240hz.sh --display-backend weston-headless --scenario frame-baseline --duration 1 --release
scripts/profile-240hz.sh --display-backend weston-headless --scenario timer-baseline --duration 1 --release
scripts/profile-240hz.sh --display-backend native-wayland --scenario scroll-burst --duration 1 --release --strict
```
