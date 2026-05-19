Benchmark suite overview

- Full pipeline: `benches/pipeline.rs`
  - `full_pipeline_markup_keyrepeat`: 128 input frames through `libghostty-vt` VT parsing, render-state snapshot extraction, and compact markup rendering.
  - `full_pipeline_frame_keyrepeat`: 128 input frames through `libghostty-vt`, `TerminalContent`, structured render runs, and app-frame rendering.
  - `full_pipeline_frame_latency_guard`: per-frame guard for the active app-frame renderer; fails if a single parser/snapshot/render frame exceeds 4 ms.
  - `frame_budget_60hz_gate`: parser/snapshot/render frame must fit the 16.67 ms 60 Hz budget.
  - `frame_budget_120hz_gate`: parser/snapshot/render frame must fit the 8.33 ms 120 Hz budget.
  - `held_key_10s_latency_gate`: sustained held-key latency histogram; defaults to 1s per Criterion sample for local speed, set `CHELOTYPE_HELD_KEY_SECONDS=10` for the full scenario.
- GTK runtime path: the app asks the backend for dirty snapshots and queues canvas draws only when terminal, resize/scroll, or selection state changed.
- GTK paint path: `CHELOTYPE_PERF_TRACE=/path/to/perf.tsv` records `input_to_render`, `input_allocs_to_render`, `input_alloc_bytes_to_render`, `gtk_render`, `gtk_paint`, per-render allocations, and `process_rss_kib` from the real app. `gtk_e2e_tracks_held_key_render_and_paint_latency_under_xvfb` holds `a` for 10 seconds under Xvfb, verifies sustained typed input, and gates p95/p99 input-to-render, per-input allocation-to-render, render, paint, per-render allocation, and process-memory behavior.

Perf signals to track

- Parser/render-state/snapshot/render should stay below the 4 ms single-frame guard.
- Parser/render-state/snapshot/render must stay within explicit 60 Hz and 120 Hz frame budgets.
- Held-key p95 must stay below 4 ms, p99 below 8 ms, and worst frame below 16 ms.
- The GTK held-key e2e gate keeps `input_to_render` p95 under 16 ms and p99 under 33 ms, keeps `gtk_render` p95 under 8 ms and p99 under 16 ms, and keeps `gtk_paint` p95 under 16 ms and p99 under 33 ms.
- The same GTK held-key e2e keeps per-input allocation-to-render and per-render allocations bounded, and fails if process RSS grows by more than 96 MiB during the 10-second key hold.
- These benches measure the active Ghostty core path, and the GTK e2e perf trace now measures the visible key-event-to-render and Cairo/Pango paint paths under held-key input. The runtime no longer repaints identical frames on every 16 ms tick and the canvas draws each terminal cell at its grid column instead of trusting full-run text layout.

Next additions

- Allocation tracking per keystroke, not only per rendered frame.
- Optional 60/120 Hz coalescing path toggled by env to compare repaint throttling.

Run

```sh
bash scripts/with-zig.sh cargo bench --bench pipeline -- --sample-size 10
CHELOTYPE_HELD_KEY_SECONDS=10 bash scripts/with-zig.sh cargo bench --bench pipeline -- held_key_10s_latency_gate --sample-size 10
```
