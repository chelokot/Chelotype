use crate::render::{RenderFrame, RenderRegion};
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
    let mut input_layout = None;
    for line in &render.lines {
        let layout = terminal_layout(widget, &line.markup);
        let top = line.row as f64 * line_height;
        gtk::render_layout(&widget.style_context(), context, 0.0, top, &layout);
        if line.region == RenderRegion::Input {
            input_layout = Some(layout);
        }
    }

    if render.cursor.visible {
        let layout = input_layout.unwrap_or_else(|| terminal_layout(widget, &render.input_markup));
        draw_caret(context, &layout, render, line_height);
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

fn draw_caret(
    context: &cairo::Context,
    input_layout: &pango::Layout,
    render: &RenderFrame,
    line_height: f64,
) {
    let byte_index = byte_index_for_char(&render.input_text, render.cursor.column.max(0) as usize);
    let caret_pos = input_layout.index_to_pos(byte_index as i32);
    let x = caret_pos.x() as f64 / pango::SCALE as f64;
    let y =
        render.cursor.line.max(0) as f64 * line_height + caret_pos.y() as f64 / pango::SCALE as f64;
    context.set_source_rgb(125.0 / 255.0, 211.0 / 255.0, 252.0 / 255.0);
    context.rectangle(x.round(), y.round(), 1.25, line_height);
    let _ = context.fill();
}

fn terminal_line_height(widget: &gtk::DrawingArea) -> f64 {
    let metrics = widget.pango_context().metrics(
        Some(&pango::FontDescription::from_string("JetBrains Mono 13")),
        None,
    );
    metrics.height() as f64 / pango::SCALE as f64
}

fn byte_index_for_char(text: &str, caret_col: usize) -> usize {
    for (chars_seen, (byte_idx, _)) in text.char_indices().enumerate() {
        if chars_seen == caret_col {
            return byte_idx;
        }
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_char_column_to_byte_index_for_unicode_text() {
        assert_eq!(byte_index_for_char("aя中", 0), 0);
        assert_eq!(byte_index_for_char("aя中", 1), 1);
        assert_eq!(byte_index_for_char("aя中", 2), 3);
        assert_eq!(byte_index_for_char("aя中", 3), "aя中".len());
        assert_eq!(byte_index_for_char("aя中", 30), "aя中".len());
    }
}
