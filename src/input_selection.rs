use crate::backend::RenderableContentOwned;
use crate::input::{CursorDirection, CursorUnit};
use crate::mouse::MouseGridPosition;
use crate::selection::{
    GridPoint, SelectionRange, line_significant_len, viewport_range_for_display,
};
use crate::terminal_grid::{TerminalCell, TerminalSemanticPrompt};

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
    let rows = active_input_rows(content, row)?;
    let start_row = *rows.start();
    let end_row = *rows.end();
    let start = input_start_column(content.lines.get(start_row)?);
    let cursor = usize::try_from(content.cursor_col).ok();
    let end = active_input_end_column(content.lines.get(end_row)?, cursor);
    let range = SelectionRange::new(
        GridPoint {
            row: start_row,
            column: start,
        },
        GridPoint {
            row: end_row,
            column: end,
        },
    );
    (!range.is_empty()).then_some(range)
}

pub fn selection_within_active_input(
    content: &RenderableContentOwned,
    selection: SelectionRange,
) -> bool {
    active_input_line_range(content).is_some_and(|input| {
        point_le(input.start, selection.start) && point_le(selection.end, input.end)
    })
}

fn point_le(left: GridPoint, right: GridPoint) -> bool {
    left.row < right.row || (left.row == right.row && left.column <= right.column)
}

pub fn input_buffer_offset_for_position(
    content: &RenderableContentOwned,
    target: MouseGridPosition,
) -> Option<usize> {
    let cursor_row = usize::try_from(content.cursor_line).ok()?;
    let rows = active_input_rows(content, cursor_row)?;
    let absolute = input_absolute_column(
        content,
        rows.clone(),
        usize::from(target.row),
        usize::from(target.column),
    )?;
    let bounds = wrapped_input_bounds(content, rows)?;
    if absolute < bounds.start || absolute > bounds.end {
        return None;
    }
    Some(absolute - bounds.start)
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
    if let Some(target) = keyboard_cursor_target_in_active_input(content, direction, unit, current)
    {
        return Some(target);
    }
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

fn keyboard_cursor_target_in_active_input(
    content: &RenderableContentOwned,
    direction: CursorDirection,
    unit: CursorUnit,
    current: Option<GridPoint>,
) -> Option<MouseGridPosition> {
    let cursor_row = usize::try_from(content.cursor_line).ok()?;
    let rows = active_input_rows(content, cursor_row)?;
    let row = current
        .map(|point| point.row)
        .or_else(|| usize::try_from(content.cursor_line).ok())?;
    if !rows.contains(&row) {
        return None;
    }
    let cursor = current
        .map(|point| point.column)
        .or_else(|| usize::try_from(content.cursor_col).ok())?;
    let source = input_absolute_column(content, rows.clone(), row, cursor)?;
    let bounds = wrapped_input_bounds(content, rows.clone())?;
    let target = match (direction, unit) {
        (CursorDirection::Left, CursorUnit::Cell) => source.saturating_sub(1).max(bounds.start),
        (CursorDirection::Right, CursorUnit::Cell) => (source + 1).min(bounds.end),
        (CursorDirection::Left, CursorUnit::Word) => {
            previous_wrapped_word_boundary(content, rows.clone(), source, bounds.start)
        }
        (CursorDirection::Right, CursorUnit::Word) => {
            next_wrapped_word_boundary(content, rows.clone(), source, bounds.end)
        }
    };
    input_position_for_absolute_column(content, rows, target)
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
        return (!bytes.is_empty()).then_some(bytes);
    }
    if let Some(current) = current
        && let Some(bytes) = crate::interaction::cursor_movement_bytes_between_editable_input_points(
            content,
            MouseGridPosition {
                row: current.row.min(u16::MAX as usize) as u16,
                column: current.column.min(u16::MAX as usize) as u16,
            },
            target,
        )
    {
        return (!bytes.is_empty()).then_some(bytes);
    }
    crate::interaction::cursor_movement_bytes_for_content(content, target)
        .filter(|bytes| !bytes.is_empty())
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

fn active_input_rows(
    content: &RenderableContentOwned,
    cursor_row: usize,
) -> Option<std::ops::RangeInclusive<usize>> {
    if cursor_row >= content.lines.len() {
        return None;
    }
    if let Some(rows) = active_semantic_prompt_rows(content, cursor_row) {
        return Some(rows);
    }
    active_wrapped_rows(content, cursor_row)
}

fn active_semantic_prompt_rows(
    content: &RenderableContentOwned,
    cursor_row: usize,
) -> Option<std::ops::RangeInclusive<usize>> {
    let metadata = content
        .line_metadata
        .get(cursor_row)
        .copied()
        .unwrap_or_default();
    if metadata.semantic_prompt == TerminalSemanticPrompt::None {
        return None;
    }
    let mut start = cursor_row;
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
        if current.semantic_prompt == TerminalSemanticPrompt::Continuation
            && matches!(
                previous.semantic_prompt,
                TerminalSemanticPrompt::Prompt | TerminalSemanticPrompt::Continuation
            )
        {
            start -= 1;
        } else {
            break;
        }
    }
    if content
        .line_metadata
        .get(start)
        .copied()
        .unwrap_or_default()
        .semantic_prompt
        != TerminalSemanticPrompt::Prompt
    {
        return None;
    }
    let editable_start =
        semantic_prompt_editable_start(content, start, cursor_row).unwrap_or(start);
    Some(editable_start..=cursor_row)
}

fn semantic_prompt_editable_start(
    content: &RenderableContentOwned,
    start: usize,
    end: usize,
) -> Option<usize> {
    (start..=end).find(|row| {
        content
            .lines
            .get(*row)
            .is_some_and(|line| prompt_leader_before_input(line))
    })
}

fn prompt_leader_before_input(line: &[TerminalCell]) -> bool {
    let Some(separator) = line
        .iter()
        .take(line_significant_len(line))
        .position(|cell| cell.text == " ")
    else {
        return false;
    };
    if separator == 0 {
        return false;
    }
    line.get(separator - 1).is_some_and(|cell| {
        cell.text
            .chars()
            .any(|ch| matches!(ch, '❯' | '>' | '$' | '#' | '%' | 'λ' | '➜'))
    })
}

fn active_wrapped_rows(
    content: &RenderableContentOwned,
    cursor_row: usize,
) -> Option<std::ops::RangeInclusive<usize>> {
    if cursor_row >= content.lines.len() {
        return None;
    }
    let mut start = cursor_row;
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
    let mut end = cursor_row;
    while end + 1 < content.lines.len() {
        let current = content.line_metadata.get(end).copied().unwrap_or_default();
        let next = content
            .line_metadata
            .get(end + 1)
            .copied()
            .unwrap_or_default();
        if current.wrapped || next.wrap_continuation {
            end += 1;
        } else {
            break;
        }
    }
    Some(start..=end)
}

fn wrapped_input_bounds(
    content: &RenderableContentOwned,
    rows: std::ops::RangeInclusive<usize>,
) -> Option<std::ops::Range<usize>> {
    let start_row = *rows.start();
    let end_row = *rows.end();
    let start = input_absolute_column(
        content,
        rows.clone(),
        start_row,
        input_start_column(content.lines.get(start_row)?),
    )?;
    let end_column = if Some(end_row) == usize::try_from(content.cursor_line).ok() {
        active_input_end_column(
            content.lines.get(end_row)?,
            usize::try_from(content.cursor_col).ok(),
        )
    } else {
        line_significant_len(content.lines.get(end_row)?)
    };
    let end = input_absolute_column(content, rows, end_row, end_column)?;
    (end >= start).then_some(start..end)
}

fn input_absolute_column(
    content: &RenderableContentOwned,
    rows: std::ops::RangeInclusive<usize>,
    row: usize,
    column: usize,
) -> Option<usize> {
    if !rows.contains(&row) {
        return None;
    }
    let mut absolute = 0;
    for line_row in rows {
        let line_width = content.lines.get(line_row)?.len();
        if line_row == row {
            return Some(absolute + column.min(line_width));
        }
        absolute += line_width;
    }
    None
}

fn input_position_for_absolute_column(
    content: &RenderableContentOwned,
    rows: std::ops::RangeInclusive<usize>,
    absolute: usize,
) -> Option<MouseGridPosition> {
    let mut offset = 0;
    for row in rows {
        let line_width = content.lines.get(row)?.len();
        if absolute <= offset + line_width {
            return Some(MouseGridPosition {
                row: row.min(u16::MAX as usize) as u16,
                column: (absolute - offset).min(u16::MAX as usize) as u16,
            });
        }
        offset += line_width;
    }
    None
}

fn previous_wrapped_word_boundary(
    content: &RenderableContentOwned,
    rows: std::ops::RangeInclusive<usize>,
    cursor: usize,
    start: usize,
) -> usize {
    let mut column = cursor.min(wrapped_input_len(content, rows.clone()));
    while column > start
        && !wrapped_input_cell(content, rows.clone(), column - 1).is_some_and(is_word_cell)
    {
        column -= 1;
    }
    while column > start
        && wrapped_input_cell(content, rows.clone(), column - 1).is_some_and(is_word_cell)
    {
        column -= 1;
    }
    column
}

fn next_wrapped_word_boundary(
    content: &RenderableContentOwned,
    rows: std::ops::RangeInclusive<usize>,
    cursor: usize,
    end: usize,
) -> usize {
    let mut column = cursor.min(end);
    while column < end
        && !wrapped_input_cell(content, rows.clone(), column).is_some_and(is_word_cell)
    {
        column += 1;
    }
    while column < end
        && wrapped_input_cell(content, rows.clone(), column).is_some_and(is_word_cell)
    {
        column += 1;
    }
    column
}

fn wrapped_input_cell(
    content: &RenderableContentOwned,
    rows: std::ops::RangeInclusive<usize>,
    absolute: usize,
) -> Option<&TerminalCell> {
    let mut offset = 0;
    for row in rows {
        let line = content.lines.get(row)?;
        if absolute < offset + line.len() {
            return line.get(absolute - offset);
        }
        offset += line.len();
    }
    None
}

fn wrapped_input_len(
    content: &RenderableContentOwned,
    rows: std::ops::RangeInclusive<usize>,
) -> usize {
    rows.filter_map(|row| content.lines.get(row))
        .map(Vec::len)
        .sum()
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
    use crate::terminal_grid::{MouseMode, TerminalColors, TerminalContent, TerminalLineMetadata};

    fn content_with_cursor(text: &str, cursor_col: i32) -> TerminalContent {
        TerminalContent {
            lines: vec![
                text.chars()
                    .map(|ch| TerminalCell {
                        text: ch.to_string().into(),
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
            text: text.to_string().into(),
            fg: Some(fg.to_string()),
            ..TerminalCell::blank()
        }
    }

    fn wrapped_content_with_cursor(
        lines: &[&str],
        cursor_row: i32,
        cursor_col: i32,
    ) -> TerminalContent {
        let mut line_metadata = vec![TerminalLineMetadata::default(); lines.len()];
        for row in 0..lines.len().saturating_sub(1) {
            line_metadata[row].wrapped = true;
            line_metadata[row + 1].wrap_continuation = true;
        }
        TerminalContent {
            lines: lines
                .iter()
                .map(|line| {
                    line.chars()
                        .map(|ch| TerminalCell {
                            text: ch.to_string().into(),
                            ..TerminalCell::blank()
                        })
                        .collect()
                })
                .collect(),
            line_metadata,
            cursor_line: cursor_row,
            cursor_col,
            cursor_visible: true,
            display_offset: 0,
            colors: TerminalColors::default(),
            mouse: MouseMode::default(),
        }
    }

    fn semantic_content_with_cursor(
        lines: &[&str],
        cursor_row: i32,
        cursor_col: i32,
    ) -> TerminalContent {
        let mut line_metadata = vec![TerminalLineMetadata::default(); lines.len()];
        if let Some(first) = line_metadata.first_mut() {
            first.semantic_prompt = TerminalSemanticPrompt::Prompt;
        }
        for metadata in line_metadata.iter_mut().skip(1) {
            metadata.semantic_prompt = TerminalSemanticPrompt::Continuation;
        }
        TerminalContent {
            lines: lines
                .iter()
                .map(|line| {
                    line.chars()
                        .map(|ch| TerminalCell {
                            text: ch.to_string().into(),
                            ..TerminalCell::blank()
                        })
                        .collect()
                })
                .collect(),
            line_metadata,
            cursor_line: cursor_row,
            cursor_col,
            cursor_visible: true,
            display_offset: 0,
            colors: TerminalColors::default(),
            mouse: MouseMode::default(),
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
    fn active_input_range_spans_semantic_prompt_continuations() {
        let content = semantic_content_with_cursor(&["❯ import {", "  Foo,", "  Bar", "}"], 3, 1);

        assert_eq!(
            active_input_line_range(&content),
            Some(SelectionRange::new(
                GridPoint { row: 0, column: 2 },
                GridPoint { row: 3, column: 1 }
            ))
        );
    }

    #[test]
    fn active_input_range_skips_decorative_semantic_prompt_header() {
        let content = semantic_content_with_cursor(
            &["~/Documents/Projects/Chelotype on main", "❯ φiв"],
            1,
            5,
        );

        assert_eq!(
            active_input_line_range(&content),
            Some(SelectionRange::new(
                GridPoint { row: 1, column: 2 },
                GridPoint { row: 1, column: 5 }
            ))
        );
    }

    #[test]
    fn selection_within_active_input_rejects_output_tail() {
        let mut content =
            semantic_content_with_cursor(&["❯ import {", "  Foo,", "  Bar", "}"], 3, 1);
        content.lines.push(
            "OUTPUT"
                .chars()
                .map(|ch| TerminalCell {
                    text: ch.to_string().into(),
                    ..TerminalCell::blank()
                })
                .collect(),
        );
        content.line_metadata.push(TerminalLineMetadata::default());

        assert!(selection_within_active_input(
            &content,
            SelectionRange::new(
                GridPoint { row: 0, column: 2 },
                GridPoint { row: 3, column: 1 }
            )
        ));
        assert!(!selection_within_active_input(
            &content,
            SelectionRange::new(
                GridPoint { row: 0, column: 2 },
                GridPoint { row: 4, column: 3 }
            )
        ));
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
        assert_eq!(
            keyboard_cursor_bytes(&content, None, MouseGridPosition { row: 0, column: 7 }),
            None
        );
    }

    #[test]
    fn input_buffer_offset_maps_visual_position_inside_wrapped_input() {
        let content = wrapped_content_with_cursor(&["❯ abc", "def"], 1, 3);

        assert_eq!(
            input_buffer_offset_for_position(&content, MouseGridPosition { row: 0, column: 2 }),
            Some(0)
        );
        assert_eq!(
            input_buffer_offset_for_position(&content, MouseGridPosition { row: 0, column: 4 }),
            Some(2)
        );
        assert_eq!(
            input_buffer_offset_for_position(&content, MouseGridPosition { row: 1, column: 0 }),
            Some(3)
        );
        assert_eq!(
            input_buffer_offset_for_position(&content, MouseGridPosition { row: 1, column: 3 }),
            Some(6)
        );
    }

    #[test]
    fn input_buffer_offset_maps_visual_position_inside_semantic_input() {
        let content = semantic_content_with_cursor(&["❯ abc", "def"], 1, 3);

        assert_eq!(
            input_buffer_offset_for_position(&content, MouseGridPosition { row: 0, column: 2 }),
            Some(0)
        );
        assert_eq!(
            input_buffer_offset_for_position(&content, MouseGridPosition { row: 0, column: 5 }),
            Some(3)
        );
        assert_eq!(
            input_buffer_offset_for_position(&content, MouseGridPosition { row: 1, column: 0 }),
            Some(3)
        );
        assert_eq!(
            input_buffer_offset_for_position(&content, MouseGridPosition { row: 1, column: 3 }),
            Some(6)
        );
    }

    #[test]
    fn directed_shift_left_selection_crosses_semantic_prompt_rows() {
        let content = semantic_content_with_cursor(&["❯ abc ", "def"], 1, 0);
        let target =
            keyboard_cursor_target(&content, CursorDirection::Left, CursorUnit::Cell, None)
                .expect("left target across semantic input rows");
        let directed = directed_selection_for_target(&content, target, None, None)
            .expect("directed semantic selection");

        assert_eq!(target, MouseGridPosition { row: 0, column: 5 });
        assert_eq!(
            directed.range(),
            Some(SelectionRange::new(
                GridPoint { row: 0, column: 5 },
                GridPoint { row: 1, column: 0 }
            ))
        );
    }

    #[test]
    fn input_buffer_offset_rejects_prompt_and_output_positions() {
        let content = wrapped_content_with_cursor(&["❯ abc", "def"], 1, 3);

        assert_eq!(
            input_buffer_offset_for_position(&content, MouseGridPosition { row: 0, column: 1 }),
            None
        );
        assert_eq!(
            input_buffer_offset_for_position(&content, MouseGridPosition { row: 2, column: 0 }),
            None
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

    #[test]
    fn shift_left_target_crosses_soft_wrapped_input_row_boundary() {
        let content = wrapped_content_with_cursor(&["❯ abc", "def"], 1, 0);

        assert_eq!(
            keyboard_cursor_target(&content, CursorDirection::Left, CursorUnit::Cell, None),
            Some(MouseGridPosition { row: 0, column: 4 })
        );
        assert_eq!(
            keyboard_cursor_bytes(&content, None, MouseGridPosition { row: 0, column: 4 })
                .as_deref(),
            Some(&b"\x1b[D"[..])
        );
    }

    #[test]
    fn directed_shift_left_selection_crosses_soft_wrapped_input_row_boundary() {
        let content = wrapped_content_with_cursor(&["❯ abc", "def"], 1, 0);
        let target =
            keyboard_cursor_target(&content, CursorDirection::Left, CursorUnit::Cell, None)
                .expect("left target across soft wrap");
        let directed = directed_selection_for_target(&content, target, None, None)
            .expect("directed selection across soft wrap");

        assert_eq!(
            directed.range(),
            Some(SelectionRange::new(
                GridPoint { row: 0, column: 4 },
                GridPoint { row: 1, column: 0 }
            ))
        );
        assert_eq!(
            keyboard_cursor_bytes(&content, None, target).as_deref(),
            Some(&b"\x1b[D"[..])
        );
    }

    #[test]
    fn directed_ctrl_shift_left_selection_crosses_soft_wrapped_input_row_boundary() {
        let content = wrapped_content_with_cursor(&["❯ abc ", "def ghi"], 1, 0);
        let target =
            keyboard_cursor_target(&content, CursorDirection::Left, CursorUnit::Word, None)
                .expect("word-left target across soft wrap");
        let directed = directed_selection_for_target(&content, target, None, None)
            .expect("directed word selection across soft wrap");

        assert_eq!(target, MouseGridPosition { row: 0, column: 2 });
        assert_eq!(
            directed.range(),
            Some(SelectionRange::new(
                GridPoint { row: 0, column: 2 },
                GridPoint { row: 1, column: 0 }
            ))
        );
        assert_eq!(
            keyboard_cursor_bytes(&content, None, target).as_deref(),
            Some(&b"\x1b[D\x1b[D\x1b[D\x1b[D"[..])
        );
    }

    #[test]
    fn directed_selection_cursor_bytes_cross_soft_wrap_from_logical_focus() {
        let content = wrapped_content_with_cursor(&["❯ abc", "def"], 1, 0);

        assert_eq!(
            keyboard_cursor_bytes(
                &content,
                Some(GridPoint { row: 0, column: 4 }),
                MouseGridPosition { row: 1, column: 0 },
            )
            .as_deref(),
            Some(&b"\x1b[C"[..])
        );
    }
}
