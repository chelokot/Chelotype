use crate::mouse::MouseGridPosition;
use alacritty_terminal::term::cell::Cell;
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

pub fn selected_text(lines: &[Vec<Cell>], range: SelectionRange) -> String {
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
            range.end.column
        } else {
            significant_len(line)
        };
        for column in start_column..end_column.min(line.len()) {
            out.push(line[column].c);
        }
        if row != range.end.row && row + 1 < lines.len() {
            out.push('\n');
        }
    }
    out
}

fn point_le(left: GridPoint, right: GridPoint) -> bool {
    left.row < right.row || left.row == right.row && left.column <= right.column
}

fn point_lt(left: GridPoint, right: GridPoint) -> bool {
    left.row < right.row || left.row == right.row && left.column < right.column
}

fn significant_len(line: &[Cell]) -> usize {
    line.iter()
        .rposition(|cell| cell.c != ' ' || cell.zerowidth().is_some() || !cell.flags.is_empty())
        .map(|idx| idx + 1)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> Vec<Cell> {
        text.chars()
            .map(|ch| Cell {
                c: ch,
                ..Cell::default()
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
        padded.extend([Cell::default(), Cell::default()]);
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
}
