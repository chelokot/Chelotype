use gtk::pango;
use gtk::prelude::*;

pub const TERMINAL_FONT: &str = "Source Code Pro";
const TERMINAL_FONT_SIZE_PX: f64 = 13.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerminalFontMetrics {
    pub cell_width: f64,
    pub line_height: f64,
}

pub fn description() -> pango::FontDescription {
    let mut description = pango::FontDescription::from_string(TERMINAL_FONT);
    description.set_absolute_size(TERMINAL_FONT_SIZE_PX * pango::SCALE as f64);
    description
}

pub fn layout_for(widget: &gtk::DrawingArea, markup: &str) -> pango::Layout {
    let layout = widget.create_pango_layout(None);
    layout.set_font_description(Some(&description()));
    layout.set_markup(markup);
    layout
}

pub fn metrics_for_widget(widget: &gtk::DrawingArea) -> Option<TerminalFontMetrics> {
    let metrics = widget.pango_context().metrics(Some(&description()), None);
    let cell_width = metrics.approximate_digit_width() as f64 / pango::SCALE as f64;
    let line_height = metrics.height() as f64 / pango::SCALE as f64;
    if cell_width <= 0.0 || line_height <= 0.0 {
        return None;
    }
    Some(TerminalFontMetrics {
        cell_width,
        line_height,
    })
}
