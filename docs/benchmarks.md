Benchmark suite overview

- Full pipeline: `benches/pipeline.rs`
  - `full_pipeline_markup_keyrepeat`: 128 input frames through `libghostty-vt` VT parsing, render-state snapshot extraction, and compact markup rendering.
  - `full_pipeline_frame_keyrepeat`: 128 input frames through `libghostty-vt`, `TerminalContent`, structured render runs, and app-frame rendering.
  - `full_pipeline_frame_latency_guard`: per-frame guard for the active app-frame renderer; fails if a single parser/snapshot/render frame exceeds 4 ms.
  - `held_key_10s_latency_gate`: sustained held-key latency histogram; defaults to 1s per Criterion sample for local speed, set `CHELOTYPE_HELD_KEY_SECONDS=10` for the full scenario.
- GTK runtime path: the app asks the backend for dirty snapshots and queues canvas draws only when terminal, resize/scroll, or selection state changed.

Perf signals to track

- Parser/render-state/snapshot/render should stay below the 4 ms single-frame guard.
- Held-key p95 must stay below 4 ms, p99 below 8 ms, and worst frame below 16 ms.
- These benches now measure the active Ghostty core path. They still do not measure the full GTK/Cairo/Pango paint path visible to the user, but the runtime no longer repaints identical frames on every 16 ms tick.

Next additions

- GTK paint-path headless benchmark with frame-time histogram and screenshot/pixel validation.
- Allocation tracking per keystroke and per frame.
- Optional 60/120 Hz coalescing path toggled by env to compare repaint throttling.

Run

```sh
bash scripts/with-zig.sh cargo bench --bench pipeline -- --sample-size 10
CHELOTYPE_HELD_KEY_SECONDS=10 bash scripts/with-zig.sh cargo bench --bench pipeline -- held_key_10s_latency_gate --sample-size 10
```
