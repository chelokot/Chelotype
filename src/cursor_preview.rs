use crate::canvas::{CursorDrawPath, CursorDrawPosition, draw_cursor_visual};
use crate::config::{CursorShape, CursorStyle};
use gtk::cairo;
use std::time::Duration;

pub const PREVIEW_WIDTH: i32 = 480;
pub const PREVIEW_HEIGHT: i32 = 270;
pub const PREVIEW_FRAME_RATE: usize = 240;
pub const PREVIEW_FRAMES: usize = PREVIEW_FRAME_RATE * 5;
const CELL_WIDTH: f64 = 10.0;
const LINE_HEIGHT: f64 = 22.0;
const SEGMENT_FRAMES: usize = PREVIEW_FRAME_RATE;

pub fn draw_preview_frame(
    context: &cairo::Context,
    shape: CursorShape,
    style: CursorStyle,
    frame: usize,
) {
    draw_demo_terminal(context, shape, style, frame);
}

fn draw_demo_terminal(
    context: &cairo::Context,
    shape: CursorShape,
    style: CursorStyle,
    frame: usize,
) {
    context.set_source_rgb(15.0 / 255.0, 17.0 / 255.0, 21.0 / 255.0);
    context.rectangle(0.0, 0.0, PREVIEW_WIDTH as f64, PREVIEW_HEIGHT as f64);
    let _ = context.fill();

    draw_demo_text(context);

    let motion = cursor_motion_sample(frame);
    let progress = motion.progress();
    let eased = 1.0 - (1.0 - progress).powi(3);
    let current = CursorDrawPosition {
        line: motion.from.line + ((motion.target.line - motion.from.line) * eased),
        column: motion.from.column + ((motion.target.column - motion.from.column) * eased),
        pane_id: 0,
    };
    let path = CursorDrawPath {
        from: motion.from,
        current,
        target: motion.target,
        progress,
        elapsed: motion.elapsed,
    };
    draw_cursor_visual(
        context,
        motion.target,
        Some(path),
        style,
        shape,
        LINE_HEIGHT,
        CELL_WIDTH,
    );
}

fn draw_demo_text(context: &cairo::Context) {
    context.select_font_face(
        "monospace",
        cairo::FontSlant::Normal,
        cairo::FontWeight::Normal,
    );
    context.set_font_size(15.0);
    draw_segments(
        context,
        1.0,
        2.0,
        &[("$", DemoColor::Prompt), (" cargo run", DemoColor::Text)],
    );
    draw_segments(
        context,
        2.0,
        4.0,
        &[
            ("Compiling", DemoColor::Yellow),
            (" chelotype v0.1.0", DemoColor::Dim),
        ],
    );
    draw_segments(
        context,
        3.0,
        4.0,
        &[
            ("Finished", DemoColor::Green),
            (" dev profile", DemoColor::Dim),
        ],
    );
    draw_segments(
        context,
        5.0,
        2.0,
        &[
            ("$", DemoColor::Prompt),
            (" echo smooth cursor previews", DemoColor::Text),
        ],
    );
    draw_segments(
        context,
        6.0,
        2.0,
        &[("smooth cursor previews", DemoColor::Cyan)],
    );
    draw_segments(
        context,
        8.0,
        2.0,
        &[
            ("$", DemoColor::Prompt),
            (" git status --short", DemoColor::Text),
        ],
    );
    draw_segments(
        context,
        9.0,
        3.0,
        &[("M", DemoColor::Red), (" src/canvas.rs", DemoColor::Text)],
    );
}

fn cursor_motion_sample(frame: usize) -> PreviewMotion {
    let positions = [
        (5.0, 8.0),
        (5.0, 31.0),
        (8.0, 4.0),
        (8.0, 22.0),
        (1.0, 13.0),
    ];
    let segment = (frame / SEGMENT_FRAMES) % positions.len();
    let next_segment = (segment + 1) % positions.len();
    let elapsed =
        Duration::from_secs_f64((frame % SEGMENT_FRAMES) as f64 / PREVIEW_FRAME_RATE as f64);
    let (from_line, from_column) = positions[segment];
    let (target_line, target_column) = positions[next_segment];
    PreviewMotion {
        from: CursorDrawPosition {
            line: from_line,
            column: from_column,
            pane_id: 0,
        },
        target: CursorDrawPosition {
            line: target_line,
            column: target_column,
            pane_id: 0,
        },
        elapsed,
    }
}

#[derive(Clone, Copy)]
struct PreviewMotion {
    from: CursorDrawPosition,
    target: CursorDrawPosition,
    elapsed: Duration,
}

impl PreviewMotion {
    fn progress(self) -> f64 {
        let duration = f64::from(crate::config::cursor_animation_duration_ms()) / 1000.0;
        (self.elapsed.as_secs_f64() / duration).clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy)]
enum DemoColor {
    Text,
    Dim,
    Prompt,
    Green,
    Yellow,
    Cyan,
    Red,
}

fn draw_segments(context: &cairo::Context, line: f64, column: f64, segments: &[(&str, DemoColor)]) {
    let mut current_column = column;
    for (text, color) in segments {
        set_demo_color(context, *color);
        context.move_to(current_column * CELL_WIDTH, (line + 0.76) * LINE_HEIGHT);
        let _ = context.show_text(text);
        current_column += text.chars().count() as f64;
    }
}

fn set_demo_color(context: &cairo::Context, color: DemoColor) {
    match color {
        DemoColor::Text => context.set_source_rgb(218.0 / 255.0, 225.0 / 255.0, 232.0 / 255.0),
        DemoColor::Dim => context.set_source_rgb(148.0 / 255.0, 163.0 / 255.0, 184.0 / 255.0),
        DemoColor::Prompt => context.set_source_rgb(181.0 / 255.0, 189.0 / 255.0, 104.0 / 255.0),
        DemoColor::Green => context.set_source_rgb(152.0 / 255.0, 195.0 / 255.0, 121.0 / 255.0),
        DemoColor::Yellow => context.set_source_rgb(224.0 / 255.0, 175.0 / 255.0, 104.0 / 255.0),
        DemoColor::Cyan => context.set_source_rgb(125.0 / 255.0, 211.0 / 255.0, 252.0 / 255.0),
        DemoColor::Red => context.set_source_rgb(239.0 / 255.0, 118.0 / 255.0, 122.0 / 255.0),
    }
}
