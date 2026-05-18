use crate::backend::RenderableContentOwned;
use crate::cell_text::{is_wide_spacer, push_cell_text};
use crate::selection::{GridPoint, SelectionRange};
use crate::terminal_grid::TerminalCell;
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
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
    pub cells: Vec<RenderCell>,
    pub runs: Vec<RenderRun>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RenderCell {
    pub column: usize,
    pub columns: usize,
    pub text: String,
    pub style: RenderStyle,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RenderRun {
    pub start_column: usize,
    pub columns: usize,
    pub text: String,
    pub style: RenderStyle,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RenderStyle {
    pub fg: Option<String>,
    pub bg: Option<String>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikeout: bool,
    pub selected: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RenderFrame {
    pub history_markup: String,
    pub input_markup: String,
    pub cursor: RenderCursor,
    pub input_text: String,
    pub lines: Vec<RenderLine>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
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
        let input_range = input_region_range(&content);
        for (idx, line) in content.lines.iter().enumerate() {
            let markup = cells_to_markup(line, idx, selection);
            if input_range.contains(&idx) {
                if !input_markup.is_empty() {
                    input_markup.push('\n');
                }
                input_markup.push_str(&markup);
                if idx as i32 == content.cursor_line {
                    input_text = cells_to_text(line);
                }
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
        let input_range = input_region_range(&content);
        for (idx, line) in content.lines.iter().enumerate() {
            let line_render = build_line_render(line, idx, selection);
            if input_range.contains(&idx) {
                if !input_markup.is_empty() {
                    input_markup.push('\n');
                }
                input_markup.push_str(&line_render.markup);
                if idx as i32 == content.cursor_line {
                    input_text = line_render.text.clone();
                }
                lines.push(RenderLine {
                    row: idx,
                    region: RenderRegion::Input,
                    text: line_render.text,
                    markup: line_render.markup,
                    cells: line_render.cells,
                    runs: line_render.runs,
                });
            } else {
                history_markup.push_str(&line_render.markup);
                if idx + 1 != content.lines.len() {
                    history_markup.push('\n');
                }
                lines.push(RenderLine {
                    row: idx,
                    region: RenderRegion::History,
                    text: line_render.text,
                    markup: line_render.markup,
                    cells: line_render.cells,
                    runs: line_render.runs,
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

fn input_region_range(content: &RenderableContentOwned) -> std::ops::RangeInclusive<usize> {
    let cursor = content.cursor_line.max(0) as usize;
    let end = cursor.min(content.lines.len().saturating_sub(1));
    let mut start = end;
    while start > 0 {
        let current = content
            .line_metadata
            .get(start)
            .copied()
            .unwrap_or_default();
        let previous = content
            .line_metadata
            .get(start - 1)
            .copied()
            .unwrap_or_default();
        if current.wrap_continuation || previous.wrapped {
            start -= 1;
        } else {
            break;
        }
    }
    start..=end
}

struct LineRender {
    text: String,
    markup: String,
    cells: Vec<RenderCell>,
    runs: Vec<RenderRun>,
}

#[derive(Clone, Eq, PartialEq)]
struct CellStyle {
    fg: Option<String>,
    bg: Option<String>,
    bold: bool,
    italic: bool,
    underline: bool,
    strikeout: bool,
    selected: bool,
}

impl CellStyle {
    fn from_cell(cell: &TerminalCell, selected: bool) -> Self {
        Self {
            fg: cell.fg.clone(),
            bg: cell.bg.clone(),
            bold: cell.bold,
            italic: cell.italic,
            underline: cell.underline,
            strikeout: cell.strikeout,
            selected,
        }
    }
}

fn style_open(style: &CellStyle) -> String {
    let mut span = String::from("<span");
    if let Some(rgb) = &style.fg {
        span.push_str(&format!(" foreground=\"{}\"", rgb));
    }
    if style.selected {
        span.push_str(" background=\"#264f78\"");
    } else if let Some(bg) = &style.bg {
        span.push_str(&format!(" background=\"{}\"", bg));
    }
    if style.bold {
        span.push_str(" weight=\"bold\"");
    }
    if style.italic {
        span.push_str(" style=\"italic\"");
    }
    if style.underline {
        span.push_str(" underline=\"single\"");
    }
    if style.strikeout {
        span.push_str(" strikethrough=\"true\"");
    }
    span.push('>');
    span
}

fn cells_to_markup(
    cells: &[TerminalCell],
    row: usize,
    selection: Option<SelectionRange>,
) -> String {
    let mut out = String::new();
    let cells = &cells[..significant_len(cells)];
    let mut idx = 0;
    while idx < cells.len() {
        let style = CellStyle::from_cell(&cells[idx], is_selected(selection, row, idx));
        out.push_str(&style_open(&style));
        while idx < cells.len()
            && CellStyle::from_cell(&cells[idx], is_selected(selection, row, idx)) == style
        {
            push_escaped_cell(&mut out, &cells[idx]);
            idx += 1;
        }
        out.push_str("</span>");
    }
    out
}

#[cfg(test)]
fn cells_to_runs(
    cells: &[TerminalCell],
    row: usize,
    selection: Option<SelectionRange>,
) -> Vec<RenderRun> {
    let cells = &cells[..significant_len(cells)];
    let mut runs = Vec::new();
    let mut idx = 0;
    while idx < cells.len() {
        let start = idx;
        let style = CellStyle::from_cell(&cells[idx], is_selected(selection, row, idx));
        let mut text = String::new();
        while idx < cells.len()
            && CellStyle::from_cell(&cells[idx], is_selected(selection, row, idx)) == style
        {
            push_cell_text(&mut text, &cells[idx]);
            idx += 1;
        }
        if !text.is_empty() {
            runs.push(RenderRun {
                start_column: start,
                columns: idx - start,
                text,
                style: style.into_render_style(),
            });
        }
    }
    runs
}

fn build_line_render(
    cells: &[TerminalCell],
    row: usize,
    selection: Option<SelectionRange>,
) -> LineRender {
    let cells = &cells[..significant_len(cells)];
    let mut text = String::new();
    let mut markup = String::new();
    let mut render_cells = Vec::new();
    let mut runs = Vec::new();
    let mut idx = 0;
    while idx < cells.len() {
        let start = idx;
        let style = CellStyle::from_cell(&cells[idx], is_selected(selection, row, idx));
        let mut run_text = String::new();
        markup.push_str(&style_open(&style));
        while idx < cells.len()
            && CellStyle::from_cell(&cells[idx], is_selected(selection, row, idx)) == style
        {
            push_cell_text(&mut text, &cells[idx]);
            push_cell_text(&mut run_text, &cells[idx]);
            push_escaped_cell(&mut markup, &cells[idx]);
            if !is_wide_spacer(&cells[idx]) {
                render_cells.push(RenderCell {
                    column: idx,
                    columns: if cells[idx].wide { 2 } else { 1 },
                    text: cells[idx].text.clone(),
                    style: CellStyle::from_cell(&cells[idx], is_selected(selection, row, idx))
                        .into_render_style(),
                });
            }
            idx += 1;
        }
        markup.push_str("</span>");
        if !run_text.is_empty() {
            runs.push(RenderRun {
                start_column: start,
                columns: idx - start,
                text: run_text,
                style: style.into_render_style(),
            });
        }
    }
    LineRender {
        text,
        markup,
        cells: render_cells,
        runs,
    }
}

fn is_selected(selection: Option<SelectionRange>, row: usize, column: usize) -> bool {
    selection
        .map(|range| range.contains(GridPoint { row, column }))
        .unwrap_or(false)
}

impl CellStyle {
    fn into_render_style(self) -> RenderStyle {
        RenderStyle {
            fg: self.fg,
            bg: if self.selected {
                Some("#264f78".to_string())
            } else {
                self.bg
            },
            bold: self.bold,
            italic: self.italic,
            underline: self.underline,
            strikeout: self.strikeout,
            selected: self.selected,
        }
    }
}

fn cells_to_text(cells: &[TerminalCell]) -> String {
    let mut out = String::new();
    for cell in &cells[..significant_len(cells)] {
        push_cell_text(&mut out, cell);
    }
    out
}

fn significant_len(cells: &[TerminalCell]) -> usize {
    cells
        .iter()
        .rposition(|cell| !is_trimmable_blank(cell))
        .map(|idx| idx + 1)
        .unwrap_or(0)
}

fn is_trimmable_blank(cell: &TerminalCell) -> bool {
    cell.text == " "
        && cell.fg.is_none()
        && cell.bg.is_none()
        && !cell.bold
        && !cell.italic
        && !cell.underline
        && !cell.inverse
        && !cell.strikeout
        && !cell.wide
        && !cell.wide_spacer
}

fn push_escaped_cell(out: &mut String, cell: &TerminalCell) {
    if is_wide_spacer(cell) {
        return;
    }
    for ch in cell.text.chars() {
        push_escaped_char(out, ch);
    }
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
    use crate::terminal_grid::{TerminalCell, TerminalColors, TerminalLineMetadata};

    fn cell(text: &str) -> TerminalCell {
        TerminalCell {
            text: text.to_string(),
            ..TerminalCell::blank()
        }
    }

    #[test]
    fn markup_escapes_text_cells() {
        let cells = [cell("<"), cell("&"), cell(">")];
        let markup = cells_to_markup(&cells, 0, None);
        assert!(markup.contains("&lt;&amp;&gt;"));
    }

    #[test]
    fn markup_trims_plain_blank_tail() {
        let cells = [
            cell("o"),
            cell("k"),
            TerminalCell::blank(),
            TerminalCell::blank(),
        ];
        let markup = cells_to_markup(&cells, 0, None);
        assert!(markup.contains(">ok</span>"));
        assert!(!markup.contains("ok  "));
    }

    #[test]
    fn markup_preserves_styled_blank_tail() {
        let mut styled_blank = TerminalCell::blank();
        styled_blank.bg = Some("#ff0000".to_string());
        let cells = [cell("x"), styled_blank];
        let markup = cells_to_markup(&cells, 0, None);
        assert!(markup.contains(">x</span>"));
        assert!(markup.contains("> </span>"));
    }

    #[test]
    fn markup_marks_selected_cells_without_changing_text() {
        let cells = [cell("a"), cell("b"), cell("c"), cell("d")];
        let selection = SelectionRange::new(
            GridPoint { row: 0, column: 1 },
            GridPoint { row: 0, column: 3 },
        );
        let markup = cells_to_markup(&cells, 0, Some(selection));
        assert!(markup.contains(">a</span>"));
        assert!(markup.contains("background=\"#264f78\">bc</span>"));
        assert!(markup.contains(">d</span>"));
    }

    #[test]
    fn runs_group_adjacent_cells_by_style_and_selection() {
        let cells = [cell("a"), cell("b"), cell("c"), cell("d")];
        let selection = SelectionRange::new(
            GridPoint { row: 0, column: 1 },
            GridPoint { row: 0, column: 3 },
        );
        let runs = cells_to_runs(&cells, 0, Some(selection));
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].start_column, 0);
        assert_eq!(runs[0].columns, 1);
        assert_eq!(runs[0].text, "a");
        assert!(!runs[0].style.selected);
        assert_eq!(runs[1].start_column, 1);
        assert_eq!(runs[1].columns, 2);
        assert_eq!(runs[1].text, "bc");
        assert!(runs[1].style.selected);
        assert_eq!(runs[2].start_column, 3);
        assert_eq!(runs[2].columns, 1);
        assert_eq!(runs[2].text, "d");
        assert!(!runs[2].style.selected);
    }

    #[test]
    fn text_and_markup_preserve_zerowidth_combining_marks() {
        let composed = cell("e\u{0301}");
        let cells = [cell("x"), composed, cell("y")];
        assert_eq!(cells_to_text(&cells), "xe\u{0301}y");
        let markup = cells_to_markup(&cells, 0, None);
        assert!(markup.contains("xe\u{0301}y"));
    }

    #[test]
    fn text_and_runs_skip_wide_char_spacer_cells() {
        let mut wide = cell("中");
        wide.wide = true;
        let mut spacer = TerminalCell::blank();
        spacer.wide_spacer = true;
        let cells = [cell("a"), wide, spacer, cell("b")];
        assert_eq!(cells_to_text(&cells), "a中b");
        let runs = cells_to_runs(&cells, 0, None);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "a中b");
        assert_eq!(runs[0].start_column, 0);
        assert_eq!(runs[0].columns, 4);
    }

    #[test]
    fn renderer_exports_structured_history_and_input_lines() {
        let content = RenderableContentOwned {
            lines: vec![
                vec![cell("o"), cell("l"), cell("d")],
                vec![cell("n"), cell("e"), cell("w")],
            ],
            line_metadata: vec![TerminalLineMetadata::default(); 2],
            cursor_line: 1,
            cursor_col: 3,
            cursor_visible: true,
            display_offset: 0,
            colors: TerminalColors::default(),
            mouse: MouseMode::default(),
        };
        let frame = Renderer::render_frame_with_selection(content, None);
        assert_eq!(frame.lines.len(), 2);
        assert_eq!(frame.lines[0].region, RenderRegion::History);
        assert_eq!(frame.lines[0].text, "old");
        assert_eq!(frame.lines[0].runs[0].text, "old");
        assert_eq!(frame.lines[1].region, RenderRegion::Input);
        assert_eq!(frame.lines[1].text, "new");
        assert_eq!(frame.lines[1].runs[0].start_column, 0);
        assert_eq!(frame.input_text, "new");
    }

    #[test]
    fn renderer_marks_soft_wrapped_cursor_line_as_single_input_region() {
        let content = RenderableContentOwned {
            lines: vec![
                vec![cell("o"), cell("l"), cell("d")],
                vec![cell("l"), cell("o"), cell("n"), cell("g")],
                vec![cell("c"), cell("m"), cell("d")],
            ],
            line_metadata: vec![
                TerminalLineMetadata::default(),
                TerminalLineMetadata {
                    wrapped: true,
                    ..TerminalLineMetadata::default()
                },
                TerminalLineMetadata {
                    wrap_continuation: true,
                    ..TerminalLineMetadata::default()
                },
            ],
            cursor_line: 2,
            cursor_col: 3,
            cursor_visible: true,
            display_offset: 0,
            colors: TerminalColors::default(),
            mouse: MouseMode::default(),
        };
        let frame = Renderer::render_frame_with_selection(content, None);
        assert_eq!(frame.lines[0].region, RenderRegion::History);
        assert_eq!(frame.lines[1].region, RenderRegion::Input);
        assert_eq!(frame.lines[2].region, RenderRegion::Input);
        assert_eq!(frame.input_text, "cmd");
        assert!(frame.input_markup.contains('\n'));
    }
}
