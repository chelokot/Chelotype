use gtk::pango;
use gtk::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering};

pub const TERMINAL_FONT: &str = "Adwaita Mono";
const TERMINAL_FONT_SIZE_PT: f64 = 13.0;
const MIN_FONT_SIZE_TENTHS: u32 = 80;
const MAX_FONT_SIZE_TENTHS: u32 = 280;
const DEFAULT_FONT_SIZE_TENTHS: u32 = (TERMINAL_FONT_SIZE_PT * 10.0) as u32;
static FONT_SIZE_TENTHS: AtomicU32 = AtomicU32::new(DEFAULT_FONT_SIZE_TENTHS);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerminalFontMetrics {
    pub cell_width: f64,
    pub line_height: f64,
}

pub fn description() -> pango::FontDescription {
    let mut description = pango::FontDescription::from_string(TERMINAL_FONT);
    description.set_size((font_size_pt() * pango::SCALE as f64).round() as i32);
    description
}

pub fn font_size_pt() -> f64 {
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
    save_configured_size();
}

pub fn load_configured_size() {
    let Some(path) = config_path() else {
        return;
    };
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let Some(value) = content
        .lines()
        .find_map(|line| line.strip_prefix("font_size_tenths="))
        .and_then(|value| value.trim().parse::<u32>().ok())
    else {
        return;
    };
    FONT_SIZE_TENTHS.store(clamp_size(value), Ordering::Relaxed);
}

fn adjust_font_size(delta: i32) {
    let current = FONT_SIZE_TENTHS.load(Ordering::Relaxed) as i32;
    let next = (current + delta).clamp(MIN_FONT_SIZE_TENTHS as i32, MAX_FONT_SIZE_TENTHS as i32);
    FONT_SIZE_TENTHS.store(next as u32, Ordering::Relaxed);
    save_configured_size();
}

fn clamp_size(value: u32) -> u32 {
    value.clamp(MIN_FONT_SIZE_TENTHS, MAX_FONT_SIZE_TENTHS)
}

fn save_configured_size() {
    let Some(path) = config_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let value = FONT_SIZE_TENTHS.load(Ordering::Relaxed);
    let _ = std::fs::write(path, format!("font_size_tenths={value}\n"));
}

fn config_path() -> Option<std::path::PathBuf> {
    if let Ok(path) = std::env::var("CHELOTYPE_CONFIG_DIR") {
        return Some(std::path::PathBuf::from(path).join("config"));
    }
    if let Ok(path) = std::env::var("XDG_CONFIG_HOME") {
        return Some(std::path::PathBuf::from(path).join("chelotype/config"));
    }
    std::env::var("HOME")
        .ok()
        .map(|home| std::path::PathBuf::from(home).join(".config/chelotype/config"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    #[serial]
    fn zoom_size_persists_to_config_and_loads_back() {
        let dir = std::env::temp_dir().join(format!(
            "chelotype-font-config-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        unsafe {
            std::env::set_var("CHELOTYPE_CONFIG_DIR", &dir);
        }

        zoom_reset();
        zoom_in();
        let config = std::fs::read_to_string(dir.join("config")).expect("font config");
        assert_eq!(config, "font_size_tenths=140\n");

        std::fs::write(dir.join("config"), "font_size_tenths=170\n").expect("write font config");
        load_configured_size();
        assert_eq!(font_size_pt(), 17.0);

        zoom_reset();
        unsafe {
            std::env::remove_var("CHELOTYPE_CONFIG_DIR");
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
