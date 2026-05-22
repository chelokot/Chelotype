use gtk::pango;
use gtk::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering};

pub const DEFAULT_TERMINAL_FONT: &str = "BlexMono Nerd Font Mono 10";
const SYSTEM_TERMINAL_FONT: &str = "Monospace";
const TERMINAL_FONT_SIZE_PT: f64 = 10.0;
const MIN_FONT_SIZE_TENTHS: u32 = 80;
const MAX_FONT_SIZE_TENTHS: u32 = 280;
const DEFAULT_FONT_SIZE_TENTHS: u32 = (TERMINAL_FONT_SIZE_PT * 10.0) as u32;
const LEGACY_DEFAULT_FONT_SIZE_TENTHS: u32 = 130;
const DEFAULT_XFT_DPI: i32 = 96 * 1024;
const MIN_TEXT_SCALE: f64 = 0.5;
const MAX_TEXT_SCALE: f64 = 3.0;
static FONT_SIZE_TENTHS: AtomicU32 = AtomicU32::new(DEFAULT_FONT_SIZE_TENTHS);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerminalFontMetrics {
    pub cell_width: f64,
    pub line_height: f64,
}

pub fn description() -> pango::FontDescription {
    description_for_text_scale(1.0)
}

pub fn description_for_text_scale(text_scale: f64) -> pango::FontDescription {
    description_for_size_and_text_scale(font_size_pt(), text_scale)
}

pub fn description_for_size_and_text_scale(
    font_size_pt: f64,
    text_scale: f64,
) -> pango::FontDescription {
    let mut description = terminal_font_description();
    let scaled_size = font_size_pt * text_scale.clamp(MIN_TEXT_SCALE, MAX_TEXT_SCALE);
    description.set_size((scaled_size * pango::SCALE as f64).round() as i32);
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

pub fn use_system_font() -> bool {
    crate::config::read_bool("use_system_font", false)
}

pub fn set_use_system_font(enabled: bool) {
    crate::config::write_value("use_system_font", if enabled { "true" } else { "false" });
}

pub fn custom_font() -> String {
    crate::config::read_value("custom_font").unwrap_or_else(|| DEFAULT_TERMINAL_FONT.to_string())
}

pub fn custom_font_label() -> String {
    custom_font()
}

pub fn set_custom_font(font: &str) {
    crate::config::write_value("custom_font", font);
    let description = pango::FontDescription::from_string(font);
    let size = description.size();
    if size > 0 {
        let size_pt = size as f64 / pango::SCALE as f64;
        let value = (size_pt * 10.0).round() as u32;
        FONT_SIZE_TENTHS.store(clamp_size(value), Ordering::Relaxed);
        save_configured_size();
    }
}

pub fn load_configured_size() {
    let Some(value) =
        crate::config::read_value("font_size_tenths").and_then(|value| value.parse::<u32>().ok())
    else {
        return;
    };
    if value == LEGACY_DEFAULT_FONT_SIZE_TENTHS
        && crate::config::read_value("custom_font").is_none()
    {
        FONT_SIZE_TENTHS.store(DEFAULT_FONT_SIZE_TENTHS, Ordering::Relaxed);
        save_configured_size();
        return;
    }
    FONT_SIZE_TENTHS.store(clamp_size(value), Ordering::Relaxed);
}

fn terminal_font_description() -> pango::FontDescription {
    if use_system_font() {
        return pango::FontDescription::from_string(SYSTEM_TERMINAL_FONT);
    }
    pango::FontDescription::from_string(&custom_font())
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
    let value = FONT_SIZE_TENTHS.load(Ordering::Relaxed);
    crate::config::write_value("font_size_tenths", &value.to_string());
}

pub fn layout_for(widget: &gtk::DrawingArea, markup: &str) -> pango::Layout {
    layout_for_size(widget, markup, font_size_pt())
}

pub fn layout_for_size(
    widget: &gtk::DrawingArea,
    markup: &str,
    font_size_pt: f64,
) -> pango::Layout {
    let letter_spacing = letter_spacing_for_widget_size(widget, font_size_pt);
    layout_for_size_with_letter_spacing(widget, markup, font_size_pt, letter_spacing)
}

pub fn layout_for_size_with_letter_spacing(
    widget: &gtk::DrawingArea,
    markup: &str,
    font_size_pt: f64,
    letter_spacing: i32,
) -> pango::Layout {
    let layout = widget.create_pango_layout(None);
    layout.set_font_description(Some(&description_for_size_and_text_scale(
        font_size_pt,
        text_scale_for_widget(widget),
    )));
    layout.set_markup(markup);
    apply_letter_spacing(&layout, letter_spacing);
    layout
}

pub fn metrics_for_widget(widget: &gtk::DrawingArea) -> Option<TerminalFontMetrics> {
    metrics_for_widget_size(widget, font_size_pt())
}

pub fn metrics_for_widget_size(
    widget: &gtk::DrawingArea,
    font_size_pt: f64,
) -> Option<TerminalFontMetrics> {
    let metrics = base_metrics_for_widget_size(widget, font_size_pt)?;
    Some(TerminalFontMetrics {
        cell_width: metrics.cell_width * crate::config::column_spacing(),
        line_height: metrics.line_height * crate::config::line_spacing(),
    })
}

pub fn letter_spacing_for_widget_size(widget: &gtk::DrawingArea, font_size_pt: f64) -> i32 {
    let Some(metrics) = base_metrics_for_widget_size(widget, font_size_pt) else {
        return 0;
    };
    (metrics.cell_width * (crate::config::column_spacing() - 1.0) * pango::SCALE as f64).round()
        as i32
}

fn base_metrics_for_widget_size(
    widget: &gtk::DrawingArea,
    font_size_pt: f64,
) -> Option<TerminalFontMetrics> {
    let description =
        description_for_size_and_text_scale(font_size_pt, text_scale_for_widget(widget));
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

fn apply_letter_spacing(layout: &pango::Layout, letter_spacing: i32) {
    if letter_spacing == 0 {
        return;
    }
    let attributes = layout.attributes().unwrap_or_default();
    let mut spacing = pango::AttrInt::new_letter_spacing(letter_spacing).upcast();
    spacing.set_start_index(0);
    spacing.set_end_index(u32::MAX);
    attributes.insert(spacing);
    layout.set_attributes(Some(&attributes));
}

pub fn text_scale_for_xft_dpi(xft_dpi: i32) -> f64 {
    if xft_dpi <= 0 {
        return 1.0;
    }
    (xft_dpi as f64 / DEFAULT_XFT_DPI as f64).clamp(MIN_TEXT_SCALE, MAX_TEXT_SCALE)
}

pub fn runtime_text_scale(xft_dpi: i32) -> f64 {
    std::env::var("GDK_DPI_SCALE")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .map(|value| value.clamp(MIN_TEXT_SCALE, MAX_TEXT_SCALE))
        .unwrap_or_else(|| text_scale_for_xft_dpi(xft_dpi))
}

pub fn text_scale_for_widget(widget: &gtk::DrawingArea) -> f64 {
    runtime_text_scale(widget.settings().property::<i32>("gtk-xft-dpi"))
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
        assert_eq!(config, "font_size_tenths=110\n");

        std::fs::write(dir.join("config"), "font_size_tenths=170\n").expect("write font config");
        load_configured_size();
        assert_eq!(font_size_pt(), 17.0);

        zoom_reset();
        unsafe {
            std::env::remove_var("CHELOTYPE_CONFIG_DIR");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[serial]
    fn legacy_default_size_migrates_to_current_default() {
        let dir = std::env::temp_dir().join(format!(
            "chelotype-font-legacy-config-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        unsafe {
            std::env::set_var("CHELOTYPE_CONFIG_DIR", &dir);
        }
        std::fs::create_dir_all(&dir).expect("config dir");
        std::fs::write(dir.join("config"), "font_size_tenths=130\n").expect("write font config");

        load_configured_size();
        assert_eq!(font_size_pt(), 10.0);
        let config = std::fs::read_to_string(dir.join("config")).expect("font config");
        assert_eq!(config, "font_size_tenths=100\n");

        unsafe {
            std::env::remove_var("CHELOTYPE_CONFIG_DIR");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[serial]
    fn explicit_legacy_size_with_custom_font_is_preserved() {
        let dir = std::env::temp_dir().join(format!(
            "chelotype-font-explicit-legacy-config-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        unsafe {
            std::env::set_var("CHELOTYPE_CONFIG_DIR", &dir);
        }
        std::fs::create_dir_all(&dir).expect("config dir");
        std::fs::write(
            dir.join("config"),
            "font_size_tenths=130\ncustom_font=BlexMono Nerd Font Mono 13\n",
        )
        .expect("write font config");

        load_configured_size();
        assert_eq!(font_size_pt(), 13.0);

        zoom_reset();
        unsafe {
            std::env::remove_var("CHELOTYPE_CONFIG_DIR");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[serial]
    fn custom_font_defaults_to_nerd_font_and_persists_selection() {
        let dir = std::env::temp_dir().join(format!(
            "chelotype-font-family-config-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        unsafe {
            std::env::set_var("CHELOTYPE_CONFIG_DIR", &dir);
        }

        assert!(!use_system_font());
        assert_eq!(custom_font(), DEFAULT_TERMINAL_FONT);
        set_use_system_font(true);
        assert!(use_system_font());
        set_custom_font("BlexMono Nerd Font Mono 15");
        assert_eq!(custom_font(), "BlexMono Nerd Font Mono 15");
        assert_eq!(font_size_pt(), 15.0);
        assert_eq!(
            terminal_font_description().family().as_deref(),
            Some(SYSTEM_TERMINAL_FONT)
        );
        set_use_system_font(false);
        assert_eq!(
            terminal_font_description().family().as_deref(),
            Some("BlexMono Nerd Font Mono")
        );

        zoom_reset();
        unsafe {
            std::env::remove_var("CHELOTYPE_CONFIG_DIR");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn xft_dpi_text_scale_tracks_system_font_scaling() {
        assert_eq!(text_scale_for_xft_dpi(-1), 1.0);
        assert_eq!(text_scale_for_xft_dpi(DEFAULT_XFT_DPI), 1.0);
        assert_eq!(text_scale_for_xft_dpi(DEFAULT_XFT_DPI * 3 / 2), 1.5);
    }

    #[test]
    #[serial]
    fn runtime_text_scale_honors_gtk_dpi_scale_override() {
        unsafe {
            std::env::set_var("GDK_DPI_SCALE", "1.5");
        }
        assert_eq!(runtime_text_scale(DEFAULT_XFT_DPI), 1.5);
        unsafe {
            std::env::remove_var("GDK_DPI_SCALE");
        }
        assert_eq!(runtime_text_scale(DEFAULT_XFT_DPI), 1.0);
    }

    #[test]
    #[serial]
    fn font_description_applies_system_text_scale_on_top_of_terminal_zoom() {
        let dir = std::env::temp_dir().join(format!(
            "chelotype-font-scale-config-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        unsafe {
            std::env::set_var("CHELOTYPE_CONFIG_DIR", &dir);
        }

        zoom_reset();
        let unscaled = description_for_text_scale(1.0).size();
        let scaled = description_for_text_scale(1.5).size();
        assert_eq!(unscaled, 10 * pango::SCALE);
        assert_eq!(scaled, (15.0 * pango::SCALE as f64).round() as i32);

        zoom_reset();
        unsafe {
            std::env::remove_var("CHELOTYPE_CONFIG_DIR");
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
