use crate::render::{RenderFrame, RenderRun};
use gtk::cairo;
use gtk::pango;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
pub struct TerminalCanvas {
    area: gtk::DrawingArea,
    render: Rc<RefCell<Option<RenderFrame>>>,
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

        let render = Rc::new(RefCell::new(None::<RenderFrame>));
        let draw_render = render.clone();
        area.set_draw_func(move |widget, context, width, height| {
            draw_background(context, width, height);
            if let Some(render) = draw_render.borrow().as_ref() {
                draw_render_output(widget, context, render);
            }
        });

        Self { area, render }
    }

    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.area
    }

    pub fn set_render(&self, render: RenderFrame) {
        *self.render.borrow_mut() = Some(render);
        self.area.queue_draw();
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

fn draw_render_output(widget: &gtk::DrawingArea, context: &cairo::Context, render: &RenderFrame) {
    let line_height = terminal_line_height(widget);
    let cell_width = terminal_cell_width(widget);
    for line in &render.lines {
        let top = line.row as f64 * line_height;
        for run in &line.runs {
            draw_run_background(context, run, cell_width, line_height, top);
        }
        for run in &line.runs {
            let left = run.start_column as f64 * cell_width;
            let layout = terminal_layout(widget, &run_markup(run));
            gtk::render_layout(&widget.style_context(), context, left, top, &layout);
        }
    }

    if render.cursor.visible {
        draw_caret(context, render, line_height, cell_width);
    }
}

fn terminal_layout(widget: &gtk::DrawingArea, markup: &str) -> pango::Layout {
    let layout = widget.create_pango_layout(None);
    layout.set_font_description(Some(&pango::FontDescription::from_string(
        "JetBrains Mono 13",
    )));
    layout.set_markup(markup);
    layout
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

fn run_markup(run: &RenderRun) -> String {
    let mut span = String::from("<span");
    if let Some(fg) = &run.style.fg {
        span.push_str(&format!(" foreground=\"{}\"", fg));
    }
    if run.style.bold {
        span.push_str(" weight=\"bold\"");
    }
    if run.style.italic {
        span.push_str(" style=\"italic\"");
    }
    if run.style.underline {
        span.push_str(" underline=\"single\"");
    }
    if run.style.strikeout {
        span.push_str(" strikethrough=\"true\"");
    }
    span.push('>');
    for ch in run.text.chars() {
        span.push_str(&markup_escape(ch));
    }
    span.push_str("</span>");
    span
}

fn terminal_line_height(widget: &gtk::DrawingArea) -> f64 {
    let metrics = widget.pango_context().metrics(
        Some(&pango::FontDescription::from_string("JetBrains Mono 13")),
        None,
    );
    metrics.height() as f64 / pango::SCALE as f64
}

fn terminal_cell_width(widget: &gtk::DrawingArea) -> f64 {
    let metrics = widget.pango_context().metrics(
        Some(&pango::FontDescription::from_string("JetBrains Mono 13")),
        None,
    );
    metrics.approximate_char_width() as f64 / pango::SCALE as f64
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
    use crate::render::RenderStyle;

    #[test]
    fn run_markup_escapes_text_and_preserves_style() {
        let run = RenderRun {
            start_column: 0,
            columns: 3,
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
}
