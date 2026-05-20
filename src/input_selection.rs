use crate::backend::RenderableContentOwned;
use crate::input::{CursorDirection, CursorUnit};
use crate::mouse::MouseGridPosition;
use crate::selection::{
    GridPoint, SelectionRange, line_significant_len, viewport_range_for_display,
};
use crate::terminal_grid::TerminalCell;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectedSelectionRange {
    pub anchor: GridPoint,
    pub focus: GridPoint,
}

impl DirectedSelectionRange {
    pub fn viewport_focus(self, display_offset: usize, viewport_rows: usize) -> Option<GridPoint> {
        viewport_point_for_display(self.focus, display_offset, viewport_rows)
    }

    pub fn range(self) -> Option<SelectionRange> {
        (self.anchor != self.focus).then_some(SelectionRange::new(self.anchor, self.focus))
    }
}

pub fn input_start_column(line: &[TerminalCell]) -> usize {
    let significant = line_significant_len(line);
    line.iter()
        .take(significant)
        .position(|cell| cell.text == " ")
        .map(|column| column + 1)
        .unwrap_or(0)
}

pub fn active_input_line_range(content: &RenderableContentOwned) -> Option<SelectionRange> {
    let row = usize::try_from(content.cursor_line).ok()?;
    let line = content.lines.get(row)?;
    let start = input_start_column(line);
    let cursor = usize::try_from(content.cursor_col).ok();
    let end = active_input_end_column(line, cursor);
    (end > start).then_some(SelectionRange::new(
        GridPoint { row, column: start },
        GridPoint { row, column: end },
    ))
}

pub fn active_cursor_point(content: &RenderableContentOwned) -> Option<GridPoint> {
    Some(GridPoint {
        row: usize::try_from(content.cursor_line).ok()?,
        column: usize::try_from(content.cursor_col).ok()?,
    })
}

pub fn keyboard_cursor_target(
    content: &RenderableContentOwned,
    direction: CursorDirection,
    unit: CursorUnit,
    current: Option<GridPoint>,
) -> Option<MouseGridPosition> {
    let row = current
        .map(|point| point.row)
        .or_else(|| usize::try_from(content.cursor_line).ok())?;
    let cursor = current
        .map(|point| point.column)
        .or_else(|| usize::try_from(content.cursor_col).ok())?;
    let line = content.lines.get(row)?;
    let start = input_start_column(line);
    let end = if Some(row) == usize::try_from(content.cursor_line).ok() {
        active_input_end_column(line, usize::try_from(content.cursor_col).ok())
    } else {
        line_significant_len(line)
    };
    let column = match (direction, unit) {
        (CursorDirection::Left, CursorUnit::Cell) => cursor.saturating_sub(1).max(start),
        (CursorDirection::Right, CursorUnit::Cell) => (cursor + 1).min(end),
        (CursorDirection::Left, CursorUnit::Word) => previous_word_boundary(line, cursor, start),
        (CursorDirection::Right, CursorUnit::Word) => next_word_boundary(line, cursor, end),
    };
    Some(MouseGridPosition {
        row: row.min(u16::MAX as usize) as u16,
        column: column.min(u16::MAX as usize) as u16,
    })
}

pub fn keyboard_selection_collapse_target(
    selection: Option<SelectionRange>,
    display_offset: usize,
    viewport_rows: usize,
    direction: CursorDirection,
) -> Option<GridPoint> {
    let range = viewport_range_for_display(selection?, display_offset, viewport_rows)?;
    Some(match direction {
        CursorDirection::Left => range.start,
        CursorDirection::Right => range.end,
    })
}

pub fn keyboard_selection_anchor(
    selection: Option<SelectionRange>,
    cursor: GridPoint,
) -> Option<GridPoint> {
    let range = selection?;
    if cursor == range.start {
        Some(range.end)
    } else if cursor == range.end {
        Some(range.start)
    } else {
        None
    }
}

pub fn directed_selection_for_target(
    content: &RenderableContentOwned,
    target: MouseGridPosition,
    existing_directed: Option<DirectedSelectionRange>,
    existing_selection: Option<SelectionRange>,
) -> Option<DirectedSelectionRange> {
    let cursor_row = usize::try_from(content.cursor_line).ok()?;
    let cursor_column = usize::try_from(content.cursor_col).ok()?;
    let cursor = GridPoint {
        row: cursor_row + content.display_offset,
        column: cursor_column,
    };
    let target = GridPoint {
        row: usize::from(target.row) + content.display_offset,
        column: usize::from(target.column),
    };
    let anchor = existing_directed
        .map(|range| range.anchor)
        .or_else(|| keyboard_selection_anchor(existing_selection, cursor))
        .unwrap_or(cursor);
    Some(DirectedSelectionRange {
        anchor,
        focus: target,
    })
}

pub fn keyboard_cursor_bytes(
    content: &RenderableContentOwned,
    current: Option<GridPoint>,
    target: MouseGridPosition,
) -> Option<Vec<u8>> {
    if let Some(current) = current
        && let Some(bytes) = cursor_movement_bytes_between_points(
            GridPoint {
                row: current.row + content.display_offset,
                column: current.column,
            },
            GridPoint {
                row: usize::from(target.row) + content.display_offset,
                column: usize::from(target.column),
            },
        )
    {
        return Some(bytes);
    }
    crate::interaction::cursor_movement_bytes_for_content(content, target)
}

pub fn cursor_movement_bytes_between_points(
    source: GridPoint,
    target: GridPoint,
) -> Option<Vec<u8>> {
    if source.row != target.row {
        return None;
    }
    let delta = target.column as i32 - source.column as i32;
    let mut bytes = Vec::new();
    let step = if delta < 0 {
        b"\x1b[D".as_slice()
    } else {
        b"\x1b[C".as_slice()
    };
    for _ in 0..delta.unsigned_abs() {
        bytes.extend_from_slice(step);
    }
    Some(bytes)
}

fn viewport_point_for_display(
    point: GridPoint,
    display_offset: usize,
    viewport_rows: usize,
) -> Option<GridPoint> {
    let viewport_end = display_offset + viewport_rows;
    if point.row < display_offset || point.row >= viewport_end {
        return None;
    }
    Some(GridPoint {
        row: point.row - display_offset,
        column: point.column,
    })
}

fn previous_word_boundary(line: &[TerminalCell], cursor: usize, start: usize) -> usize {
    let mut column = cursor.min(line_significant_len(line));
    while column > start && !is_word_cell(&line[column - 1]) {
        column -= 1;
    }
    while column > start && is_word_cell(&line[column - 1]) {
        column -= 1;
    }
    column
}

fn next_word_boundary(line: &[TerminalCell], cursor: usize, end: usize) -> usize {
    let mut column = cursor.min(end);
    while column < end && !is_word_cell(&line[column]) {
        column += 1;
    }
    while column < end && is_word_cell(&line[column]) {
        column += 1;
    }
    column
}

fn is_word_cell(cell: &TerminalCell) -> bool {
    cell.text
        .chars()
        .any(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

fn active_input_end_column(line: &[TerminalCell], cursor: Option<usize>) -> usize {
    let significant = line_significant_len(line);
    let Some(cursor) = cursor.filter(|cursor| *cursor < significant) else {
        return significant;
    };
    if line[cursor..significant]
        .iter()
        .all(is_fish_autosuggestion_cell)
    {
        cursor
    } else {
        significant
    }
}

fn is_fish_autosuggestion_cell(cell: &TerminalCell) -> bool {
    cell.fg
        .as_deref()
        .and_then(parse_hex_rgb)
        .is_some_and(|(red, green, blue)| {
            let max_channel = red.max(green).max(blue);
            let min_channel = red.min(green).min(blue);
            max_channel <= 160 && max_channel.saturating_sub(min_channel) <= 48
        })
}

fn parse_hex_rgb(value: &str) -> Option<(u8, u8, u8)> {
    let value = value.strip_prefix('#')?;
    (value.len() == 6).then_some(())?;
    Some((
        u8::from_str_radix(&value[0..2], 16).ok()?,
        u8::from_str_radix(&value[2..4], 16).ok()?,
        u8::from_str_radix(&value[4..6], 16).ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_grid::{MouseMode, TerminalColors, TerminalContent};

    fn content_with_cursor(text: &str, cursor_col: i32) -> TerminalContent {
        TerminalContent {
            lines: vec![
                text.chars()
                    .map(|ch| TerminalCell {
                        text: ch.to_string(),
                        ..TerminalCell::blank()
                    })
                    .collect(),
            ],
            line_metadata: Vec::new(),
            cursor_line: 0,
            cursor_col,
            cursor_visible: true,
            display_offset: 0,
            colors: TerminalColors::default(),
            mouse: MouseMode::default(),
        }
    }

    fn colored_cell(text: &str, fg: &str) -> TerminalCell {
        TerminalCell {
            text: text.to_string(),
            fg: Some(fg.to_string()),
            ..TerminalCell::blank()
        }
    }

    #[test]
    fn active_input_range_stops_before_fish_autosuggestion_tail() {
        let mut content = content_with_cursor("", 7);
        content.lines = vec![vec![
            colored_cell("❯", "#b5bd68"),
            TerminalCell::blank(),
            colored_cell("P", "#ff6b81"),
            colored_cell("A", "#ff6b81"),
            colored_cell("S", "#ff6b81"),
            colored_cell("T", "#ff6b81"),
            colored_cell("E", "#ff6b81"),
            colored_cell(" ", "#585b70"),
            TerminalCell::blank(),
        ]];

        assert_eq!(
            active_input_line_range(&content),
            Some(SelectionRange::new(
                GridPoint { row: 0, column: 2 },
                GridPoint { row: 0, column: 7 }
            ))
        );
    }

    #[test]
    fn cursor_right_does_not_move_into_fish_autosuggestion_tail() {
        let mut content = content_with_cursor("", 7);
        content.lines = vec![vec![
            colored_cell("❯", "#b5bd68"),
            TerminalCell::blank(),
            colored_cell("P", "#ff6b81"),
            colored_cell("A", "#ff6b81"),
            colored_cell("S", "#ff6b81"),
            colored_cell("T", "#ff6b81"),
            colored_cell("E", "#ff6b81"),
            colored_cell(" ", "#585b70"),
            TerminalCell::blank(),
        ]];

        assert_eq!(
            keyboard_cursor_target(&content, CursorDirection::Right, CursorUnit::Cell, None),
            Some(MouseGridPosition { row: 0, column: 7 })
        );
    }

    #[test]
    fn keyboard_selection_anchor_stays_fixed_while_focus_moves_both_directions() {
        let range = SelectionRange::new(
            GridPoint { row: 4, column: 5 },
            GridPoint { row: 4, column: 6 },
        );

        assert_eq!(
            keyboard_selection_anchor(Some(range), GridPoint { row: 4, column: 5 }),
            Some(GridPoint { row: 4, column: 6 })
        );
        assert_eq!(
            keyboard_selection_anchor(Some(range), GridPoint { row: 4, column: 6 }),
            Some(GridPoint { row: 4, column: 5 })
        );
    }

    #[test]
    fn keyboard_selection_collapse_uses_requested_selection_boundary() {
        let range = SelectionRange::new(
            GridPoint { row: 4, column: 5 },
            GridPoint { row: 4, column: 8 },
        );

        assert_eq!(
            keyboard_selection_collapse_target(Some(range), 0, 10, CursorDirection::Left),
            Some(GridPoint { row: 4, column: 5 })
        );
        assert_eq!(
            keyboard_selection_collapse_target(Some(range), 0, 10, CursorDirection::Right),
            Some(GridPoint { row: 4, column: 8 })
        );
    }

    #[test]
    fn directed_keyboard_selection_shrinks_back_to_empty_without_stale_terminal_cursor() {
        let content = content_with_cursor("❯ abcdef", 8);

        let initial = directed_selection_for_target(
            &content,
            MouseGridPosition { column: 7, row: 0 },
            None,
            None,
        )
        .expect("initial selection");
        assert_eq!(
            initial.range(),
            Some(SelectionRange::new(
                GridPoint { row: 0, column: 7 },
                GridPoint { row: 0, column: 8 }
            ))
        );

        let current = initial.viewport_focus(0, content.lines.len());
        let target =
            keyboard_cursor_target(&content, CursorDirection::Right, CursorUnit::Cell, current)
                .expect("right target from directed focus");
        let next = directed_selection_for_target(&content, target, Some(initial), initial.range())
            .expect("next selection");

        assert_eq!(next.range(), None);
    }

    #[test]
    fn directed_ctrl_shift_word_selection_starts_from_current_position() {
        let content = content_with_cursor("❯ abcde", 4);

        let target = keyboard_cursor_target(
            &content,
            CursorDirection::Right,
            CursorUnit::Word,
            Some(GridPoint { row: 0, column: 4 }),
        )
        .expect("word target from middle of word");
        let directed = directed_selection_for_target(&content, target, None, None)
            .expect("directed selection");

        assert_eq!(
            directed.range(),
            Some(SelectionRange::new(
                GridPoint { row: 0, column: 4 },
                GridPoint { row: 0, column: 7 }
            ))
        );
    }

    #[test]
    fn directed_cursor_bytes_use_logical_focus_instead_of_stale_terminal_cursor() {
        let content = content_with_cursor("❯ abcdef", 8);
        let bytes = keyboard_cursor_bytes(
            &content,
            Some(GridPoint { row: 0, column: 5 }),
            MouseGridPosition { row: 0, column: 8 },
        )
        .expect("movement bytes");

        assert_eq!(bytes, b"\x1b[C\x1b[C\x1b[C");
    }
}
