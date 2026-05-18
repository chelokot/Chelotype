use chelotype::bridge::{
    CaretHandle, EntryHandle, InputBridge, LabelHandle, TerminalAdapter, html_to_pango,
};
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use gtk::Label;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Once;
use std::time::{Duration, Instant};
use vte::{self, Format, Terminal as VteTerminal, prelude::*};

#[derive(Clone, Default)]
struct BenchEntry {
    text: Rc<RefCell<String>>,
    position: Rc<Cell<i32>>,
}

impl EntryHandle for BenchEntry {
    fn set_text(&self, text: &str) {
        self.text.borrow_mut().clear();
        self.text.borrow_mut().push_str(text);
    }

    fn set_position(&self, pos: i32) {
        self.position.set(pos);
    }
}

#[derive(Clone, Default)]
struct BenchLabel {
    markup: Rc<RefCell<String>>,
}

impl LabelHandle for BenchLabel {
    fn set_markup(&self, markup: &str) {
        self.markup.borrow_mut().clear();
        self.markup.borrow_mut().push_str(markup);
    }

    fn index_at_x(&self, x: f64, _y: f64) -> Option<usize> {
        Some(x as usize)
    }

    fn caret_offset(&self, caret_pos: usize) -> f64 {
        caret_pos as f64
    }
}

#[derive(Clone, Default)]
struct BenchCaret {
    offset: Rc<Cell<f64>>,
    visible: Rc<Cell<bool>>,
}

impl CaretHandle for BenchCaret {
    fn set_offset(&self, x: f64) {
        self.offset.set(x);
    }

    fn set_visible(&self, visible: bool) {
        self.visible.set(visible);
    }
}

#[derive(Clone, Default)]
struct BenchRenderProbe {
    invocations: Rc<Cell<u64>>,
}

impl BenchRenderProbe {
    fn bump(&self) {
        let v = self.invocations.get();
        self.invocations.set(v + 1);
    }

    fn count(&self) -> u64 {
        self.invocations.get()
    }
}

static GTK_INIT: Once = Once::new();

fn ensure_gtk() {
    GTK_INIT.call_once(|| {
        gtk::init().expect("gtk init failed for benchmarks");
    });
}

#[derive(Clone, Default)]
struct BenchTerminal {
    fed: Rc<RefCell<Vec<u8>>>,
    line_html: Rc<RefCell<String>>,
    line_text: Rc<RefCell<String>>,
    cursor: Rc<RefCell<(i64, i64)>>,
    columns: i64,
    render_probe: Option<BenchRenderProbe>,
}

impl BenchTerminal {
    fn with_line(text: &str) -> Self {
        Self {
            fed: Rc::new(RefCell::new(Vec::new())),
            line_html: Rc::new(RefCell::new(text.to_string())),
            line_text: Rc::new(RefCell::new(text.to_string())),
            cursor: Rc::new(RefCell::new((text.len() as i64, 0))),
            columns: text.len() as i64 + 10,
            render_probe: None,
        }
    }

    fn with_probe(text: &str, probe: BenchRenderProbe) -> Self {
        Self {
            render_probe: Some(probe),
            ..Self::with_line(text)
        }
    }

    fn set_line(&self, text: &str, html: Option<&str>, cursor: i64) {
        *self.line_text.borrow_mut() = text.to_string();
        *self.line_html.borrow_mut() = html.unwrap_or(text).to_string();
        *self.cursor.borrow_mut() = (cursor, 0);
        if let Some(p) = &self.render_probe {
            p.bump();
        }
    }
}

impl TerminalAdapter for BenchTerminal {
    fn feed_child(&self, bytes: &[u8]) {
        self.fed.borrow_mut().extend_from_slice(bytes);
    }

    fn cursor_position(&self) -> (i64, i64) {
        *self.cursor.borrow()
    }

    fn line_html(&self, _row: i64) -> String {
        self.line_html.borrow().clone()
    }

    fn line_text(&self, _row: i64) -> String {
        self.line_text.borrow().clone()
    }

    fn column_count(&self) -> i64 {
        self.columns
    }
}

fn bench_sync_large_markup(c: &mut Criterion) {
    let entry = BenchEntry::default();
    let ghost = BenchLabel::default();
    let caret = BenchCaret::default();
    let shadow = BenchTerminal::with_line("");
    let output = BenchTerminal::with_line("");
    let bridge = InputBridge::new(entry, ghost, caret, shadow.clone(), output);
    let long = "libadwaita-".repeat(512);
    let markup = html_to_pango(&format!("<span foreground=\"#00ff88\">{long}</span>"));
    shadow.set_line(&long, Some(&markup), long.len() as i64);
    c.bench_function("sync_from_terminal_large_markup", |b| {
        b.iter(|| bridge.sync_from_terminal());
    });
}

fn bench_insert_and_sync(c: &mut Criterion) {
    let payload = "echo hello world";
    c.bench_function("insert_and_sync_shadow", |b| {
        b.iter_batched(
            || {
                let entry = BenchEntry::default();
                let ghost = BenchLabel::default();
                let caret = BenchCaret::default();
                let shadow = BenchTerminal::with_line("");
                let output = BenchTerminal::with_line("");
                let bridge = InputBridge::new(entry, ghost, caret, shadow.clone(), output);
                (bridge, shadow)
            },
            |(bridge, shadow)| {
                bridge.handle_insert_text(payload);
                shadow.set_line(payload, None, payload.len() as i64);
                bridge.sync_from_terminal();
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_cursor_moves(c: &mut Criterion) {
    let payload = "x".repeat(256);
    c.bench_function("cursor_moves_shadow", |b| {
        b.iter_batched(
            || {
                let entry = BenchEntry::default();
                let ghost = BenchLabel::default();
                let caret = BenchCaret::default();
                let shadow = BenchTerminal::with_line(&payload);
                shadow.set_line(&payload, None, 0);
                let output = BenchTerminal::with_line("");
                let bridge = InputBridge::new(entry, ghost, caret, shadow.clone(), output);
                bridge.sync_from_terminal();
                bridge
            },
            |bridge| {
                bridge.move_cursor_to(128);
                bridge.move_cursor_to(0);
                bridge.move_cursor_to(255);
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_sustained_key_repeat(c: &mut Criterion) {
    c.bench_function("sustained_key_repeat_shadow", |b| {
        b.iter_custom(|iters| {
            let entry = BenchEntry::default();
            let ghost = BenchLabel::default();
            let caret = BenchCaret::default();
            let shadow = BenchTerminal::with_line("");
            let output = BenchTerminal::with_line("");
            let bridge = InputBridge::new(entry, ghost, caret, shadow.clone(), output);
            let mut line = String::new();
            let start = Instant::now();
            for _ in 0..iters {
                bridge.handle_insert_text("a");
                line.push('a');
                shadow.set_line(&line, None, line.len() as i64);
                bridge.sync_from_terminal();
            }
            start.elapsed()
        });
    });
}

fn bench_sustained_key_hold_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("sustained_key_hold_latency");
    group
        .sample_size(10)
        .measurement_time(Duration::from_secs(5));
    group.bench_function("sustained_key_hold_latency", |b| {
        b.iter_batched(
            || {
                let entry = BenchEntry::default();
                let ghost = BenchLabel::default();
                let caret = BenchCaret::default();
                let probe = BenchRenderProbe::default();
                let shadow = BenchTerminal::with_probe("", probe.clone());
                let output = BenchTerminal::with_line("");
                let bridge = InputBridge::new(entry, ghost, caret, shadow.clone(), output);
                (bridge, shadow, probe)
            },
            |(bridge, shadow, probe)| {
                let deadline = Instant::now() + Duration::from_secs(1);
                let mut line = String::new();
                let mut worst = Duration::ZERO;
                while Instant::now() < deadline {
                    let t0 = Instant::now();
                    bridge.handle_insert_text("a");
                    line.push('a');
                    shadow.set_line(&line, None, line.len() as i64);
                    bridge.sync_from_terminal();
                    let dt = t0.elapsed();
                    if dt > worst {
                        worst = dt;
                    }
                }
                let _renders = probe.count();
                worst
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

fn bench_label_render_markup(c: &mut Criterion) {
    ensure_gtk();
    let markup = html_to_pango(&format!(
        "<span foreground=\"#7dd3fc\">{}</span>",
        "libadwaita-".repeat(256)
    ));
    c.bench_function("label_render_markup_layout", |b| {
        b.iter_batched(
            || Label::new(None),
            |label| {
                label.set_use_markup(true);
                label.set_markup(&markup);
                let layout = label.layout();
                let _ = layout.extents();
                let _ = layout.line_count();
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_overlay_render_cycle(c: &mut Criterion) {
    ensure_gtk();
    let markup = html_to_pango(&format!(
        "<span foreground=\"#00ff88\">{}</span>",
        "libadwaita-".repeat(128)
    ));
    c.bench_function("overlay_render_cycle", |b| {
        b.iter_batched(
            || {
                let ghost = Label::new(None);
                ghost.set_use_markup(true);
                let caret = Label::new(Some("│"));
                (ghost, caret)
            },
            |(ghost, caret)| {
                ghost.set_markup(&markup);
                let layout = ghost.layout();
                let _ = layout.extents();
                caret.set_margin_start((layout.pixel_size().0 / 2) as i32);
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_overlay_frame_budget(c: &mut Criterion) {
    ensure_gtk();
    let markup = html_to_pango(&format!(
        "<span foreground=\"#00ff88\">{}</span>",
        "libadwaita-".repeat(256)
    ));
    let budget = Duration::from_micros(16_666);
    c.bench_function("overlay_frame_budget_60hz", |b| {
        b.iter_custom(|iters| {
            let ghost = Label::new(None);
            ghost.set_use_markup(true);
            let caret = Label::new(Some("│"));
            let start = Instant::now();
            let mut misses = 0u64;
            for _ in 0..iters {
                let t0 = Instant::now();
                ghost.set_markup(&markup);
                let layout = ghost.layout();
                let size = layout.pixel_size().0;
                caret.set_margin_start(size / 2);
                let dt = t0.elapsed();
                if dt > budget {
                    misses += 1;
                }
            }
            if misses > 0 {
                panic!("frame budget missed {misses} times over {iters} frames");
            }
            start.elapsed()
        });
    });
}

fn bench_jitter_latency_p99(c: &mut Criterion) {
    ensure_gtk();
    let mut group = c.benchmark_group("jitter_latency_overlay");
    group
        .sample_size(10)
        .measurement_time(Duration::from_secs(3));
    let markup = html_to_pango(&format!(
        "<span foreground=\"#5fd700\">{}</span>",
        "libadwaita-".repeat(64)
    ));
    group.bench_function("jitter_latency_p99_overlay", |b| {
        b.iter_custom(|_| {
            let ghost = Label::new(None);
            ghost.set_use_markup(true);
            let caret = Label::new(Some("│"));
            let mut samples = Vec::with_capacity(5_000);
            let deadline = Instant::now() + Duration::from_secs(1);
            while Instant::now() < deadline {
                let t0 = Instant::now();
                ghost.set_markup(&markup);
                let layout = ghost.layout();
                let size = layout.pixel_size().0;
                caret.set_margin_start(size / 2);
                samples.push(t0.elapsed());
            }
            samples.sort();
            let idx95 = (samples.len() as f64 * 0.95).floor() as usize;
            let idx99 = (samples.len() as f64 * 0.99).floor() as usize;
            let p95 = samples.get(idx95).copied().unwrap_or_default();
            let p99 = samples.get(idx99).copied().unwrap_or_default();
            if p95 > Duration::from_millis(5) || p99 > Duration::from_millis(8) {
                panic!("jitter too high: p95={:?}, p99={:?}", p95, p99);
            }
            samples.into_iter().max().unwrap_or_default()
        });
    });
    group.finish();
}

fn bench_vte_repaint_cycle(c: &mut Criterion) {
    ensure_gtk();
    let term = VteTerminal::builder()
        .input_enabled(true)
        .can_focus(false)
        .build();
    term.set_size(200, 80);
    term.set_scrollback_lines(1000);
    let line = format!("{}\n", "libadwaita-".repeat(64));
    term.feed(line.as_bytes());
    c.bench_function("vte_repaint_cycle_html", |b| {
        b.iter(|| {
            term.feed(b"a");
            let (html, _) = term.text_range_format(Format::Html, 0, 0, 0, term.column_count());
            let _ = html.map(|h| h.len()).unwrap_or(0);
        });
    });
}

fn bench_vte_line_extract(c: &mut Criterion) {
    ensure_gtk();
    let term = VteTerminal::builder()
        .input_enabled(true)
        .can_focus(false)
        .build();
    term.set_size(240, 80);
    term.set_scrollback_lines(5000);
    let long = "libadwaita-".repeat(512);
    let with_newline = format!("{long}\n");
    term.feed(with_newline.as_bytes());
    c.bench_function("vte_line_extract_html", |b| {
        b.iter(|| {
            let _ = term.cursor_position();
            let (html, _) = term.text_range_format(Format::Html, 0, 0, 0, term.column_count());
            let _ = html.map(|h| h.len()).unwrap_or(0);
        });
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(50)
        .measurement_time(Duration::from_secs(2));
    targets =
        bench_sync_large_markup,
        bench_insert_and_sync,
        bench_cursor_moves,
        bench_sustained_key_repeat,
        bench_sustained_key_hold_latency,
        bench_label_render_markup,
        bench_overlay_render_cycle,
        bench_overlay_frame_budget,
        bench_jitter_latency_p99,
        bench_vte_repaint_cycle,
        bench_vte_line_extract
}
criterion_main!(benches);
