use crate::backend::MouseMode;
use crate::mouse::{
    MouseButton, MouseGridPosition, sgr_drag_bytes, sgr_press_bytes, sgr_release_bytes,
};
use crate::selection::{GridPoint, SelectionRange};

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
    let delta = target_column - cursor_column;
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
}
