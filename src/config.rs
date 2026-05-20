#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorStyle {
    Steady,
    Smooth,
    Smear,
    Neovide,
}

impl CursorStyle {
    pub const ALL: [Self; 4] = [Self::Steady, Self::Smooth, Self::Smear, Self::Neovide];

    pub fn label(self) -> &'static str {
        match self {
            Self::Steady => "Steady",
            Self::Smooth => "Smooth",
            Self::Smear => "Smear",
            Self::Neovide => "Neovide",
        }
    }

    pub fn config_value(self) -> &'static str {
        match self {
            Self::Steady => "steady",
            Self::Smooth => "smooth",
            Self::Smear => "smear",
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
            "smear" => Some(Self::Smear),
            "neovide" => Some(Self::Neovide),
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

pub fn cursor_style() -> CursorStyle {
    if let Some(style) =
        read_value("cursor_style").and_then(|value| CursorStyle::from_config_value(&value))
    {
        return style;
    }
    match read_value("cursor_animation").as_deref() {
        Some("off") => CursorStyle::Steady,
        Some("on") => CursorStyle::Smooth,
        _ => CursorStyle::Neovide,
    }
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
        write_value("cursor_style", "smear");
        assert!(cursor_animation_enabled());
        assert_eq!(cursor_style(), CursorStyle::Smear);
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
}
