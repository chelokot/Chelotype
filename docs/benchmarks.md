Benchmark suite overview

- Core shadow/input pipeline
  - sync_from_terminal_large_markup: VT output → Pango markup path.
  - insert_and_sync_shadow: insert + shadow sync.
  - cursor_moves_shadow: cursor escap es + sync.
  - sustained_key_repeat_shadow: continuous input throughput.
  - sustained_key_hold_latency: long-hold worst-case latency over a 1s window (sample_size=10, measurement_time=5s).
- Render-path probes
  - label_render_markup_layout: GTK label markup + layout on long line.
  - overlay_render_cycle: ghost + caret overlay render.
  - overlay_frame_budget_60hz: frame-budget check with panic on missed 16.6 ms slots.
  - jitter_latency_p99_overlay: 1s jitter run; panics if p95 > 5 ms or p99 > 8 ms (sample_size=10, measurement_time=3s).
- VTE probes
  - vte_line_extract_html: text_range_format HTML read.
  - vte_repaint_cycle_html: feed + HTML extract to simulate redraw pressure.
- Full pipeline (PTY + parser + renderer)
  - full_pipeline_markup_keyrepeat: 128 keystroke frames through alacritty parser and compact markup renderer.
  - full_pipeline_frame_keyrepeat: 128 keystroke frames through alacritty parser and structured app-frame renderer.
  - full_pipeline_frame_latency_guard: per-frame guard for the active app-frame renderer; fails if a single frame exceeds 1.2 ms.
  - held_key_10s_latency_gate: sustained held-key latency histogram; defaults to 1s per Criterion sample for local speed, set `CHELOTYPE_HELD_KEY_SECONDS=10` for the full scenario.

Perf signals to track
- Overlay/layout path dominates: label render ~0.85–0.9 ms/op, overlay cycle ~0.44–0.46 ms/op.
- Core bridge ops are sub-0.4 ms; sustained repeat ~0.33 ms/op.
- Budget/jitter benches will fail if render stalls exceed thresholds.

Next additions
- Frame-time histogram dump (p95/p99) for VTE + overlay combined.
- Allocation tracking per keystroke and per frame (to detect churn).
- Optional 60/120 Hz coalescing path toggled by env to compare repaint throttling.

Latest local baseline
- `CARGO_BUILD_JOBS=1 cargo bench --bench pipeline -- --sample-size 10`
- Before renderer grouping: `full_pipeline_keyrepeat` was ~85 ms for 128 parser+render frames and `full_pipeline_latency_guard` failed at ~1.14 ms worst-frame latency.
- After adding structured render runs, a naive multi-pass frame build regressed to ~24 ms for 128 frames. The single-pass frame builder recovered and improved it: compact markup render is ~19.5 ms, structured app-frame render is ~19.6 ms, and `full_pipeline_frame_latency_guard` passes at ~163-167 us.
- Full `CHELOTYPE_HELD_KEY_SECONDS=10` held-key Criterion run passes, but takes about 100s because Criterion collects 10 samples. Use default 1s samples locally and full 10s samples for slower perf validation.
- Default local held-key gate passes with 1s samples.
- This is useful as a parser+render regression target, but it still does not measure the full GTK frame path visible to the user.
