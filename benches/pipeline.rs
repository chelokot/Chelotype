use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::term::Config;
use alacritty_terminal::term::Term;
use alacritty_terminal::vte::ansi::Processor;
use chelotype::backend::{RenderableContentOwned, ScreenSize};
use chelotype::render::Renderer;
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Default)]
struct BenchListener;

impl EventListener for BenchListener {
    fn send_event(&self, _: Event) {}
}

fn build_term() -> (Term<BenchListener>, Processor) {
    (
        Term::new(Config::default(), &ScreenSize::default(), BenchListener),
        Processor::new(),
    )
}

fn bench_full_pipeline_keyrepeat(c: &mut Criterion) {
    let mut group = c.benchmark_group("full_pipeline");
    group.sample_size(30);
    group.measurement_time(Duration::from_secs(2));
    group.bench_function("full_pipeline_keyrepeat", |b| {
        b.iter_batched(
            || {
                let (mut term, mut parser) = build_term();
                parser.advance(&mut term, b"echo ready\n");
                (term, parser)
            },
            |(mut term, mut parser)| {
                for _ in 0..128 {
                    parser.advance(&mut term, b"a");
                    let snapshot =
                        RenderableContentOwned::from_renderable(term.renderable_content());
                    let _ = Renderer::render(snapshot);
                }
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

fn bench_full_pipeline_latency_guard(c: &mut Criterion) {
    c.bench_function("full_pipeline_latency_guard", |b| {
        b.iter_custom(|iters| {
            let (mut term, mut parser) = build_term();
            parser.advance(&mut term, b"echo latency\n");
            let mut worst = Duration::ZERO;
            let start = Instant::now();
            for _ in 0..iters {
                let t0 = Instant::now();
                parser.advance(&mut term, b"x");
                let snapshot = RenderableContentOwned::from_renderable(term.renderable_content());
                let _ = Renderer::render(snapshot);
                let dt = t0.elapsed();
                if dt > worst {
                    worst = dt;
                }
            }
            if worst > Duration::from_micros(900) {
                panic!("render latency too high: {:?}", worst);
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
            let (mut term, mut parser) = build_term();
            parser.advance(&mut term, b"held-key-start\n");
            let scenario_seconds = std::env::var("CHELOTYPE_HELD_KEY_SECONDS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1);
            let deadline = Instant::now() + Duration::from_secs(scenario_seconds);
            let start = Instant::now();
            let mut samples = Vec::with_capacity(60_000);
            while Instant::now() < deadline {
                let frame_start = Instant::now();
                parser.advance(&mut term, b"a");
                let snapshot = RenderableContentOwned::from_renderable(term.renderable_content());
                let _ = Renderer::render(snapshot);
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
