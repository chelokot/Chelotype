#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorStyle {
    Steady,
    Smooth,
    Snappy,
    Neovide,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorShape {
    Bar,
    Block,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorCornerStyle {
    Square,
    Rounded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorBlinking {
    FollowSystem,
    Enabled,
    Disabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorBlinkAnimation {
    Instant,
    Smooth,
}

pub const DEFAULT_CURSOR_ANIMATION_DURATION_MS: u32 = 100;
pub const DEFAULT_CURSOR_BLINK_INTERVAL_MS: u32 = 530;
pub const DEFAULT_CURSOR_WIDTH_RATIO: f64 = 0.125;
pub const DEFAULT_NEOVIDE_SHORT_ANIMATION_DURATION_MS: u32 = 40;
pub const DEFAULT_NEOVIDE_SHORT_JUMP_DISTANCE: f64 = 2.0;
pub const DEFAULT_NEOVIDE_TRAIL_SIZE: f64 = 0.65;
pub const DEFAULT_NEOVIDE_BLOCK_OPACITY: f64 = 0.72;
pub const DEFAULT_SMOOTH_SCROLLING: bool = true;
pub const DEFAULT_LINE_SPACING: f64 = 1.0;
pub const DEFAULT_COLUMN_SPACING: f64 = 1.0;
const MIN_TERMINAL_SPACING: f64 = 0.5;
const MAX_TERMINAL_SPACING: f64 = 2.0;

impl CursorStyle {
    pub const ALL: [Self; 4] = [Self::Steady, Self::Smooth, Self::Snappy, Self::Neovide];

    pub fn label(self) -> &'static str {
        match self {
            Self::Steady => "Instant",
            Self::Smooth => "Smooth",
            Self::Snappy => "Snappy",
            Self::Neovide => "Neovide",
        }
    }

    pub fn config_value(self) -> &'static str {
        match self {
            Self::Steady => "steady",
            Self::Smooth => "smooth",
            Self::Snappy => "snappy",
            Self::Neovide => "neovide",
        }
    }

    pub fn selected_index(self) -> u32 {
        Self::ALL
            .iter()
            .position(|style| *style == self)
            .unwrap_or(0)
            .min(u32::MAX as usize) as u32
    }

    pub fn from_selected_index(index: u32) -> Self {
        Self::ALL
            .get(index as usize)
            .copied()
            .unwrap_or(Self::Neovide)
    }

    fn from_config_value(value: &str) -> Option<Self> {
        match value {
            "off" | "steady" => Some(Self::Steady),
            "on" | "smooth" => Some(Self::Smooth),
            "snappy" => Some(Self::Snappy),
            "neovide" => Some(Self::Neovide),
            _ => None,
        }
    }
}

impl CursorShape {
    pub const ALL: [Self; 2] = [Self::Bar, Self::Block];

    pub fn label(self) -> &'static str {
        match self {
            Self::Bar => "Bar",
            Self::Block => "Block",
        }
    }

    pub fn config_value(self) -> &'static str {
        match self {
            Self::Bar => "bar",
            Self::Block => "block",
        }
    }

    pub fn selected_index(self) -> u32 {
        Self::ALL
            .iter()
            .position(|shape| *shape == self)
            .unwrap_or(0)
            .min(u32::MAX as usize) as u32
    }

    pub fn from_selected_index(index: u32) -> Self {
        Self::ALL.get(index as usize).copied().unwrap_or(Self::Bar)
    }

    fn from_config_value(value: &str) -> Option<Self> {
        match value {
            "bar" => Some(Self::Bar),
            "block" => Some(Self::Block),
            _ => None,
        }
    }
}

impl CursorCornerStyle {
    pub const ALL: [Self; 2] = [Self::Square, Self::Rounded];

    pub fn label(self) -> &'static str {
        match self {
            Self::Square => "Square",
            Self::Rounded => "Rounded",
        }
    }

    pub fn config_value(self) -> &'static str {
        match self {
            Self::Square => "square",
            Self::Rounded => "rounded",
        }
    }

    fn from_config_value(value: &str) -> Option<Self> {
        match value {
            "square" => Some(Self::Square),
            "rounded" => Some(Self::Rounded),
            _ => None,
        }
    }
}

impl CursorBlinking {
    pub const ALL: [Self; 3] = [Self::FollowSystem, Self::Enabled, Self::Disabled];

    pub fn label(self) -> &'static str {
        match self {
            Self::FollowSystem => "Follow System",
            Self::Enabled => "Enabled",
            Self::Disabled => "Disabled",
        }
    }

    pub fn config_value(self) -> &'static str {
        match self {
            Self::FollowSystem => "system",
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        }
    }

    pub fn selected_index(self) -> u32 {
        Self::ALL
            .iter()
            .position(|mode| *mode == self)
            .unwrap_or(0)
            .min(u32::MAX as usize) as u32
    }

    pub fn from_selected_index(index: u32) -> Self {
        Self::ALL
            .get(index as usize)
            .copied()
            .unwrap_or(Self::FollowSystem)
    }

    fn from_config_value(value: &str) -> Option<Self> {
        match value {
            "system" | "follow-system" => Some(Self::FollowSystem),
            "on" | "true" | "enabled" => Some(Self::Enabled),
            "off" | "false" | "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }
}

impl CursorBlinkAnimation {
    pub const ALL: [Self; 2] = [Self::Instant, Self::Smooth];

    pub fn label(self) -> &'static str {
        match self {
            Self::Instant => "Instant",
            Self::Smooth => "Smooth",
        }
    }

    pub fn config_value(self) -> &'static str {
        match self {
            Self::Instant => "instant",
            Self::Smooth => "smooth",
        }
    }

    fn from_config_value(value: &str) -> Option<Self> {
        match value {
            "instant" => Some(Self::Instant),
            "smooth" => Some(Self::Smooth),
            _ => None,
        }
    }
}

pub fn read_value(key: &str) -> Option<String> {
    let path = config_path()?;
    let content = std::fs::read_to_string(path).ok()?;
    content.lines().find_map(|line| {
        let (line_key, value) = line.split_once('=')?;
        (line_key == key).then(|| value.trim().to_string())
    })
}

pub fn write_value(key: &str, value: &str) {
    let Some(path) = config_path() else {
        return;
    };
    let mut entries = std::fs::read_to_string(&path)
        .ok()
        .map(|content| {
            content
                .lines()
                .filter_map(|line| {
                    let (line_key, line_value) = line.split_once('=')?;
                    Some((line_key.to_string(), line_value.to_string()))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if let Some((_, existing)) = entries.iter_mut().find(|(line_key, _)| line_key == key) {
        *existing = value.to_string();
    } else {
        entries.push((key.to_string(), value.to_string()));
    }
    let content = entries
        .into_iter()
        .map(|(line_key, line_value)| format!("{line_key}={line_value}\n"))
        .collect::<String>();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, content);
}

pub fn cursor_animation_enabled() -> bool {
    cursor_style() != CursorStyle::Steady
}

pub fn smooth_scrolling_enabled() -> bool {
    read_bool("smooth_scrolling", DEFAULT_SMOOTH_SCROLLING)
}

pub fn take_first_launch_preferences() -> bool {
    if read_bool("first_launch_preferences_shown", false) {
        return false;
    }
    write_value("first_launch_preferences_shown", "true");
    true
}

pub fn cursor_style() -> CursorStyle {
    if let Some(value) = read_value("cursor_style") {
        return CursorStyle::from_config_value(&value).unwrap_or(CursorStyle::Neovide);
    }
    match read_value("cursor_animation").as_deref() {
        Some("off") => CursorStyle::Steady,
        Some("on") => CursorStyle::Smooth,
        _ => CursorStyle::Neovide,
    }
}

pub fn cursor_shape() -> CursorShape {
    read_value("cursor_shape")
        .and_then(|value| CursorShape::from_config_value(&value))
        .unwrap_or(CursorShape::Bar)
}

pub fn cursor_corner_style() -> CursorCornerStyle {
    read_value("cursor_corner_style")
        .and_then(|value| CursorCornerStyle::from_config_value(&value))
        .unwrap_or(CursorCornerStyle::Square)
}

pub fn cursor_width_ratio() -> f64 {
    read_f64("cursor_width_ratio", DEFAULT_CURSOR_WIDTH_RATIO, 0.05, 1.0)
}

pub fn cursor_blinking() -> CursorBlinking {
    read_value("cursor_blinking")
        .and_then(|value| CursorBlinking::from_config_value(&value))
        .unwrap_or(CursorBlinking::FollowSystem)
}

pub fn cursor_blink_animation() -> CursorBlinkAnimation {
    read_value("cursor_blink_animation")
        .and_then(|value| CursorBlinkAnimation::from_config_value(&value))
        .unwrap_or(CursorBlinkAnimation::Instant)
}

pub fn cursor_blink_interval_ms() -> u32 {
    read_u32(
        "cursor_blink_interval_ms",
        DEFAULT_CURSOR_BLINK_INTERVAL_MS,
        150,
        1500,
    )
}

pub fn cursor_animation_duration_ms() -> u32 {
    read_u32(
        "cursor_animation_duration_ms",
        DEFAULT_CURSOR_ANIMATION_DURATION_MS,
        40,
        500,
    )
}

pub fn cursor_neovide_trail_size() -> f64 {
    read_f64(
        "cursor_neovide_trail_size",
        DEFAULT_NEOVIDE_TRAIL_SIZE,
        0.0,
        1.0,
    )
}

pub fn cursor_neovide_short_animation_duration_ms() -> u32 {
    read_u32(
        "cursor_neovide_short_animation_duration_ms",
        DEFAULT_NEOVIDE_SHORT_ANIMATION_DURATION_MS,
        10,
        150,
    )
}

pub fn cursor_neovide_short_jump_distance() -> f64 {
    read_f64(
        "cursor_neovide_short_jump_distance",
        DEFAULT_NEOVIDE_SHORT_JUMP_DISTANCE,
        0.0,
        8.0,
    )
}

pub fn cursor_neovide_block_opacity() -> f64 {
    read_f64(
        "cursor_neovide_block_opacity",
        DEFAULT_NEOVIDE_BLOCK_OPACITY,
        0.1,
        1.0,
    )
}

pub fn line_spacing() -> f64 {
    read_f64(
        "line_spacing",
        DEFAULT_LINE_SPACING,
        MIN_TERMINAL_SPACING,
        MAX_TERMINAL_SPACING,
    )
}

pub fn set_line_spacing(value: f64) {
    write_terminal_spacing("line_spacing", value);
}

pub fn column_spacing() -> f64 {
    read_f64(
        "column_spacing",
        DEFAULT_COLUMN_SPACING,
        MIN_TERMINAL_SPACING,
        MAX_TERMINAL_SPACING,
    )
}

pub fn set_column_spacing(value: f64) {
    write_terminal_spacing("column_spacing", value);
}

fn write_terminal_spacing(key: &str, value: f64) {
    let value = value.clamp(MIN_TERMINAL_SPACING, MAX_TERMINAL_SPACING);
    write_value(key, &format!("{value:.1}"));
}

fn read_u32(key: &str, default: u32, min: u32, max: u32) -> u32 {
    read_value(key)
        .and_then(|value| value.parse::<u32>().ok())
        .map(|value| value.clamp(min, max))
        .unwrap_or(default)
}

fn read_f64(key: &str, default: f64, min: f64, max: f64) -> f64 {
    read_value(key)
        .and_then(|value| value.parse::<f64>().ok())
        .map(|value| value.clamp(min, max))
        .unwrap_or(default)
}

pub fn read_bool(key: &str, default: bool) -> bool {
    match read_value(key).as_deref() {
        Some("on" | "true" | "1") => true,
        Some("off" | "false" | "0") => false,
        Some(_) | None => default,
    }
}

fn config_path() -> Option<std::path::PathBuf> {
    config_dir().map(|dir| dir.join("config"))
}

pub fn wheel_profile_path() -> Option<std::path::PathBuf> {
    config_dir().map(|dir| dir.join("wheel-profile.tsv"))
}

fn config_dir() -> Option<std::path::PathBuf> {
    if let Ok(path) = std::env::var("CHELOTYPE_CONFIG_DIR") {
        return Some(std::path::PathBuf::from(path));
    }
    if let Ok(path) = std::env::var("XDG_CONFIG_HOME") {
        return Some(std::path::PathBuf::from(path).join("chelotype"));
    }
    std::env::var("HOME")
        .ok()
        .map(|home| std::path::PathBuf::from(home).join(".config/chelotype"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    #[serial]
    fn config_writer_updates_one_key_without_dropping_others() {
        let dir = std::env::temp_dir().join(format!(
            "chelotype-config-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        unsafe {
            std::env::set_var("CHELOTYPE_CONFIG_DIR", &dir);
        }

        write_value("font_size_tenths", "150");
        write_value("startup_launch_target", "toolbox:fedora-toolbox-latest");
        write_value("font_size_tenths", "160");

        assert_eq!(read_value("font_size_tenths").as_deref(), Some("160"));
        assert_eq!(
            read_value("startup_launch_target").as_deref(),
            Some("toolbox:fedora-toolbox-latest")
        );

        unsafe {
            std::env::remove_var("CHELOTYPE_CONFIG_DIR");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[serial]
    fn cursor_style_defaults_to_neovide_and_keeps_legacy_animation_config() {
        let dir = std::env::temp_dir().join(format!(
            "chelotype-cursor-style-config-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        unsafe {
            std::env::set_var("CHELOTYPE_CONFIG_DIR", &dir);
        }

        assert!(cursor_animation_enabled());
        assert_eq!(cursor_style(), CursorStyle::Neovide);
        write_value("cursor_animation", "off");
        assert!(!cursor_animation_enabled());
        assert_eq!(cursor_style(), CursorStyle::Steady);
        write_value("cursor_animation", "on");
        assert!(cursor_animation_enabled());
        assert_eq!(cursor_style(), CursorStyle::Smooth);
        write_value("cursor_style", "unknown");
        assert!(cursor_animation_enabled());
        assert_eq!(cursor_style(), CursorStyle::Neovide);
        write_value("cursor_style", "snappy");
        assert!(cursor_animation_enabled());
        assert_eq!(cursor_style(), CursorStyle::Snappy);
        write_value("cursor_style", "neovide");
        assert_eq!(cursor_style(), CursorStyle::Neovide);
        write_value("cursor_style", "steady");
        assert!(!cursor_animation_enabled());
        assert_eq!(cursor_style(), CursorStyle::Steady);

        unsafe {
            std::env::remove_var("CHELOTYPE_CONFIG_DIR");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[serial]
    fn first_launch_preferences_are_taken_once() {
        let dir = std::env::temp_dir().join(format!(
            "chelotype-first-launch-config-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        unsafe {
            std::env::set_var("CHELOTYPE_CONFIG_DIR", &dir);
        }

        assert!(take_first_launch_preferences());
        assert!(!take_first_launch_preferences());
        assert_eq!(
            read_value("first_launch_preferences_shown").as_deref(),
            Some("true")
        );

        unsafe {
            std::env::remove_var("CHELOTYPE_CONFIG_DIR");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[serial]
    fn terminal_spacing_defaults_persists_and_clamps() {
        let dir = std::env::temp_dir().join(format!(
            "chelotype-terminal-spacing-config-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        unsafe {
            std::env::set_var("CHELOTYPE_CONFIG_DIR", &dir);
        }

        assert_eq!(line_spacing(), DEFAULT_LINE_SPACING);
        assert_eq!(column_spacing(), DEFAULT_COLUMN_SPACING);
        set_line_spacing(1.3);
        set_column_spacing(0.8);
        assert_eq!(line_spacing(), 1.3);
        assert_eq!(column_spacing(), 0.8);
        assert_eq!(read_value("line_spacing").as_deref(), Some("1.3"));
        assert_eq!(read_value("column_spacing").as_deref(), Some("0.8"));
        set_line_spacing(4.0);
        set_column_spacing(0.1);
        assert_eq!(line_spacing(), MAX_TERMINAL_SPACING);
        assert_eq!(column_spacing(), MIN_TERMINAL_SPACING);

        unsafe {
            std::env::remove_var("CHELOTYPE_CONFIG_DIR");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[serial]
    fn cursor_shape_defaults_to_bar_and_reads_block_config() {
        let dir = std::env::temp_dir().join(format!(
            "chelotype-cursor-shape-config-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        unsafe {
            std::env::set_var("CHELOTYPE_CONFIG_DIR", &dir);
        }

        assert_eq!(cursor_shape(), CursorShape::Bar);
        write_value("cursor_shape", "block");
        assert_eq!(cursor_shape(), CursorShape::Block);
        write_value("cursor_shape", "bar");
        assert_eq!(cursor_shape(), CursorShape::Bar);
        write_value("cursor_shape", "unknown");
        assert_eq!(cursor_shape(), CursorShape::Bar);

        unsafe {
            std::env::remove_var("CHELOTYPE_CONFIG_DIR");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[serial]
    fn cursor_corner_style_defaults_to_square_and_reads_config() {
        let dir = std::env::temp_dir().join(format!(
            "chelotype-cursor-corner-config-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        unsafe {
            std::env::set_var("CHELOTYPE_CONFIG_DIR", &dir);
        }

        assert_eq!(cursor_corner_style(), CursorCornerStyle::Square);
        assert_eq!(cursor_width_ratio(), DEFAULT_CURSOR_WIDTH_RATIO);
        write_value("cursor_corner_style", "rounded");
        assert_eq!(cursor_corner_style(), CursorCornerStyle::Rounded);
        write_value("cursor_corner_style", "square");
        assert_eq!(cursor_corner_style(), CursorCornerStyle::Square);
        write_value("cursor_corner_style", "unknown");
        assert_eq!(cursor_corner_style(), CursorCornerStyle::Square);
        write_value("cursor_width_ratio", "0.25");
        assert_eq!(cursor_width_ratio(), 0.25);
        write_value("cursor_width_ratio", "0.01");
        assert_eq!(cursor_width_ratio(), 0.05);
        write_value("cursor_width_ratio", "2");
        assert_eq!(cursor_width_ratio(), 1.0);

        unsafe {
            std::env::remove_var("CHELOTYPE_CONFIG_DIR");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[serial]
    fn cursor_blinking_defaults_to_system_and_reads_config() {
        let dir = std::env::temp_dir().join(format!(
            "chelotype-cursor-blinking-config-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        unsafe {
            std::env::set_var("CHELOTYPE_CONFIG_DIR", &dir);
        }

        assert_eq!(cursor_blinking(), CursorBlinking::FollowSystem);
        write_value("cursor_blinking", "enabled");
        assert_eq!(cursor_blinking(), CursorBlinking::Enabled);
        write_value("cursor_blinking", "disabled");
        assert_eq!(cursor_blinking(), CursorBlinking::Disabled);
        write_value("cursor_blinking", "unexpected");
        assert_eq!(cursor_blinking(), CursorBlinking::FollowSystem);

        unsafe {
            std::env::remove_var("CHELOTYPE_CONFIG_DIR");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[serial]
    fn cursor_blink_animation_and_interval_default_and_clamp() {
        let dir = std::env::temp_dir().join(format!(
            "chelotype-cursor-blink-animation-config-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        unsafe {
            std::env::set_var("CHELOTYPE_CONFIG_DIR", &dir);
        }

        assert_eq!(cursor_blink_animation(), CursorBlinkAnimation::Instant);
        write_value("cursor_blink_animation", "smooth");
        assert_eq!(cursor_blink_animation(), CursorBlinkAnimation::Smooth);
        write_value("cursor_blink_animation", "instant");
        assert_eq!(cursor_blink_animation(), CursorBlinkAnimation::Instant);
        write_value("cursor_blink_animation", "unknown");
        assert_eq!(cursor_blink_animation(), CursorBlinkAnimation::Instant);
        assert_eq!(cursor_blink_interval_ms(), DEFAULT_CURSOR_BLINK_INTERVAL_MS);
        write_value("cursor_blink_interval_ms", "100");
        assert_eq!(cursor_blink_interval_ms(), 150);
        write_value("cursor_blink_interval_ms", "2500");
        assert_eq!(cursor_blink_interval_ms(), 1500);

        unsafe {
            std::env::remove_var("CHELOTYPE_CONFIG_DIR");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[serial]
    fn smooth_scrolling_defaults_to_enabled_and_reads_config() {
        let dir = std::env::temp_dir().join(format!(
            "chelotype-smooth-scrolling-config-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        unsafe {
            std::env::set_var("CHELOTYPE_CONFIG_DIR", &dir);
        }

        assert!(smooth_scrolling_enabled());
        write_value("smooth_scrolling", "off");
        assert!(!smooth_scrolling_enabled());
        write_value("smooth_scrolling", "on");
        assert!(smooth_scrolling_enabled());
        write_value("smooth_scrolling", "unexpected");
        assert!(smooth_scrolling_enabled());

        unsafe {
            std::env::remove_var("CHELOTYPE_CONFIG_DIR");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[serial]
    fn cursor_animation_parameters_clamp_config_values() {
        let dir = std::env::temp_dir().join(format!(
            "chelotype-cursor-animation-config-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        unsafe {
            std::env::set_var("CHELOTYPE_CONFIG_DIR", &dir);
        }

        assert_eq!(
            cursor_animation_duration_ms(),
            DEFAULT_CURSOR_ANIMATION_DURATION_MS
        );
        write_value("cursor_animation_duration_ms", "15");
        assert_eq!(cursor_animation_duration_ms(), 40);
        write_value("cursor_animation_duration_ms", "900");
        assert_eq!(cursor_animation_duration_ms(), 500);
        write_value("cursor_neovide_trail_size", "2");
        assert_eq!(cursor_neovide_trail_size(), 1.0);
        write_value("cursor_neovide_short_animation_duration_ms", "2");
        assert_eq!(cursor_neovide_short_animation_duration_ms(), 10);
        write_value("cursor_neovide_short_animation_duration_ms", "900");
        assert_eq!(cursor_neovide_short_animation_duration_ms(), 150);
        write_value("cursor_neovide_short_jump_distance", "-1");
        assert_eq!(cursor_neovide_short_jump_distance(), 0.0);
        write_value("cursor_neovide_short_jump_distance", "20");
        assert_eq!(cursor_neovide_short_jump_distance(), 8.0);
        write_value("cursor_neovide_block_opacity", "0");
        assert_eq!(cursor_neovide_block_opacity(), 0.1);
        write_value("cursor_neovide_block_opacity", "2");
        assert_eq!(cursor_neovide_block_opacity(), 1.0);

        unsafe {
            std::env::remove_var("CHELOTYPE_CONFIG_DIR");
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
