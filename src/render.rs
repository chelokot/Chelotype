use crate::backend::RenderableContentOwned;
use crate::selection::{GridPoint, SelectionRange};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::vte::ansi::{Color, NamedColor};
use serde::Serialize;

#[derive(Clone, Serialize)]
pub struct RenderCursor {
    pub line: i32,
    pub column: i32,
    pub visible: bool,
    pub color: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum RenderRegion {
    History,
    Input,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RenderLine {
    pub row: usize,
    pub region: RenderRegion,
    pub text: String,
    pub markup: String,
}

#[derive(Clone, Serialize)]
pub struct RenderFrame {
    pub history_markup: String,
    pub input_markup: String,
    pub cursor: RenderCursor,
    pub input_text: String,
    pub lines: Vec<RenderLine>,
}

#[derive(Clone)]
pub struct RenderOutput {
    pub history_markup: String,
    pub input_markup: String,
    pub cursor: RenderCursor,
    pub input_text: String,
}

pub struct Renderer;

impl Renderer {
    pub fn render(content: RenderableContentOwned) -> RenderOutput {
        Self::render_with_selection(content, None)
    }

    pub fn render_with_selection(
        content: RenderableContentOwned,
        selection: Option<SelectionRange>,
    ) -> RenderOutput {
        let mut history_markup = String::new();
        let mut input_markup = String::new();
        let mut input_text = String::new();
        for (idx, line) in content.lines.iter().enumerate() {
            let markup = cells_to_markup(line, &content.colors, idx, selection);
            if idx as i32 == content.cursor_line {
                input_markup.push_str(&markup);
                input_text = cells_to_text(line);
            } else {
                history_markup.push_str(&markup);
                if idx + 1 != content.lines.len() {
                    history_markup.push('\n');
                }
            }
        }

        RenderOutput {
            history_markup,
            input_markup,
            cursor: RenderCursor {
                line: content.cursor_line,
                column: content.cursor_col,
                visible: content.cursor_visible,
                color: "#7dd3fc".to_string(),
            },
            input_text,
        }
    }

    pub fn render_frame_with_selection(
        content: RenderableContentOwned,
        selection: Option<SelectionRange>,
    ) -> RenderFrame {
        let mut history_markup = String::new();
        let mut input_markup = String::new();
        let mut input_text = String::new();
        let mut lines = Vec::with_capacity(content.lines.len());
        for (idx, line) in content.lines.iter().enumerate() {
            let markup = cells_to_markup(line, &content.colors, idx, selection);
            let text = cells_to_text(line);
            if idx as i32 == content.cursor_line {
                input_markup.push_str(&markup);
                input_text = text.clone();
                lines.push(RenderLine {
                    row: idx,
                    region: RenderRegion::Input,
                    text,
                    markup,
                });
            } else {
                history_markup.push_str(&markup);
                if idx + 1 != content.lines.len() {
                    history_markup.push('\n');
                }
                lines.push(RenderLine {
                    row: idx,
                    region: RenderRegion::History,
                    text,
                    markup,
                });
            }
        }

        RenderFrame {
            history_markup,
            input_markup,
            cursor: RenderCursor {
                line: content.cursor_line,
                column: content.cursor_col,
                visible: content.cursor_visible,
                color: "#7dd3fc".to_string(),
            },
            input_text,
            lines,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct CellStyle {
    fg: Color,
    bg: Color,
    flags: Flags,
    selected: bool,
}

impl CellStyle {
    fn from_cell(cell: &Cell, selected: bool) -> Self {
        Self {
            fg: cell.fg,
            bg: cell.bg,
            flags: cell.flags,
            selected,
        }
    }
}

fn style_open(style: CellStyle, colors: &alacritty_terminal::term::color::Colors) -> String {
    let mut span = String::from("<span");
    if let Some(rgb) = color_to_hex(style.fg, colors) {
        span.push_str(&format!(" foreground=\"{}\"", rgb));
    }
    if style.selected {
        span.push_str(" background=\"#264f78\"");
    } else if let Some(bg) = color_to_hex(style.bg, colors) {
        span.push_str(&format!(" background=\"{}\"", bg));
    }
    if style.flags.contains(Flags::BOLD) {
        span.push_str(" weight=\"bold\"");
    }
    if style.flags.contains(Flags::ITALIC) {
        span.push_str(" style=\"italic\"");
    }
    if style.flags.intersects(Flags::ALL_UNDERLINES) {
        span.push_str(" underline=\"single\"");
    }
    if style.flags.contains(Flags::STRIKEOUT) {
        span.push_str(" strikethrough=\"true\"");
    }
    span.push('>');
    span
}

fn color_to_hex(color: Color, palette: &alacritty_terminal::term::color::Colors) -> Option<String> {
    match color {
        Color::Named(named) => {
            palette[named].map(|rgb| format!("#{:02x}{:02x}{:02x}", rgb.r, rgb.g, rgb.b))
        }
        Color::Indexed(idx) => {
            palette[idx as usize].map(|rgb| format!("#{:02x}{:02x}{:02x}", rgb.r, rgb.g, rgb.b))
        }
        Color::Spec(rgb) => Some(format!("#{:02x}{:02x}{:02x}", rgb.r, rgb.g, rgb.b)),
    }
}

fn cells_to_markup(
    cells: &[Cell],
    colors: &alacritty_terminal::term::color::Colors,
    row: usize,
    selection: Option<SelectionRange>,
) -> String {
    let mut out = String::new();
    let cells = &cells[..significant_len(cells)];
    let mut idx = 0;
    while idx < cells.len() {
        let style = CellStyle::from_cell(&cells[idx], is_selected(selection, row, idx));
        out.push_str(&style_open(style, colors));
        while idx < cells.len()
            && CellStyle::from_cell(&cells[idx], is_selected(selection, row, idx)) == style
        {
            push_escaped_char(&mut out, cells[idx].c);
            idx += 1;
        }
        out.push_str("</span>");
    }
    out
}

fn is_selected(selection: Option<SelectionRange>, row: usize, column: usize) -> bool {
    selection
        .map(|range| range.contains(GridPoint { row, column }))
        .unwrap_or(false)
}

fn cells_to_text(cells: &[Cell]) -> String {
    let mut out = String::new();
    for cell in &cells[..significant_len(cells)] {
        out.push(cell.c);
    }
    out
}

fn significant_len(cells: &[Cell]) -> usize {
    cells
        .iter()
        .rposition(|cell| !is_trimmable_blank(cell))
        .map(|idx| idx + 1)
        .unwrap_or(0)
}

fn is_trimmable_blank(cell: &Cell) -> bool {
    cell.c == ' '
        && cell.zerowidth().is_none()
        && cell.flags.is_empty()
        && matches!(cell.bg, Color::Named(NamedColor::Background))
}

fn push_escaped_char(out: &mut String, ch: char) {
    match ch {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        _ => out.push(ch),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{MouseMode, RenderableContentOwned};
    use alacritty_terminal::term::cell::Cell;
    use alacritty_terminal::term::color::Colors;
    use alacritty_terminal::vte::ansi::NamedColor;

    fn cell(ch: char) -> Cell {
        Cell {
            c: ch,
            ..Cell::default()
        }
    }

    #[test]
    fn markup_escapes_text_cells() {
        let cells = [cell('<'), cell('&'), cell('>')];
        let markup = cells_to_markup(&cells, &Colors::default(), 0, None);
        assert!(markup.contains("&lt;&amp;&gt;"));
    }

    #[test]
    fn markup_trims_plain_blank_tail() {
        let cells = [cell('o'), cell('k'), Cell::default(), Cell::default()];
        let markup = cells_to_markup(&cells, &Colors::default(), 0, None);
        assert!(markup.contains(">ok</span>"));
        assert!(!markup.contains("ok  "));
    }

    #[test]
    fn markup_preserves_styled_blank_tail() {
        let mut styled_blank = Cell::default();
        styled_blank.bg = Color::Named(NamedColor::Red);
        let cells = [cell('x'), styled_blank];
        let markup = cells_to_markup(&cells, &Colors::default(), 0, None);
        assert!(markup.contains(">x</span>"));
        assert!(markup.contains("> </span>"));
    }

    #[test]
    fn markup_marks_selected_cells_without_changing_text() {
        let cells = [cell('a'), cell('b'), cell('c'), cell('d')];
        let selection = SelectionRange::new(
            GridPoint { row: 0, column: 1 },
            GridPoint { row: 0, column: 3 },
        );
        let markup = cells_to_markup(&cells, &Colors::default(), 0, Some(selection));
        assert!(markup.contains(">a</span>"));
        assert!(markup.contains("background=\"#264f78\">bc</span>"));
        assert!(markup.contains(">d</span>"));
    }

    #[test]
    fn renderer_exports_structured_history_and_input_lines() {
        let content = RenderableContentOwned {
            lines: vec![
                vec![cell('o'), cell('l'), cell('d')],
                vec![cell('n'), cell('e'), cell('w')],
            ],
            cursor_line: 1,
            cursor_col: 3,
            cursor_visible: true,
            display_offset: 0,
            colors: Colors::default(),
            mouse: MouseMode::default(),
        };
        let frame = Renderer::render_frame_with_selection(content, None);
        assert_eq!(frame.lines.len(), 2);
        assert_eq!(frame.lines[0].region, RenderRegion::History);
        assert_eq!(frame.lines[0].text, "old");
        assert_eq!(frame.lines[1].region, RenderRegion::Input);
        assert_eq!(frame.lines[1].text, "new");
        assert_eq!(frame.input_text, "new");
    }
}
