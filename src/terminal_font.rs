use gtk::pango;
use gtk::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering};

pub const TERMINAL_FONT: &str = "Adwaita Mono";
const TERMINAL_FONT_SIZE_PX: f64 = 13.0;
const MIN_FONT_SIZE_TENTHS: u32 = 80;
const MAX_FONT_SIZE_TENTHS: u32 = 280;
const DEFAULT_FONT_SIZE_TENTHS: u32 = (TERMINAL_FONT_SIZE_PX * 10.0) as u32;
static FONT_SIZE_TENTHS: AtomicU32 = AtomicU32::new(DEFAULT_FONT_SIZE_TENTHS);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerminalFontMetrics {
    pub cell_width: f64,
    pub line_height: f64,
}

pub fn description() -> pango::FontDescription {
    let mut description = pango::FontDescription::from_string(TERMINAL_FONT);
    description.set_absolute_size(font_size_px() * pango::SCALE as f64);
    description
}

pub fn font_size_px() -> f64 {
    FONT_SIZE_TENTHS.load(Ordering::Relaxed) as f64 / 10.0
}

pub fn zoom_in() {
    adjust_font_size(10);
}

pub fn zoom_out() {
    adjust_font_size(-10);
}

pub fn zoom_reset() {
    FONT_SIZE_TENTHS.store(DEFAULT_FONT_SIZE_TENTHS, Ordering::Relaxed);
}

fn adjust_font_size(delta: i32) {
    let current = FONT_SIZE_TENTHS.load(Ordering::Relaxed) as i32;
    let next = (current + delta).clamp(MIN_FONT_SIZE_TENTHS as i32, MAX_FONT_SIZE_TENTHS as i32);
    FONT_SIZE_TENTHS.store(next as u32, Ordering::Relaxed);
}

pub fn layout_for(widget: &gtk::DrawingArea, markup: &str) -> pango::Layout {
    let layout = widget.create_pango_layout(None);
    layout.set_font_description(Some(&description()));
    layout.set_markup(markup);
    layout
}

pub fn metrics_for_widget(widget: &gtk::DrawingArea) -> Option<TerminalFontMetrics> {
    let description = description();
    let sample = "00000000000000000000000000000000";
    let layout = widget.create_pango_layout(Some(sample));
    layout.set_font_description(Some(&description));
    let (sample_width, line_height) = layout.size();
    let cell_width = sample_width as f64 / pango::SCALE as f64 / sample.chars().count() as f64;
    let line_height = line_height as f64 / pango::SCALE as f64;
    if cell_width <= 0.0 || line_height <= 0.0 {
        return None;
    }
    Some(TerminalFontMetrics {
        cell_width,
        line_height,
    })
}
