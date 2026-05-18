use chelotype::ghostty_snapshot::GhosttySnapshotter;
use chelotype::render::Renderer;
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use libghostty_vt::{Terminal, TerminalOptions};
use std::time::{Duration, Instant};

fn build_core() -> (Terminal<'static, 'static>, GhosttySnapshotter) {
    let mut terminal = Terminal::new(TerminalOptions {
        cols: 120,
        rows: 36,
        max_scrollback: 10000,
    })
    .expect("create ghostty terminal");
    terminal.vt_write(b"echo ready\n");
    (
        terminal,
        GhosttySnapshotter::new().expect("create ghostty snapshotter"),
    )
}

fn bench_full_pipeline_keyrepeat(c: &mut Criterion) {
    let mut group = c.benchmark_group("full_pipeline");
    group.sample_size(30);
    group.measurement_time(Duration::from_secs(2));
    group.bench_function("full_pipeline_markup_keyrepeat", |b| {
        b.iter_batched(
            build_core,
            |(mut terminal, mut snapshotter)| {
                for _ in 0..128 {
                    terminal.vt_write(b"a");
                    snapshotter.invalidate();
                    let snapshot = snapshotter.snapshot(&terminal).expect("snapshot terminal");
                    let _ = Renderer::render(snapshot);
                }
            },
            BatchSize::SmallInput,
        );
    });
    group.bench_function("full_pipeline_frame_keyrepeat", |b| {
        b.iter_batched(
            build_core,
            |(mut terminal, mut snapshotter)| {
                for _ in 0..128 {
                    terminal.vt_write(b"a");
                    snapshotter.invalidate();
                    let snapshot = snapshotter.snapshot(&terminal).expect("snapshot terminal");
                    let _ = Renderer::render_frame_with_selection(snapshot, None);
                }
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

fn bench_full_pipeline_latency_guard(c: &mut Criterion) {
    c.bench_function("full_pipeline_frame_latency_guard", |b| {
        b.iter_custom(|iters| {
            let (mut terminal, mut snapshotter) = build_core();
            let mut worst = Duration::ZERO;
            let start = Instant::now();
            for _ in 0..iters {
                let frame_start = Instant::now();
                terminal.vt_write(b"x");
                snapshotter.invalidate();
                let snapshot = snapshotter.snapshot(&terminal).expect("snapshot terminal");
                let _ = Renderer::render_frame_with_selection(snapshot, None);
                let elapsed = frame_start.elapsed();
                if elapsed > worst {
                    worst = elapsed;
                }
            }
            if worst > Duration::from_millis(4) {
                panic!("frame render latency too high: {worst:?}");
            }
            start.elapsed()
        });
    });
}

fn bench_held_key_10s_latency_gate(c: &mut Criterion) {
    let mut group = c.benchmark_group("held_key");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(3));
    group.bench_function("held_key_10s_latency_gate", |b| {
        b.iter_custom(|_| {
            let (mut terminal, mut snapshotter) = build_core();
            terminal.vt_write(b"held-key-start\n");
            let scenario_seconds = std::env::var("CHELOTYPE_HELD_KEY_SECONDS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1);
            let deadline = Instant::now() + Duration::from_secs(scenario_seconds);
            let start = Instant::now();
            let mut samples = Vec::with_capacity(60_000);
            while Instant::now() < deadline {
                let frame_start = Instant::now();
                terminal.vt_write(b"a");
                snapshotter.invalidate();
                let snapshot = snapshotter.snapshot(&terminal).expect("snapshot terminal");
                let _ = Renderer::render_frame_with_selection(snapshot, None);
                samples.push(frame_start.elapsed());
            }
            samples.sort_unstable();
            let p95 = percentile(&samples, 95);
            let p99 = percentile(&samples, 99);
            let worst = samples.last().copied().unwrap_or_default();
            if p95 > Duration::from_millis(4)
                || p99 > Duration::from_millis(8)
                || worst > Duration::from_millis(16)
            {
                panic!("held-key latency too high: p95={p95:?}, p99={p99:?}, worst={worst:?}");
            }
            start.elapsed()
        });
    });
    group.finish();
}

fn percentile(samples: &[Duration], percentile: usize) -> Duration {
    if samples.is_empty() {
        return Duration::ZERO;
    }
    let idx = ((samples.len() - 1) * percentile) / 100;
    samples[idx]
}

criterion_group!(
    pipeline,
    bench_full_pipeline_keyrepeat,
    bench_full_pipeline_latency_guard,
    bench_held_key_10s_latency_gate
);
criterion_main!(pipeline);
