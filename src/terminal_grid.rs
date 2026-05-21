use crate::terminal_palette::default_terminal_palette;
use serde::Serialize;
use std::borrow::Cow;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalCell {
    pub text: Cow<'static, str>,
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
            text: Cow::Borrowed(" "),
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
        let palette = default_terminal_palette();
        Self {
            foreground: palette.foreground.to_string(),
            background: palette.background.to_string(),
            cursor: Some(palette.cursor.to_string()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TerminalLineMetadata {
    pub wrapped: bool,
    pub wrap_continuation: bool,
    pub semantic_prompt: TerminalSemanticPrompt,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TerminalSemanticPrompt {
    #[default]
    None,
    Prompt,
    Continuation,
}

#[derive(Clone)]
pub struct TerminalContent {
    pub lines: Vec<Vec<TerminalCell>>,
    pub line_metadata: Vec<TerminalLineMetadata>,
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
