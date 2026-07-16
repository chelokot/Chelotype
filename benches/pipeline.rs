use chelotype::ghostty_snapshot::GhosttySnapshotter;
use chelotype::input::{CursorDirection, CursorUnit};
use chelotype::input_selection::{input_buffer_range_for_selection, keyboard_cursor_target};
use chelotype::interaction::cursor_movement_bytes_between_editable_input_points;
use chelotype::mouse::MouseGridPosition;
use chelotype::render::Renderer;
use chelotype::selection::{GridPoint, SelectionRange};
use chelotype::terminal_grid::{
    MouseMode, TerminalCell, TerminalColors, TerminalContent, TerminalLineMetadata,
    TerminalSemanticPrompt,
};
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

fn build_dense_large_core() -> (Terminal<'static, 'static>, GhosttySnapshotter) {
    let mut terminal = Terminal::new(TerminalOptions {
        cols: 240,
        rows: 67,
        max_scrollback: 10000,
    })
    .expect("create large ghostty terminal");
    let line = vec![b'x'; 240];
    for _ in 0..67 {
        terminal.vt_write(&line);
    }
    terminal.vt_write(b"\x1b[34;121H");
    let mut snapshotter = GhosttySnapshotter::new().expect("create ghostty snapshotter");
    snapshotter
        .snapshot(&terminal)
        .expect("initial large snapshot");
    (terminal, snapshotter)
}

fn bench_incremental_snapshot(c: &mut Criterion) {
    let mut group = c.benchmark_group("snapshot_240x67");
    group.sample_size(30);
    group.measurement_time(Duration::from_secs(2));
    group.bench_function("dirty_row", |b| {
        b.iter_batched(
            build_dense_large_core,
            |(mut terminal, mut snapshotter)| {
                terminal.vt_write(b"a");
                let snapshot = snapshotter.snapshot(&terminal).expect("partial snapshot");
                assert_eq!(snapshotter.last_converted_rows(), 1);
                snapshot
            },
            BatchSize::SmallInput,
        );
    });
    group.bench_function("forced_full", |b| {
        b.iter_batched(
            build_dense_large_core,
            |(mut terminal, mut snapshotter)| {
                terminal.vt_write(b"a");
                snapshotter.invalidate();
                let snapshot = snapshotter.snapshot(&terminal).expect("full snapshot");
                assert_eq!(snapshotter.last_converted_rows(), 67);
                snapshot
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
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
                    let snapshot = snapshotter.snapshot(&terminal).expect("snapshot terminal");
                    let _ = Renderer::render(&snapshot);
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
                    let snapshot = snapshotter.snapshot(&terminal).expect("snapshot terminal");
                    let _ = Renderer::render_frame_with_selection(&snapshot, None);
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
                let snapshot = snapshotter.snapshot(&terminal).expect("snapshot terminal");
                let _ = Renderer::render_frame_with_selection(&snapshot, None);
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

fn bench_frame_budget_gates(c: &mut Criterion) {
    let mut group = c.benchmark_group("frame_budget");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(2));
    group.bench_function("frame_budget_60hz_gate", |b| {
        b.iter_custom(|iters| run_frame_budget_gate(iters, Duration::from_micros(16_667)));
    });
    group.bench_function("frame_budget_120hz_gate", |b| {
        b.iter_custom(|iters| run_frame_budget_gate(iters, Duration::from_micros(8_333)));
    });
    group.bench_function("frame_budget_240hz_gate", |b| {
        b.iter_custom(|iters| run_frame_budget_gate(iters, Duration::from_micros(4_166)));
    });
    group.finish();
}

fn run_frame_budget_gate(iters: u64, budget: Duration) -> Duration {
    let (mut terminal, mut snapshotter) = build_core();
    let mut worst = Duration::ZERO;
    let start = Instant::now();
    for _ in 0..iters {
        let frame_start = Instant::now();
        terminal.vt_write(b"x");
        let snapshot = snapshotter.snapshot(&terminal).expect("snapshot terminal");
        let _ = Renderer::render_frame_with_selection(&snapshot, None);
        let elapsed = frame_start.elapsed();
        if elapsed > worst {
            worst = elapsed;
        }
    }
    assert!(
        worst <= budget,
        "frame budget exceeded: worst={worst:?}, budget={budget:?}"
    );
    start.elapsed()
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
                let snapshot = snapshotter.snapshot(&terminal).expect("snapshot terminal");
                let _ = Renderer::render_frame_with_selection(&snapshot, None);
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

fn bench_input_selection_long_line_drag_follow(c: &mut Criterion) {
    let mut group = c.benchmark_group("input_selection");
    group.sample_size(30);
    group.measurement_time(Duration::from_secs(2));
    group.bench_function("long_line_drag_follow", |b| {
        b.iter_batched(
            || long_input_content(16_384),
            |content| {
                for column in 256..4096 {
                    let source = MouseGridPosition { row: 0, column };
                    let target = MouseGridPosition {
                        row: 0,
                        column: column + 1,
                    };
                    let bytes = cursor_movement_bytes_between_editable_input_points(
                        &content, source, target,
                    )
                    .expect("adjacent input cursor movement");
                    assert_eq!(bytes, b"\x1b[C");
                }
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

fn bench_input_multiline_edit_range(c: &mut Criterion) {
    let mut group = c.benchmark_group("input_edit");
    group.sample_size(30);
    group.measurement_time(Duration::from_secs(2));
    group.bench_function("multiline_semantic_selection_range_10x1024", |b| {
        b.iter_batched(
            || multiline_semantic_input_content(10, 1024),
            |content| {
                let range = SelectionRange::new(
                    GridPoint {
                        row: 2,
                        column: 128,
                    },
                    GridPoint {
                        row: 5,
                        column: 900,
                    },
                );
                for _ in 0..10_000 {
                    assert_eq!(
                        input_buffer_range_for_selection(&content, range),
                        Some(2176..6023)
                    );
                }
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

fn bench_input_wrapped_word_jump(c: &mut Criterion) {
    let mut group = c.benchmark_group("input_edit");
    group.sample_size(30);
    group.measurement_time(Duration::from_secs(2));
    group.bench_function("wrapped_word_jump_8x1024", |b| {
        b.iter_batched(
            || soft_wrapped_input_content(8, 1024),
            |mut content| {
                content.cursor_line = 7;
                content.cursor_col = 1024;
                for _ in 0..10_000 {
                    assert_eq!(
                        keyboard_cursor_target(
                            &content,
                            CursorDirection::Left,
                            CursorUnit::Word,
                            None
                        ),
                        Some(MouseGridPosition { row: 0, column: 2 })
                    );
                }
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

fn bench_fish_undo_capture_coalescing(c: &mut Criterion) {
    let Some(fish_path) = ["/usr/bin/fish", "/bin/fish"]
        .into_iter()
        .find(|path| std::path::Path::new(path).is_file())
    else {
        return;
    };
    let mut group = c.benchmark_group("input_edit");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(2));
    group.bench_function("fish_undo_capture_1000_repeated_edits", |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _ in 0..iters {
                let output = std::process::Command::new(fish_path)
                    .args([
                        "--init-command",
                        chelotype::shell::FISH_CHELOTYPE_INIT_FOR_TESTS,
                        "-ic",
                        r#"
commandline --replace ''
commandline -C 0
__chelotype_capture_undo
for index in (seq 1 1000)
    commandline --insert x
    __chelotype_capture_undo
end
__chelotype_undo
test (string length -- (commandline)) -eq 0
"#,
                    ])
                    .output()
                    .expect("run fish undo capture benchmark");
                assert!(
                    output.status.success(),
                    "fish undo capture benchmark failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            start.elapsed()
        });
    });
    group.finish();
}

fn long_input_content(columns: usize) -> TerminalContent {
    TerminalContent {
        lines: vec![vec![TerminalCell::blank(); columns].into()],
        line_metadata: vec![TerminalLineMetadata::default()],
        cursor_line: 0,
        cursor_col: 256,
        cursor_visible: true,
        display_offset: 0,
        colors: TerminalColors::default(),
        mouse: MouseMode::default(),
    }
}

fn multiline_semantic_input_content(rows: usize, columns: usize) -> TerminalContent {
    let lines = (0..rows)
        .map(|row| {
            let mut line = vec![
                TerminalCell {
                    text: "a".into(),
                    ..TerminalCell::blank()
                };
                columns
            ];
            if row == 0 {
                line[0].text = "❯".into();
                line[1].text = " ".into();
            }
            line.into()
        })
        .collect();
    let mut line_metadata = vec![TerminalLineMetadata::default(); rows];
    line_metadata[0].semantic_prompt = TerminalSemanticPrompt::Prompt;
    for metadata in line_metadata.iter_mut().skip(1) {
        metadata.semantic_prompt = TerminalSemanticPrompt::Continuation;
    }
    TerminalContent {
        lines,
        line_metadata,
        cursor_line: rows as i32 - 1,
        cursor_col: columns as i32,
        cursor_visible: true,
        display_offset: 0,
        colors: TerminalColors::default(),
        mouse: MouseMode::default(),
    }
}

fn soft_wrapped_input_content(rows: usize, columns: usize) -> TerminalContent {
    let lines = (0..rows)
        .map(|row| {
            let mut line = vec![
                TerminalCell {
                    text: "a".into(),
                    ..TerminalCell::blank()
                };
                columns
            ];
            if row == 0 {
                line[0].text = "❯".into();
                line[1].text = " ".into();
            }
            line.into()
        })
        .collect();
    let mut line_metadata = vec![TerminalLineMetadata::default(); rows];
    for row in 0..rows.saturating_sub(1) {
        line_metadata[row].wrapped = true;
        line_metadata[row + 1].wrap_continuation = true;
    }
    TerminalContent {
        lines,
        line_metadata,
        cursor_line: rows as i32 - 1,
        cursor_col: columns as i32,
        cursor_visible: true,
        display_offset: 0,
        colors: TerminalColors::default(),
        mouse: MouseMode::default(),
    }
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
    bench_incremental_snapshot,
    bench_full_pipeline_keyrepeat,
    bench_full_pipeline_latency_guard,
    bench_frame_budget_gates,
    bench_held_key_10s_latency_gate,
    bench_input_selection_long_line_drag_follow,
    bench_input_multiline_edit_range,
    bench_input_wrapped_word_jump,
    bench_fish_undo_capture_coalescing
);
criterion_main!(pipeline);
