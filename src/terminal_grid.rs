use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalCell {
    pub text: String,
    pub fg: Option<String>,
    pub bg: Option<String>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
    pub strikeout: bool,
    pub wide: bool,
    pub wide_spacer: bool,
}

impl TerminalCell {
    pub fn blank() -> Self {
        Self {
            text: " ".to_string(),
            fg: None,
            bg: None,
            bold: false,
            italic: false,
            underline: false,
            inverse: false,
            strikeout: false,
            wide: false,
            wide_spacer: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalColors {
    pub foreground: String,
    pub background: String,
    pub cursor: Option<String>,
}

impl Default for TerminalColors {
    fn default() -> Self {
        Self {
            foreground: "#e5e7eb".to_string(),
            background: "#0f1115".to_string(),
            cursor: None,
        }
    }
}

#[derive(Clone)]
pub struct TerminalContent {
    pub lines: Vec<Vec<TerminalCell>>,
    pub cursor_line: i32,
    pub cursor_col: i32,
    pub cursor_visible: bool,
    pub display_offset: usize,
    pub colors: TerminalColors,
    pub mouse: MouseMode,
}

#[derive(Clone, Copy, Default, Debug, Eq, PartialEq, Serialize)]
pub struct MouseMode {
    pub click: bool,
    pub drag: bool,
    pub motion: bool,
    pub sgr: bool,
    pub utf8: bool,
}

impl MouseMode {
    pub fn sends_press_release(self) -> bool {
        self.click || self.drag || self.motion
    }

    pub fn sends_drag(self) -> bool {
        self.drag || self.motion
    }
}
