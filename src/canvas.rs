use crate::render::{RenderFrame, RenderRun, RenderStyle};
use crate::terminal_font::{layout_for, metrics_for_widget};
use crate::workspace_render::WorkspaceRenderFrame;
use gtk::cairo;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

const CURSOR_BLINK_PERIOD: Duration = Duration::from_millis(530);
const COMMAND_BLOCK_RAIL: RgbU8 = RgbU8 {
    red: 46,
    green: 166,
    blue: 199,
};
const COMMAND_BLOCK_BG: RgbU8 = RgbU8 {
    red: 18,
    green: 23,
    blue: 31,
};

#[derive(Clone)]
pub struct TerminalCanvas {
    area: gtk::DrawingArea,
    render: Rc<RefCell<Option<CanvasRenderFrame>>>,
    cursor_blink: Rc<Cell<CursorBlinkState>>,
}

impl TerminalCanvas {
    pub fn new() -> Self {
        let area = gtk::DrawingArea::new();
        area.set_focusable(true);
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.set_margin_start(14);
        area.set_margin_end(10);
        area.set_margin_top(8);
        area.set_margin_bottom(12);
        area.add_css_class("term-canvas");
        area.set_cursor_from_name(Some("text"));

        let render = Rc::new(RefCell::new(None::<CanvasRenderFrame>));
        let cursor_blink = Rc::new(Cell::new(CursorBlinkState::default()));
        let draw_render = render.clone();
        let draw_cursor_blink = cursor_blink.clone();
        area.set_draw_func(move |widget, context, width, height| {
            let started = Instant::now();
            draw_background(context, width, height);
            if let Some(render) = draw_render.borrow().as_ref() {
                draw_canvas_render(widget, context, render, draw_cursor_blink.get());
            }
            crate::perf_trace::record_duration("gtk_paint", started.elapsed());
        });

        Self {
            area,
            render,
            cursor_blink,
        }
    }

    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.area
    }

    pub fn set_render(&self, render: RenderFrame) {
        self.set_canvas_render(CanvasRenderFrame::Single(render));
    }

    pub fn set_workspace_render(&self, render: WorkspaceRenderFrame) {
        self.set_canvas_render(CanvasRenderFrame::Workspace(render));
    }

    fn set_canvas_render(&self, render: CanvasRenderFrame) {
        let mut current = self.render.borrow_mut();
        let previous_blink = self.cursor_blink.get();
        self.cursor_blink.set(
            previous_blink.sync(
                render
                    .active_cursor_identity()
                    .unwrap_or_else(CursorIdentity::hidden),
                Instant::now(),
            ),
        );
        if current.as_ref() == Some(&render) {
            if previous_blink != self.cursor_blink.get() {
                self.area.queue_draw();
            }
            return;
        }
        *current = Some(render);
        self.area.queue_draw();
    }

    pub fn tick_cursor_blink(&self) {
        let Some(render) = self.render.borrow().as_ref().cloned() else {
            return;
        };
        let next = self
            .cursor_blink
            .get()
            .tick(render.active_cursor_visible(), Instant::now());
        if self.cursor_blink.get() != next {
            self.cursor_blink.set(next);
            self.area.queue_draw();
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CanvasRenderFrame {
    Single(RenderFrame),
    Workspace(WorkspaceRenderFrame),
}

impl CanvasRenderFrame {
    fn active_cursor_identity(&self) -> Option<CursorIdentity> {
        match self {
            Self::Single(render) => Some(CursorIdentity {
                pane_id: 0,
                line: render.cursor.line,
                column: render.cursor.column,
                visible: render.cursor.visible,
            }),
            Self::Workspace(render) => {
                render
                    .panes
                    .iter()
                    .find(|pane| pane.active)
                    .map(|pane| CursorIdentity {
                        pane_id: pane.pane_id,
                        line: pane.frame.cursor.line,
                        column: pane.frame.cursor.column,
                        visible: pane.frame.cursor.visible,
                    })
            }
        }
    }

    fn active_cursor_visible(&self) -> bool {
        self.active_cursor_identity()
            .is_some_and(|cursor| cursor.visible)
    }
}

impl Default for TerminalCanvas {
    fn default() -> Self {
        Self::new()
    }
}

fn draw_background(context: &cairo::Context, width: i32, height: i32) {
    context.set_source_rgb(15.0 / 255.0, 17.0 / 255.0, 21.0 / 255.0);
    context.rectangle(0.0, 0.0, width as f64, height as f64);
    let _ = context.fill();
}

fn draw_canvas_render(
    widget: &gtk::DrawingArea,
    context: &cairo::Context,
    render: &CanvasRenderFrame,
    cursor_blink: CursorBlinkState,
) {
    let Some(metrics) = metrics_for_widget(widget) else {
        return;
    };
    let line_height = metrics.line_height;
    let cell_width = metrics.cell_width;
    match render {
        CanvasRenderFrame::Single(render) => draw_render_frame(
            widget,
            context,
            render,
            cursor_blink.visible,
            cell_width,
            line_height,
        ),
        CanvasRenderFrame::Workspace(render) => draw_workspace_render(
            widget,
            context,
            render,
            cursor_blink,
            cell_width,
            line_height,
        ),
    }
}

fn draw_render_frame(
    widget: &gtk::DrawingArea,
    context: &cairo::Context,
    render: &RenderFrame,
    draw_cursor: bool,
    cell_width: f64,
    line_height: f64,
) {
    draw_command_blocks(context, render, line_height, cell_width);
    for line in &render.lines {
        let top = line.row as f64 * line_height;
        for run in &line.runs {
            draw_run_background(context, run, cell_width, line_height, top);
        }
        for run in &line.runs {
            if run.text.trim().is_empty() {
                continue;
            }
            let left = run.start_column as f64 * cell_width;
            let layout = layout_for(widget, &run_markup(run));
            let _ = context.save();
            context.rectangle(left, top, run.columns as f64 * cell_width, line_height);
            context.clip();
            gtk::render_layout(&widget.style_context(), context, left, top, &layout);
            let _ = context.restore();
        }
    }

    if render.cursor.visible && draw_cursor {
        if let Some(preedit) = &render.preedit {
            draw_preedit(widget, context, render, preedit, line_height, cell_width);
        } else {
            draw_caret(context, render, line_height, cell_width);
        }
    } else if let Some(preedit) = &render.preedit {
        draw_preedit(widget, context, render, preedit, line_height, cell_width);
    }
}

fn draw_command_blocks(
    context: &cairo::Context,
    render: &RenderFrame,
    line_height: f64,
    cell_width: f64,
) {
    for block in &render.command_blocks {
        if block.end_row < block.prompt_start_row {
            continue;
        }
        let top = block.prompt_start_row as f64 * line_height;
        let height = (block.end_row - block.prompt_start_row + 1) as f64 * line_height;
        set_rgb(context, COMMAND_BLOCK_BG);
        context.rectangle(0.0, top, cell_width * 0.5, height);
        let _ = context.fill();
        set_rgb(context, COMMAND_BLOCK_RAIL);
        context.rectangle(0.0, top + 2.0, 2.0, (height - 4.0).max(1.0));
        let _ = context.fill();
    }
}

fn draw_workspace_render(
    widget: &gtk::DrawingArea,
    context: &cairo::Context,
    render: &WorkspaceRenderFrame,
    cursor_blink: CursorBlinkState,
    cell_width: f64,
    line_height: f64,
) {
    for pane in &render.panes {
        let left = pane.origin_col as f64 * cell_width;
        let top = pane.origin_row as f64 * line_height;
        let width = pane.cols as f64 * cell_width;
        let height = pane.rows as f64 * line_height;
        let _ = context.save();
        context.rectangle(left, top, width, height);
        context.clip();
        context.translate(left, top);
        draw_render_frame(
            widget,
            context,
            &pane.frame,
            pane.active && cursor_blink.visible,
            cell_width,
            line_height,
        );
        let _ = context.restore();
        if pane.index > 0 {
            draw_pane_separator(context, left, top, height);
        }
    }
}

fn draw_pane_separator(context: &cairo::Context, left: f64, top: f64, height: f64) {
    context.set_source_rgb(48.0 / 255.0, 51.0 / 255.0, 58.0 / 255.0);
    context.rectangle(left.round() - 1.0, top, 1.0, height);
    let _ = context.fill();
}

#[derive(Clone, Copy)]
struct RgbU8 {
    red: u8,
    green: u8,
    blue: u8,
}

fn set_rgb(context: &cairo::Context, color: RgbU8) {
    context.set_source_rgb(
        f64::from(color.red) / 255.0,
        f64::from(color.green) / 255.0,
        f64::from(color.blue) / 255.0,
    );
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct CursorBlinkState {
    visible: bool,
    cursor: Option<CursorIdentity>,
    reset_at: Option<Instant>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CursorIdentity {
    pane_id: u64,
    line: i32,
    column: i32,
    visible: bool,
}

impl CursorBlinkState {
    fn sync(self, identity: CursorIdentity, now: Instant) -> Self {
        if !identity.visible {
            return Self {
                visible: false,
                cursor: Some(identity),
                reset_at: None,
            };
        }
        if self.cursor != Some(identity) || !self.visible {
            return Self {
                visible: true,
                cursor: Some(identity),
                reset_at: Some(now),
            };
        }
        self
    }

    fn tick(self, cursor_visible: bool, now: Instant) -> Self {
        if !cursor_visible {
            return Self {
                visible: false,
                cursor: self.cursor,
                reset_at: None,
            };
        }
        let reset_at = self.reset_at.unwrap_or(now);
        let elapsed_periods =
            now.saturating_duration_since(reset_at).as_millis() / CURSOR_BLINK_PERIOD.as_millis();
        Self {
            visible: elapsed_periods.is_multiple_of(2),
            cursor: self.cursor,
            reset_at: Some(reset_at),
        }
    }
}

impl CursorIdentity {
    fn hidden() -> Self {
        Self {
            pane_id: 0,
            line: 0,
            column: 0,
            visible: false,
        }
    }
}

fn draw_run_background(
    context: &cairo::Context,
    run: &RenderRun,
    cell_width: f64,
    line_height: f64,
    top: f64,
) {
    if let Some(color) = run.style.bg.as_deref().and_then(parse_hex_color) {
        context.set_source_rgb(color.red, color.green, color.blue);
        context.rectangle(
            run.start_column as f64 * cell_width,
            top,
            run.columns as f64 * cell_width,
            line_height,
        );
        let _ = context.fill();
    }
}

fn draw_caret(context: &cairo::Context, render: &RenderFrame, line_height: f64, cell_width: f64) {
    let x = render.cursor.column.max(0) as f64 * cell_width;
    let y = render.cursor.line.max(0) as f64 * line_height;
    context.set_source_rgb(125.0 / 255.0, 211.0 / 255.0, 252.0 / 255.0);
    context.rectangle(x.round(), y.round(), 1.25, line_height);
    let _ = context.fill();
}

fn draw_preedit(
    widget: &gtk::DrawingArea,
    context: &cairo::Context,
    render: &RenderFrame,
    preedit: &crate::render::RenderPreedit,
    line_height: f64,
    cell_width: f64,
) {
    let x = preedit.column.max(0) as f64 * cell_width;
    let y = preedit.line.max(0) as f64 * line_height;
    let columns = preedit.text.chars().count().max(1);
    context.set_source_rgb(37.0 / 255.0, 41.0 / 255.0, 48.0 / 255.0);
    context.rectangle(x, y, columns as f64 * cell_width, line_height);
    let _ = context.fill();

    let style = RenderStyle {
        fg: Some("#e5e7eb".to_string()),
        bg: None,
        bold: false,
        italic: false,
        underline: true,
        strikeout: false,
        selected: false,
    };
    let run = RenderRun {
        start_column: preedit.column.max(0) as usize,
        columns,
        text: preedit.text.clone(),
        style,
    };
    let layout = layout_for(widget, &run_markup(&run));
    gtk::render_layout(&widget.style_context(), context, x, y, &layout);

    if render.cursor.visible {
        let cursor_text = preedit
            .text
            .chars()
            .take(preedit.cursor.min(preedit.text.chars().count()))
            .collect::<String>();
        let cursor_x = if cursor_text.is_empty() {
            x
        } else {
            let cursor_layout = layout_for(widget, &markup_escape_text(&cursor_text));
            let (width, _) = cursor_layout.pixel_size();
            x + f64::from(width)
        };
        context.set_source_rgb(125.0 / 255.0, 211.0 / 255.0, 252.0 / 255.0);
        context.rectangle(cursor_x.round(), y.round(), 1.25, line_height);
        let _ = context.fill();
    }
}

fn run_markup(run: &RenderRun) -> String {
    let mut span = String::from("<span");
    push_style_markup(&mut span, &run.style);
    span.push('>');
    for ch in run.text.chars() {
        span.push_str(&markup_escape(ch));
    }
    span.push_str("</span>");
    span
}

fn markup_escape_text(text: &str) -> String {
    let mut escaped = String::new();
    for ch in text.chars() {
        escaped.push_str(&markup_escape(ch));
    }
    escaped
}

fn push_style_markup(span: &mut String, style: &RenderStyle) {
    if let Some(fg) = &style.fg {
        span.push_str(&format!(" foreground=\"{}\"", fg));
    }
    if style.bold {
        span.push_str(" weight=\"bold\"");
    }
    if style.italic {
        span.push_str(" style=\"italic\"");
    }
    if style.underline {
        span.push_str(" underline=\"single\"");
    }
    if style.strikeout {
        span.push_str(" strikethrough=\"true\"");
    }
}

struct Rgb {
    red: f64,
    green: f64,
    blue: f64,
}

fn parse_hex_color(value: &str) -> Option<Rgb> {
    let value = value.strip_prefix('#')?;
    if value.len() != 6 {
        return None;
    }
    let red = u8::from_str_radix(&value[0..2], 16).ok()?;
    let green = u8::from_str_radix(&value[2..4], 16).ok()?;
    let blue = u8::from_str_radix(&value[4..6], 16).ok()?;
    Some(Rgb {
        red: f64::from(red) / 255.0,
        green: f64::from(green) / 255.0,
        blue: f64::from(blue) / 255.0,
    })
}

fn markup_escape(ch: char) -> String {
    match ch {
        '&' => "&amp;".to_string(),
        '<' => "&lt;".to_string(),
        '>' => "&gt;".to_string(),
        _ => ch.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::RenderRun;

    #[test]
    fn run_markup_escapes_text_and_preserves_style() {
        let run = RenderRun {
            start_column: 0,
            columns: 1,
            text: "<&>".to_string(),
            style: RenderStyle {
                fg: Some("#ff0000".to_string()),
                bg: None,
                bold: true,
                italic: true,
                underline: true,
                strikeout: true,
                selected: false,
            },
        };
        let markup = run_markup(&run);
        assert!(markup.contains("foreground=\"#ff0000\""));
        assert!(markup.contains("weight=\"bold\""));
        assert!(markup.contains("style=\"italic\""));
        assert!(markup.contains("underline=\"single\""));
        assert!(markup.contains("strikethrough=\"true\""));
        assert!(markup.contains("&lt;&amp;&gt;"));
    }

    #[test]
    fn parse_hex_color_rejects_invalid_colors() {
        assert!(parse_hex_color("#264f78").is_some());
        assert!(parse_hex_color("264f78").is_none());
        assert!(parse_hex_color("#xyzxyz").is_none());
    }

    #[test]
    fn cursor_blink_toggles_and_resets_on_cursor_movement() {
        let start = Instant::now();
        let cursor = CursorIdentity {
            pane_id: 0,
            line: 1,
            column: 2,
            visible: true,
        };
        let state = CursorBlinkState::default().sync(cursor, start);
        assert!(state.visible);
        let hidden = state.tick(true, start + CURSOR_BLINK_PERIOD);
        assert!(!hidden.visible);
        let moved = hidden.sync(
            CursorIdentity {
                column: 3,
                ..cursor
            },
            start + CURSOR_BLINK_PERIOD + Duration::from_millis(1),
        );
        assert!(moved.visible);
        assert!(
            moved
                .tick(
                    true,
                    start + CURSOR_BLINK_PERIOD + Duration::from_millis(120)
                )
                .visible
        );
    }
}
