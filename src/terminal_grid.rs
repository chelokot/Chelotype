use crate::terminal_palette::default_terminal_palette;
use serde::Serialize;
use std::borrow::Cow;
use std::ops::Deref;
use std::sync::Arc;

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

#[derive(Clone, Debug)]
pub struct TerminalLine {
    cells: Arc<[TerminalCell]>,
    character_offsets: Option<Arc<[usize]>>,
}

impl TerminalLine {
    pub fn uses_visual_character_offsets(&self) -> bool {
        self.character_offsets.is_none()
    }

    pub fn character_offset(&self, column: usize) -> usize {
        self.character_offsets
            .as_ref()
            .map(|offsets| offsets[column.min(self.cells.len())])
            .unwrap_or_else(|| column.min(self.cells.len()))
    }
}

impl From<Vec<TerminalCell>> for TerminalLine {
    fn from(cells: Vec<TerminalCell>) -> Self {
        let visual_offsets = cells
            .iter()
            .all(|cell| !cell.wide_spacer && cell.text.len() == 1 && cell.text.is_ascii());
        let character_offsets = if visual_offsets {
            None
        } else {
            let mut offsets = Vec::with_capacity(cells.len() + 1);
            let mut offset = 0;
            offsets.push(offset);
            for cell in &cells {
                if !cell.wide_spacer {
                    offset += cell.text.chars().count();
                }
                offsets.push(offset);
            }
            Some(offsets.into())
        };
        Self {
            cells: cells.into(),
            character_offsets,
        }
    }
}

impl FromIterator<TerminalCell> for TerminalLine {
    fn from_iter<Cells: IntoIterator<Item = TerminalCell>>(cells: Cells) -> Self {
        cells.into_iter().collect::<Vec<_>>().into()
    }
}

impl Deref for TerminalLine {
    type Target = [TerminalCell];

    fn deref(&self) -> &Self::Target {
        &self.cells
    }
}

impl AsRef<[TerminalCell]> for TerminalLine {
    fn as_ref(&self) -> &[TerminalCell] {
        self
    }
}

impl PartialEq for TerminalLine {
    fn eq(&self, other: &Self) -> bool {
        self.cells == other.cells
    }
}

impl Eq for TerminalLine {}

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
    pub lines: Vec<TerminalLine>,
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
