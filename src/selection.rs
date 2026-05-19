use crate::cell_text::push_cell_text;
use crate::mouse::MouseGridPosition;
use crate::terminal_grid::TerminalCell;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct GridPoint {
    pub row: usize,
    pub column: usize,
}

impl GridPoint {
    pub fn next_cell(self) -> Self {
        Self {
            row: self.row,
            column: self.column + 1,
        }
    }
}

impl From<MouseGridPosition> for GridPoint {
    fn from(position: MouseGridPosition) -> Self {
        Self {
            row: position.row as usize,
            column: position.column as usize,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct SelectionRange {
    pub start: GridPoint,
    pub end: GridPoint,
}

impl SelectionRange {
    pub fn new(start: GridPoint, end: GridPoint) -> Self {
        if point_le(start, end) {
            Self { start, end }
        } else {
            Self {
                start: end,
                end: start,
            }
        }
    }

    pub fn between_cells(anchor: GridPoint, focus: GridPoint) -> Self {
        if point_le(anchor, focus) {
            Self::new(anchor, focus.next_cell())
        } else {
            Self::new(focus, anchor.next_cell())
        }
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end
    }

    pub fn contains(self, point: GridPoint) -> bool {
        point_le(self.start, point) && point_lt(point, self.end)
    }
}

pub fn selected_text(lines: &[Vec<TerminalCell>], range: SelectionRange) -> String {
    if range.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for row in range.start.row..=range.end.row {
        let Some(line) = lines.get(row) else {
            break;
        };
        let start_column = if row == range.start.row {
            range.start.column
        } else {
            0
        };
        let end_column = if row == range.end.row {
            range.end.column.min(significant_len(line))
        } else {
            significant_len(line)
        };
        for cell in line
            .iter()
            .take(end_column.min(line.len()))
            .skip(start_column)
        {
            push_cell_text(&mut out, cell);
        }
        if row != range.end.row && row + 1 < lines.len() {
            out.push('\n');
        }
    }
    out
}

pub fn line_range(lines: &[Vec<TerminalCell>], row: usize) -> Option<SelectionRange> {
    let line = lines.get(row)?;
    let end = significant_len(line);
    (end > 0).then_some(SelectionRange::new(
        GridPoint { row, column: 0 },
        GridPoint { row, column: end },
    ))
}

pub fn word_range_at(
    lines: &[Vec<TerminalCell>],
    row: usize,
    column: usize,
) -> Option<SelectionRange> {
    let line = lines.get(row)?;
    let end = significant_len(line);
    if column >= end || !is_word_cell(line.get(column)?) {
        return None;
    }
    let mut start = column;
    while start > 0 && is_word_cell(&line[start - 1]) {
        start -= 1;
    }
    let mut word_end = column + 1;
    while word_end < end && is_word_cell(&line[word_end]) {
        word_end += 1;
    }
    Some(SelectionRange::new(
        GridPoint { row, column: start },
        GridPoint {
            row,
            column: word_end,
        },
    ))
}

pub fn find_text_range(lines: &[Vec<TerminalCell>], text: &str) -> Option<SelectionRange> {
    if text.is_empty() || text.contains('\n') {
        return None;
    }
    lines
        .iter()
        .enumerate()
        .find_map(|(row, line)| find_text_in_line(line, text).map(|(start, end)| (row, start, end)))
        .map(|(row, start, end)| {
            SelectionRange::new(
                GridPoint { row, column: start },
                GridPoint { row, column: end },
            )
        })
}

pub fn line_significant_len(line: &[TerminalCell]) -> usize {
    significant_len(line)
}

pub fn anchor_range_to_display(range: SelectionRange, display_offset: usize) -> SelectionRange {
    SelectionRange {
        start: GridPoint {
            row: range.start.row + display_offset,
            column: range.start.column,
        },
        end: GridPoint {
            row: range.end.row + display_offset,
            column: range.end.column,
        },
    }
}

pub fn viewport_range_for_display(
    range: SelectionRange,
    display_offset: usize,
    viewport_rows: usize,
) -> Option<SelectionRange> {
    let viewport_end = display_offset + viewport_rows;
    if viewport_rows == 0 || range.start.row >= viewport_end || range.end.row < display_offset {
        return None;
    }
    let start = if range.start.row < display_offset {
        GridPoint { row: 0, column: 0 }
    } else {
        GridPoint {
            row: range.start.row - display_offset,
            column: range.start.column,
        }
    };
    let end = if range.end.row > viewport_end {
        GridPoint {
            row: viewport_rows,
            column: 0,
        }
    } else {
        GridPoint {
            row: range.end.row - display_offset,
            column: range.end.column,
        }
    };
    let visible = SelectionRange::new(start, end);
    if visible.is_empty() {
        None
    } else {
        Some(visible)
    }
}

fn point_le(left: GridPoint, right: GridPoint) -> bool {
    left.row < right.row || left.row == right.row && left.column <= right.column
}

fn point_lt(left: GridPoint, right: GridPoint) -> bool {
    left.row < right.row || left.row == right.row && left.column < right.column
}

fn significant_len(line: &[TerminalCell]) -> usize {
    line.iter()
        .rposition(|cell| {
            cell.text != " "
                || cell.fg.is_some()
                || cell.bg.is_some()
                || cell.bold
                || cell.italic
                || cell.underline
                || cell.inverse
                || cell.strikeout
                || cell.wide
                || cell.wide_spacer
        })
        .map(|idx| idx + 1)
        .unwrap_or(0)
}

fn find_text_in_line(line: &[TerminalCell], needle: &str) -> Option<(usize, usize)> {
    let mut haystack = String::new();
    let mut spans = Vec::new();
    for (column, cell) in line.iter().take(significant_len(line)).enumerate() {
        let start_byte = haystack.len();
        push_cell_text(&mut haystack, cell);
        let end_byte = haystack.len();
        if start_byte != end_byte {
            spans.push((start_byte, end_byte, column, column + 1));
        }
    }
    let start_byte = haystack.find(needle)?;
    let end_byte = start_byte + needle.len();
    let start_column = spans
        .iter()
        .find_map(|(start, _, column, _)| (*start == start_byte).then_some(*column))?;
    let end_column = spans
        .iter()
        .find_map(|(_, end, _, next_column)| (*end == end_byte).then_some(*next_column))?;
    Some((start_column, end_column))
}

fn is_word_cell(cell: &TerminalCell) -> bool {
    cell.text
        .chars()
        .any(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> Vec<TerminalCell> {
        text.chars()
            .map(|ch| TerminalCell {
                text: ch.to_string(),
                ..TerminalCell::blank()
            })
            .collect()
    }

    #[test]
    fn normalizes_half_open_range() {
        let range = SelectionRange::new(
            GridPoint { row: 3, column: 4 },
            GridPoint { row: 1, column: 2 },
        );
        assert_eq!(range.start, GridPoint { row: 1, column: 2 });
        assert_eq!(range.end, GridPoint { row: 3, column: 4 });
    }

    #[test]
    fn builds_cell_inclusive_drag_range_as_half_open() {
        let range = SelectionRange::between_cells(
            GridPoint { row: 1, column: 2 },
            GridPoint { row: 1, column: 4 },
        );
        assert_eq!(range.start, GridPoint { row: 1, column: 2 });
        assert_eq!(range.end, GridPoint { row: 1, column: 5 });
    }

    #[test]
    fn normalizes_reverse_drag_and_keeps_anchor_cell() {
        let range = SelectionRange::between_cells(
            GridPoint { row: 3, column: 4 },
            GridPoint { row: 1, column: 2 },
        );
        assert_eq!(range.start, GridPoint { row: 1, column: 2 });
        assert_eq!(range.end, GridPoint { row: 3, column: 5 });
    }

    #[test]
    fn detects_contained_points() {
        let range = SelectionRange::new(
            GridPoint { row: 1, column: 2 },
            GridPoint { row: 2, column: 3 },
        );
        assert!(range.contains(GridPoint { row: 1, column: 2 }));
        assert!(range.contains(GridPoint { row: 2, column: 2 }));
        assert!(!range.contains(GridPoint { row: 2, column: 3 }));
        assert!(!range.contains(GridPoint { row: 0, column: 9 }));
        assert!(!range.contains(GridPoint { row: 2, column: 4 }));
    }

    #[test]
    fn extracts_single_line_text() {
        let lines = [line("abcdef")];
        let range = SelectionRange::new(
            GridPoint { row: 0, column: 1 },
            GridPoint { row: 0, column: 4 },
        );
        assert_eq!(selected_text(&lines, range), "bcd");
    }

    #[test]
    fn extracts_multiline_text() {
        let lines = [line("abcdef"), line("ghijkl"), line("mnopqr")];
        let range = SelectionRange::new(
            GridPoint { row: 0, column: 2 },
            GridPoint { row: 2, column: 3 },
        );
        assert_eq!(selected_text(&lines, range), "cdef\nghijkl\nmno");
    }

    #[test]
    fn trims_plain_blank_tail_on_full_middle_lines() {
        let mut padded = line("gh");
        padded.extend([TerminalCell::blank(), TerminalCell::blank()]);
        let lines = [line("abcdef"), padded, line("mnopqr")];
        let range = SelectionRange::new(
            GridPoint { row: 0, column: 5 },
            GridPoint { row: 2, column: 1 },
        );
        assert_eq!(selected_text(&lines, range), "f\ngh\nm");
    }

    #[test]
    fn returns_empty_text_for_empty_range() {
        let lines = [line("abcdef")];
        let range = SelectionRange::new(
            GridPoint { row: 0, column: 2 },
            GridPoint { row: 0, column: 2 },
        );
        assert_eq!(selected_text(&lines, range), "");
    }

    #[test]
    fn selected_text_preserves_grapheme_cells_and_skips_wide_spacers() {
        let composed = TerminalCell {
            text: "e\u{0301}".to_string(),
            ..TerminalCell::blank()
        };
        let mut wide = TerminalCell {
            text: "中".to_string(),
            ..TerminalCell::blank()
        };
        wide.wide = true;
        let mut spacer = TerminalCell::blank();
        spacer.wide_spacer = true;
        let lines = [vec![
            line("a")[0].clone(),
            composed,
            wide,
            spacer,
            line("b")[0].clone(),
        ]];
        let range = SelectionRange::new(
            GridPoint { row: 0, column: 0 },
            GridPoint { row: 0, column: 5 },
        );
        assert_eq!(selected_text(&lines, range), "ae\u{0301}中b");
    }

    #[test]
    fn projects_display_anchored_selection_into_scrolled_viewport() {
        let range = SelectionRange::new(
            GridPoint { row: 12, column: 3 },
            GridPoint { row: 14, column: 5 },
        );
        assert_eq!(
            viewport_range_for_display(range, 10, 6),
            Some(SelectionRange::new(
                GridPoint { row: 2, column: 3 },
                GridPoint { row: 4, column: 5 },
            ))
        );
        assert_eq!(
            viewport_range_for_display(range, 13, 6),
            Some(SelectionRange::new(
                GridPoint { row: 0, column: 0 },
                GridPoint { row: 1, column: 5 },
            ))
        );
        assert_eq!(viewport_range_for_display(range, 20, 6), None);
    }

    #[test]
    fn selects_word_and_line_ranges_from_grid_cells() {
        let lines = [line("one two-three.")];
        assert_eq!(
            word_range_at(&lines, 0, 5),
            Some(SelectionRange::new(
                GridPoint { row: 0, column: 4 },
                GridPoint { row: 0, column: 14 },
            ))
        );
        assert_eq!(word_range_at(&lines, 0, 3), None);
        assert_eq!(
            line_range(&lines, 0),
            Some(SelectionRange::new(
                GridPoint { row: 0, column: 0 },
                GridPoint { row: 0, column: 14 },
            ))
        );
    }

    #[test]
    fn finds_text_range_on_cell_boundaries() {
        let lines = [line("prefix KEEP_TOKEN suffix")];

        assert_eq!(
            find_text_range(&lines, "KEEP_TOKEN"),
            Some(SelectionRange::new(
                GridPoint { row: 0, column: 7 },
                GridPoint { row: 0, column: 17 },
            ))
        );
    }

    #[test]
    fn finds_text_range_after_wrapped_reflow_changes_row_and_column() {
        let lines = [line(
            "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxKEEP_TOKEN",
        )];

        assert_eq!(
            find_text_range(&lines, "KEEP_TOKEN"),
            Some(SelectionRange::new(
                GridPoint { row: 0, column: 50 },
                GridPoint { row: 0, column: 60 },
            ))
        );
    }

    #[test]
    fn text_range_search_ignores_empty_and_multiline_needles() {
        let lines = [line("first"), line("second")];

        assert_eq!(find_text_range(&lines, ""), None);
        assert_eq!(find_text_range(&lines, "first\nsecond"), None);
    }
}
