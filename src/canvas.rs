use crate::render::{RenderCell, RenderFrame, RenderStyle};
use crate::terminal_font::{layout_for, metrics_for_widget};
use gtk::cairo;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

const CURSOR_BLINK_PERIOD: Duration = Duration::from_millis(530);

#[derive(Clone)]
pub struct TerminalCanvas {
    area: gtk::DrawingArea,
    render: Rc<RefCell<Option<RenderFrame>>>,
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

        let render = Rc::new(RefCell::new(None::<RenderFrame>));
        let cursor_blink = Rc::new(Cell::new(CursorBlinkState::default()));
        let draw_render = render.clone();
        let draw_cursor_blink = cursor_blink.clone();
        area.set_draw_func(move |widget, context, width, height| {
            draw_background(context, width, height);
            if let Some(render) = draw_render.borrow().as_ref() {
                draw_render_output(widget, context, render, draw_cursor_blink.get());
            }
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
        let mut current = self.render.borrow_mut();
        let previous_blink = self.cursor_blink.get();
        self.cursor_blink
            .set(previous_blink.sync(render.cursor.clone(), Instant::now()));
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
            .tick(render.cursor.visible, Instant::now());
        if self.cursor_blink.get() != next {
            self.cursor_blink.set(next);
            self.area.queue_draw();
        }
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

fn draw_render_output(
    widget: &gtk::DrawingArea,
    context: &cairo::Context,
    render: &RenderFrame,
    cursor_blink: CursorBlinkState,
) {
    let Some(metrics) = metrics_for_widget(widget) else {
        return;
    };
    let line_height = metrics.line_height;
    let cell_width = metrics.cell_width;
    for line in &render.lines {
        let top = line.row as f64 * line_height;
        for cell in &line.cells {
            draw_cell_background(context, cell, cell_width, line_height, top);
        }
        for cell in &line.cells {
            if cell.text == " " {
                continue;
            }
            let left = cell.column as f64 * cell_width;
            let layout = layout_for(widget, &cell_markup(cell));
            gtk::render_layout(&widget.style_context(), context, left, top, &layout);
        }
    }

    if render.cursor.visible && cursor_blink.visible {
        draw_caret(context, render, line_height, cell_width);
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct CursorBlinkState {
    visible: bool,
    cursor: Option<CursorIdentity>,
    reset_at: Option<Instant>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CursorIdentity {
    line: i32,
    column: i32,
    visible: bool,
}

impl CursorBlinkState {
    fn sync(self, cursor: crate::render::RenderCursor, now: Instant) -> Self {
        let identity = CursorIdentity {
            line: cursor.line,
            column: cursor.column,
            visible: cursor.visible,
        };
        if !cursor.visible {
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

fn draw_cell_background(
    context: &cairo::Context,
    cell: &RenderCell,
    cell_width: f64,
    line_height: f64,
    top: f64,
) {
    if let Some(color) = cell.style.bg.as_deref().and_then(parse_hex_color) {
        context.set_source_rgb(color.red, color.green, color.blue);
        context.rectangle(
            cell.column as f64 * cell_width,
            top,
            cell.columns as f64 * cell_width,
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

fn cell_markup(cell: &RenderCell) -> String {
    let mut span = String::from("<span");
    push_style_markup(&mut span, &cell.style);
    span.push('>');
    for ch in cell.text.chars() {
        span.push_str(&markup_escape(ch));
    }
    span.push_str("</span>");
    span
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
    use crate::render::RenderCell;

    #[test]
    fn cell_markup_escapes_text_and_preserves_style() {
        let cell = RenderCell {
            column: 0,
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
        let markup = cell_markup(&cell);
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
        let cursor = crate::render::RenderCursor {
            line: 1,
            column: 2,
            visible: true,
            color: "#7dd3fc".to_string(),
        };
        let state = CursorBlinkState::default().sync(cursor.clone(), start);
        assert!(state.visible);
        let hidden = state.tick(true, start + CURSOR_BLINK_PERIOD);
        assert!(!hidden.visible);
        let moved = hidden.sync(
            crate::render::RenderCursor {
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
