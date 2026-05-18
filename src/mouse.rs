#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MouseGridPosition {
    pub column: u16,
    pub row: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

pub fn sgr_press_bytes(button: MouseButton, position: MouseGridPosition) -> Vec<u8> {
    sgr_mouse_bytes(button_code(button), position, 'M')
}

pub fn sgr_release_bytes(position: MouseGridPosition) -> Vec<u8> {
    sgr_mouse_bytes(0, position, 'm')
}

pub fn sgr_drag_bytes(button: MouseButton, position: MouseGridPosition) -> Vec<u8> {
    sgr_mouse_bytes(button_code(button) + 32, position, 'M')
}

fn sgr_mouse_bytes(code: u16, position: MouseGridPosition, suffix: char) -> Vec<u8> {
    format!(
        "\x1b[<{};{};{}{}",
        code,
        position.column.saturating_add(1),
        position.row.saturating_add(1),
        suffix
    )
    .into_bytes()
}

fn button_code(button: MouseButton) -> u16 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_sgr_left_press_as_one_based_grid_position() {
        assert_eq!(
            sgr_press_bytes(MouseButton::Left, MouseGridPosition { column: 4, row: 2 }),
            b"\x1b[<0;5;3M"
        );
    }

    #[test]
    fn encodes_sgr_release() {
        assert_eq!(
            sgr_release_bytes(MouseGridPosition { column: 4, row: 2 }),
            b"\x1b[<0;5;3m"
        );
    }

    #[test]
    fn encodes_sgr_drag() {
        assert_eq!(
            sgr_drag_bytes(MouseButton::Left, MouseGridPosition { column: 4, row: 2 }),
            b"\x1b[<32;5;3M"
        );
    }
}
