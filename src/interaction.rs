use crate::mouse::{
    MouseButton, MouseGridPosition, sgr_drag_bytes, sgr_press_bytes, sgr_release_bytes,
};
use crate::selection::{GridPoint, SelectionRange};
use crate::terminal_grid::{MouseMode, TerminalContent, TerminalSemanticPrompt};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InteractionEffect {
    Write(Vec<u8>),
    SelectionChanged(Option<SelectionRange>),
    MoveCursorTo(MouseGridPosition),
}

#[derive(Default)]
pub struct PointerInteraction {
    pressed_button: Option<MouseButton>,
    selection_anchor: Option<GridPoint>,
    selection_moved: bool,
    selection: Option<SelectionRange>,
}

pub fn wheel_scroll_lines(delta_y: f64) -> Option<i32> {
    if delta_y < 0.0 {
        Some(3)
    } else if delta_y > 0.0 {
        Some(-3)
    } else {
        None
    }
}

pub fn scroll_delta_pixels(delta_y: f64, line_height: f64) -> Option<f64> {
    if delta_y == 0.0 || line_height <= 0.0 {
        return None;
    }
    if (delta_y.abs() - 1.0).abs() <= f64::EPSILON {
        Some(-delta_y.signum() * 3.0 * line_height)
    } else {
        Some(-delta_y)
    }
}

impl PointerInteraction {
    pub fn selection(&self) -> Option<SelectionRange> {
        self.selection
    }

    pub fn press(
        &mut self,
        mode: MouseMode,
        button: MouseButton,
        position: MouseGridPosition,
    ) -> Vec<InteractionEffect> {
        self.selection = None;
        self.selection_anchor = None;
        self.selection_moved = false;
        if mode.sends_press_release() {
            self.pressed_button = Some(button);
            vec![
                InteractionEffect::SelectionChanged(None),
                InteractionEffect::Write(sgr_press_bytes(button, position)),
            ]
        } else {
            self.pressed_button = None;
            self.selection_anchor = Some(GridPoint::from(position));
            vec![InteractionEffect::SelectionChanged(None)]
        }
    }

    pub fn release(
        &mut self,
        mode: MouseMode,
        position: MouseGridPosition,
    ) -> Vec<InteractionEffect> {
        self.pressed_button = None;
        if mode.sends_press_release() {
            self.selection_anchor = None;
            self.selection_moved = false;
            vec![InteractionEffect::Write(sgr_release_bytes(position))]
        } else {
            if !self.selection_moved {
                self.selection_anchor = None;
                self.selection_moved = false;
                return vec![InteractionEffect::MoveCursorTo(position)];
            }
            self.selection_anchor = None;
            self.selection_moved = false;
            Vec::new()
        }
    }

    pub fn cancel(&mut self) -> Vec<InteractionEffect> {
        self.pressed_button = None;
        self.selection_anchor = None;
        self.selection_moved = false;
        Vec::new()
    }

    pub fn motion(
        &mut self,
        mode: MouseMode,
        position: MouseGridPosition,
    ) -> Vec<InteractionEffect> {
        if mode.sends_drag() {
            if let Some(button) = self.pressed_button {
                return vec![InteractionEffect::Write(sgr_drag_bytes(button, position))];
            }
            return Vec::new();
        }
        let Some(anchor) = self.selection_anchor else {
            return Vec::new();
        };
        let focus = GridPoint::from(position);
        if focus == anchor {
            return Vec::new();
        }
        let range = SelectionRange::between_cells(anchor, focus);
        if self.selection == Some(range) {
            return Vec::new();
        }
        self.selection = Some(range);
        self.selection_moved = true;
        vec![InteractionEffect::SelectionChanged(Some(range))]
    }
}

pub fn cursor_movement_bytes(
    cursor_row: i32,
    cursor_column: i32,
    target: MouseGridPosition,
) -> Option<Vec<u8>> {
    if cursor_row < 0 || cursor_column < 0 || cursor_row as u16 != target.row {
        return None;
    }
    let target_column = i32::from(target.column);
    arrow_bytes_for_delta(target_column - cursor_column)
}

pub fn cursor_movement_bytes_for_content(
    content: &TerminalContent,
    target: MouseGridPosition,
) -> Option<Vec<u8>> {
    let cursor_row = usize::try_from(content.cursor_line).ok()?;
    let cursor_column = usize::try_from(content.cursor_col).ok()?;
    let target_row = usize::from(target.row);
    let target_column = usize::from(target.column);
    let input_rows = active_input_rows(content, cursor_row)?;
    if !input_rows.contains(&target_row) {
        return None;
    }
    let cursor_absolute =
        absolute_input_column(content, input_rows.clone(), cursor_row, cursor_column)?;
    let target_absolute = absolute_input_column(content, input_rows, target_row, target_column)?;
    let delta = target_absolute as i32 - cursor_absolute as i32;
    arrow_bytes_for_delta(delta)
}

pub fn cursor_movement_bytes_between_input_points(
    content: &TerminalContent,
    source: MouseGridPosition,
    target: MouseGridPosition,
) -> Option<Vec<u8>> {
    let cursor_row = usize::try_from(content.cursor_line).ok()?;
    let input_rows = active_input_rows(content, cursor_row)?;
    let source_absolute = absolute_input_column(
        content,
        input_rows.clone(),
        usize::from(source.row),
        usize::from(source.column),
    )?;
    let target_absolute = absolute_input_column(
        content,
        input_rows,
        usize::from(target.row),
        usize::from(target.column),
    )?;
    let delta = target_absolute as i32 - source_absolute as i32;
    arrow_bytes_for_delta(delta)
}

pub fn cursor_movement_bytes_for_editable_input(
    content: &TerminalContent,
    target: MouseGridPosition,
) -> Option<Vec<u8>> {
    let cursor_row = usize::try_from(content.cursor_line).ok()?;
    let cursor_column = usize::try_from(content.cursor_col).ok()?;
    let target_row = usize::from(target.row);
    let target_column = usize::from(target.column);
    let input_rows = active_wrapped_rows(content, cursor_row)?;
    if !input_rows.contains(&target_row) {
        return None;
    }
    let cursor_absolute =
        absolute_input_column(content, input_rows.clone(), cursor_row, cursor_column)?;
    let target_absolute = absolute_input_column(content, input_rows, target_row, target_column)?;
    let delta = target_absolute as i32 - cursor_absolute as i32;
    arrow_bytes_for_delta(delta)
}

pub fn cursor_movement_bytes_between_editable_input_points(
    content: &TerminalContent,
    source: MouseGridPosition,
    target: MouseGridPosition,
) -> Option<Vec<u8>> {
    let cursor_row = usize::try_from(content.cursor_line).ok()?;
    let input_rows = active_wrapped_rows(content, cursor_row)?;
    let source_absolute = absolute_input_column(
        content,
        input_rows.clone(),
        usize::from(source.row),
        usize::from(source.column),
    )?;
    let target_absolute = absolute_input_column(
        content,
        input_rows,
        usize::from(target.row),
        usize::from(target.column),
    )?;
    let delta = target_absolute as i32 - source_absolute as i32;
    arrow_bytes_for_delta(delta)
}

pub fn input_position_in_active_input(
    content: &TerminalContent,
    position: MouseGridPosition,
) -> bool {
    let Some(cursor_row) = usize::try_from(content.cursor_line).ok() else {
        return false;
    };
    let Some(input_rows) = active_input_rows(content, cursor_row) else {
        return false;
    };
    absolute_input_column(
        content,
        input_rows,
        usize::from(position.row),
        usize::from(position.column),
    )
    .is_some()
}

pub fn input_position_in_editable_input(
    content: &TerminalContent,
    position: MouseGridPosition,
) -> bool {
    let Some(cursor_row) = usize::try_from(content.cursor_line).ok() else {
        return false;
    };
    let Some(input_rows) = active_wrapped_rows(content, cursor_row) else {
        return false;
    };
    absolute_input_column(
        content,
        input_rows,
        usize::from(position.row),
        usize::from(position.column),
    )
    .is_some()
}

fn active_input_rows(
    content: &TerminalContent,
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
    content: &TerminalContent,
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
    content: &TerminalContent,
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

fn prompt_leader_before_input(line: &[crate::terminal_grid::TerminalCell]) -> bool {
    let Some(separator) = line
        .iter()
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
        .and_then(|last| line.iter().take(last + 1).position(|cell| cell.text == " "))
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
    content: &TerminalContent,
    cursor_row: usize,
) -> Option<std::ops::RangeInclusive<usize>> {
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

fn absolute_input_column(
    content: &TerminalContent,
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

fn arrow_bytes_for_delta(delta: i32) -> Option<Vec<u8>> {
    let sequence = if delta < 0 { b"\x1b[D" } else { b"\x1b[C" };
    let mut out = Vec::new();
    for _ in 0..delta.unsigned_abs() {
        out.extend_from_slice(sequence);
    }
    if out.is_empty() { None } else { Some(out) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_grid::{
        TerminalCell, TerminalColors, TerminalLineMetadata, TerminalSemanticPrompt,
    };

    fn pos(column: u16, row: u16) -> MouseGridPosition {
        MouseGridPosition { column, row }
    }

    fn reporting_mode() -> MouseMode {
        MouseMode {
            click: true,
            drag: true,
            motion: false,
            sgr: true,
            utf8: false,
        }
    }

    fn line(text: &str) -> Vec<TerminalCell> {
        text.chars()
            .map(|ch| TerminalCell {
                text: ch.to_string().into(),
                ..TerminalCell::blank()
            })
            .collect()
    }

    #[test]
    fn local_drag_creates_grid_selection_when_terminal_mouse_reporting_is_off() {
        let mut interaction = PointerInteraction::default();
        assert_eq!(
            interaction.press(MouseMode::default(), MouseButton::Left, pos(2, 1)),
            vec![InteractionEffect::SelectionChanged(None)]
        );
        assert_eq!(
            interaction.motion(MouseMode::default(), pos(5, 1)),
            vec![InteractionEffect::SelectionChanged(Some(
                SelectionRange::new(
                    GridPoint { row: 1, column: 2 },
                    GridPoint { row: 1, column: 6 }
                )
            ))]
        );
        assert_eq!(
            interaction.selection(),
            Some(SelectionRange::new(
                GridPoint { row: 1, column: 2 },
                GridPoint { row: 1, column: 6 }
            ))
        );
    }

    #[test]
    fn local_drag_reports_selection_only_when_cell_range_changes() {
        let mut interaction = PointerInteraction::default();
        interaction.press(MouseMode::default(), MouseButton::Left, pos(2, 1));
        assert_eq!(
            interaction.motion(MouseMode::default(), pos(5, 1)),
            vec![InteractionEffect::SelectionChanged(Some(
                SelectionRange::new(
                    GridPoint { row: 1, column: 2 },
                    GridPoint { row: 1, column: 6 }
                )
            ))]
        );
        assert_eq!(
            interaction.motion(MouseMode::default(), pos(5, 1)),
            Vec::new()
        );
    }

    #[test]
    fn local_reverse_drag_normalizes_selection() {
        let mut interaction = PointerInteraction::default();
        interaction.press(MouseMode::default(), MouseButton::Left, pos(8, 3));
        interaction.motion(MouseMode::default(), pos(4, 2));
        assert_eq!(
            interaction.selection(),
            Some(SelectionRange::new(
                GridPoint { row: 2, column: 4 },
                GridPoint { row: 3, column: 9 }
            ))
        );
    }

    #[test]
    fn local_drag_stops_changing_selection_after_release() {
        let mut interaction = PointerInteraction::default();
        interaction.press(MouseMode::default(), MouseButton::Left, pos(2, 1));
        interaction.motion(MouseMode::default(), pos(5, 1));
        interaction.release(MouseMode::default(), pos(5, 1));
        assert_eq!(
            interaction.motion(MouseMode::default(), pos(8, 1)),
            Vec::new()
        );
        assert_eq!(
            interaction.selection(),
            Some(SelectionRange::new(
                GridPoint { row: 1, column: 2 },
                GridPoint { row: 1, column: 6 }
            ))
        );
    }

    #[test]
    fn local_drag_stops_changing_selection_after_cancelled_release() {
        let mut interaction = PointerInteraction::default();
        interaction.press(MouseMode::default(), MouseButton::Left, pos(2, 1));
        interaction.motion(MouseMode::default(), pos(5, 1));
        assert_eq!(interaction.cancel(), Vec::new());
        assert_eq!(
            interaction.motion(MouseMode::default(), pos(8, 1)),
            Vec::new()
        );
        assert_eq!(
            interaction.selection(),
            Some(SelectionRange::new(
                GridPoint { row: 1, column: 2 },
                GridPoint { row: 1, column: 6 }
            ))
        );
    }

    #[test]
    fn local_click_without_drag_does_not_leave_selection() {
        let mut interaction = PointerInteraction::default();
        interaction.press(MouseMode::default(), MouseButton::Left, pos(2, 1));
        assert_eq!(
            interaction.release(MouseMode::default(), pos(2, 1)),
            vec![InteractionEffect::MoveCursorTo(pos(2, 1))]
        );
        assert_eq!(interaction.selection(), None);
    }

    #[test]
    fn cursor_movement_uses_arrow_bytes_on_current_row() {
        assert_eq!(
            cursor_movement_bytes(2, 5, pos(3, 2)).as_deref(),
            Some(&b"\x1b[D\x1b[D"[..])
        );
        assert_eq!(
            cursor_movement_bytes(2, 3, pos(5, 2)).as_deref(),
            Some(&b"\x1b[C\x1b[C"[..])
        );
        assert_eq!(cursor_movement_bytes(2, 3, pos(3, 2)), None);
        assert_eq!(cursor_movement_bytes(2, 3, pos(1, 1)), None);
    }

    #[test]
    fn cursor_movement_uses_soft_wrapped_input_rows() {
        let content = TerminalContent {
            lines: vec![
                vec![TerminalCell::blank(); 4],
                vec![TerminalCell::blank(); 4],
                vec![TerminalCell::blank(); 4],
            ],
            line_metadata: vec![
                TerminalLineMetadata {
                    wrapped: true,
                    ..TerminalLineMetadata::default()
                },
                TerminalLineMetadata {
                    wrapped: true,
                    wrap_continuation: true,
                    ..TerminalLineMetadata::default()
                },
                TerminalLineMetadata {
                    wrap_continuation: true,
                    ..TerminalLineMetadata::default()
                },
            ],
            cursor_line: 2,
            cursor_col: 1,
            cursor_visible: true,
            display_offset: 0,
            colors: TerminalColors::default(),
            mouse: MouseMode::default(),
        };
        assert_eq!(
            cursor_movement_bytes_for_content(&content, pos(2, 1)).as_deref(),
            Some(&b"\x1b[D\x1b[D\x1b[D"[..])
        );
        assert_eq!(
            cursor_movement_bytes_for_content(&content, pos(2, 0)).as_deref(),
            Some(&b"\x1b[D\x1b[D\x1b[D\x1b[D\x1b[D\x1b[D\x1b[D"[..])
        );
        assert_eq!(cursor_movement_bytes_for_content(&content, pos(1, 3)), None);
    }

    #[test]
    fn cursor_movement_between_input_points_uses_input_absolute_columns() {
        let content = TerminalContent {
            lines: vec![
                vec![TerminalCell::blank(); 4],
                vec![TerminalCell::blank(); 4],
                vec![TerminalCell::blank(); 4],
            ],
            line_metadata: vec![
                TerminalLineMetadata {
                    wrapped: true,
                    ..TerminalLineMetadata::default()
                },
                TerminalLineMetadata {
                    wrapped: true,
                    wrap_continuation: true,
                    ..TerminalLineMetadata::default()
                },
                TerminalLineMetadata {
                    wrap_continuation: true,
                    ..TerminalLineMetadata::default()
                },
            ],
            cursor_line: 2,
            cursor_col: 1,
            cursor_visible: true,
            display_offset: 0,
            colors: TerminalColors::default(),
            mouse: MouseMode::default(),
        };

        assert!(input_position_in_active_input(&content, pos(2, 0)));
        assert_eq!(
            cursor_movement_bytes_between_input_points(&content, pos(2, 0), pos(2, 1)).as_deref(),
            Some(&b"\x1b[C\x1b[C\x1b[C\x1b[C"[..])
        );
        assert_eq!(
            cursor_movement_bytes_between_input_points(&content, pos(3, 1), pos(2, 0)).as_deref(),
            Some(&b"\x1b[D\x1b[D\x1b[D\x1b[D\x1b[D"[..])
        );
        assert_eq!(
            cursor_movement_bytes_between_input_points(&content, pos(1, 3), pos(9, 3)),
            None
        );
        assert!(!input_position_in_active_input(&content, pos(9, 3)));
    }

    #[test]
    fn cursor_movement_uses_semantic_prompt_continuation_rows() {
        let content = TerminalContent {
            lines: vec![
                vec![TerminalCell::blank(); 3],
                vec![TerminalCell::blank(); 4],
                vec![TerminalCell::blank(); 5],
                vec![TerminalCell::blank(); 2],
            ],
            line_metadata: vec![
                TerminalLineMetadata::default(),
                TerminalLineMetadata {
                    semantic_prompt: TerminalSemanticPrompt::Prompt,
                    ..TerminalLineMetadata::default()
                },
                TerminalLineMetadata {
                    semantic_prompt: TerminalSemanticPrompt::Continuation,
                    ..TerminalLineMetadata::default()
                },
                TerminalLineMetadata::default(),
            ],
            cursor_line: 2,
            cursor_col: 3,
            cursor_visible: true,
            display_offset: 0,
            colors: TerminalColors::default(),
            mouse: MouseMode::default(),
        };
        assert_eq!(
            cursor_movement_bytes_for_content(&content, pos(2, 1)).as_deref(),
            Some(&b"\x1b[D\x1b[D\x1b[D\x1b[D\x1b[D"[..])
        );
        assert_eq!(
            cursor_movement_bytes_for_content(&content, pos(1, 2)).as_deref(),
            Some(&b"\x1b[D\x1b[D"[..])
        );
        assert_eq!(cursor_movement_bytes_for_content(&content, pos(1, 0)), None);
        assert_eq!(cursor_movement_bytes_for_content(&content, pos(0, 3)), None);
    }

    #[test]
    fn cursor_movement_skips_decorative_semantic_prompt_header() {
        let content = TerminalContent {
            lines: vec![
                line("~/Documents/Projects/Chelotype on main"),
                line("❯ abc"),
            ],
            line_metadata: vec![
                TerminalLineMetadata {
                    semantic_prompt: TerminalSemanticPrompt::Prompt,
                    ..TerminalLineMetadata::default()
                },
                TerminalLineMetadata {
                    semantic_prompt: TerminalSemanticPrompt::Continuation,
                    ..TerminalLineMetadata::default()
                },
            ],
            cursor_line: 1,
            cursor_col: 5,
            cursor_visible: true,
            display_offset: 0,
            colors: TerminalColors::default(),
            mouse: MouseMode::default(),
        };

        assert_eq!(
            cursor_movement_bytes_for_content(&content, pos(2, 1)).as_deref(),
            Some(&b"\x1b[D\x1b[D\x1b[D"[..])
        );
        assert_eq!(
            cursor_movement_bytes_for_content(&content, pos(10, 0)),
            None
        );
    }

    #[test]
    fn terminal_mouse_reporting_writes_sgr_press_drag_release_and_skips_local_selection() {
        let mut interaction = PointerInteraction::default();
        assert_eq!(
            interaction.press(reporting_mode(), MouseButton::Left, pos(2, 1)),
            vec![
                InteractionEffect::SelectionChanged(None),
                InteractionEffect::Write(b"\x1b[<0;3;2M".to_vec())
            ]
        );
        assert_eq!(
            interaction.motion(reporting_mode(), pos(3, 1)),
            vec![InteractionEffect::Write(b"\x1b[<32;4;2M".to_vec())]
        );
        assert_eq!(
            interaction.release(reporting_mode(), pos(3, 1)),
            vec![InteractionEffect::Write(b"\x1b[<0;4;2m".to_vec())]
        );
        assert_eq!(interaction.selection(), None);
    }

    #[test]
    fn wheel_delta_maps_to_scrollback_lines() {
        assert_eq!(wheel_scroll_lines(-1.0), Some(3));
        assert_eq!(wheel_scroll_lines(1.0), Some(-3));
        assert_eq!(wheel_scroll_lines(0.0), None);
    }

    #[test]
    fn scroll_delta_preserves_high_resolution_surface_pixels() {
        assert_eq!(scroll_delta_pixels(-2.5, 20.0), Some(2.5));
        assert_eq!(scroll_delta_pixels(1.25, 20.0), Some(-1.25));
        assert_eq!(scroll_delta_pixels(-1.0, 20.0), Some(60.0));
    }
}
