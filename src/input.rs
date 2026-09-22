use crate::command_blocks::CommandBlockDirection;
use gtk::gdk;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeyAction {
    Write(Vec<u8>),
    CursorMove {
        direction: CursorDirection,
        unit: CursorUnit,
        selecting: bool,
    },
    SelectInput,
    SelectCommandBlockOutput(CommandBlockDirection),
    ScrollDisplay(i32),
    CopySelection,
    CutSelection,
    PasteClipboard,
    UndoInput,
    RedoInput,
    NewTab,
    NewWindow,
    CloseTab,
    NextTab,
    PreviousTab,
    SplitPane,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    OpenSettings,
    OpenAbout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorDirection {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorUnit {
    Cell,
    Word,
}

pub fn cursor_move_terminal_bytes(direction: CursorDirection, unit: CursorUnit) -> Vec<u8> {
    match (direction, unit) {
        (CursorDirection::Left, CursorUnit::Cell) => b"\x1b[D".to_vec(),
        (CursorDirection::Right, CursorUnit::Cell) => b"\x1b[C".to_vec(),
        (CursorDirection::Left, CursorUnit::Word) => b"\x1b[1;5D".to_vec(),
        (CursorDirection::Right, CursorUnit::Word) => b"\x1b[1;5C".to_vec(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PhysicalKey {
    A,
    C,
    E,
    N,
    T,
    V,
    W,
    X,
    Z,
    Comma,
    Minus,
    Equal,
    Digit0,
}

pub fn key_to_action(key: gdk::Key, keycode: u32, state: gdk::ModifierType) -> Option<KeyAction> {
    let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
    let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
    if matches!(key, gdk::Key::Left | gdk::Key::Right) {
        return Some(KeyAction::CursorMove {
            direction: if key == gdk::Key::Left {
                CursorDirection::Left
            } else {
                CursorDirection::Right
            },
            unit: if ctrl {
                CursorUnit::Word
            } else {
                CursorUnit::Cell
            },
            selecting: shift,
        });
    }
    if ctrl && shift && key == gdk::Key::F1 {
        return Some(KeyAction::OpenAbout);
    }
    if ctrl && shift && matches!(key, gdk::Key::Up | gdk::Key::Down) {
        return Some(KeyAction::SelectCommandBlockOutput(
            if key == gdk::Key::Up {
                CommandBlockDirection::Previous
            } else {
                CommandBlockDirection::Next
            },
        ));
    }
    if ctrl && !shift && key_matches(key, keycode, PhysicalKey::A, &['a']) {
        return Some(KeyAction::SelectInput);
    }
    if ctrl {
        if key == gdk::Key::Page_Down {
            return Some(KeyAction::NextTab);
        }
        if key == gdk::Key::Page_Up {
            return Some(KeyAction::PreviousTab);
        }
        if matches!(key, gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add)
            || physical_key(keycode) == Some(PhysicalKey::Equal)
        {
            return Some(KeyAction::ZoomIn);
        }
        if matches!(key, gdk::Key::minus | gdk::Key::KP_Subtract)
            || physical_key(keycode) == Some(PhysicalKey::Minus)
        {
            return Some(KeyAction::ZoomOut);
        }
        if matches!(key, gdk::Key::_0 | gdk::Key::KP_0)
            || physical_key(keycode) == Some(PhysicalKey::Digit0)
        {
            return Some(KeyAction::ZoomReset);
        }
        if key == gdk::Key::comma || physical_key(keycode) == Some(PhysicalKey::Comma) {
            return Some(KeyAction::OpenSettings);
        }
    }
    if ctrl && shift && key_matches(key, keycode, PhysicalKey::C, &['c']) {
        return Some(KeyAction::CopySelection);
    }
    if ctrl && shift && key_matches(key, keycode, PhysicalKey::T, &['t']) {
        return Some(KeyAction::NewTab);
    }
    if ctrl && shift && key_matches(key, keycode, PhysicalKey::N, &['n']) {
        return Some(KeyAction::NewWindow);
    }
    if ctrl && shift && key_matches(key, keycode, PhysicalKey::W, &['w']) {
        return Some(KeyAction::CloseTab);
    }
    if ctrl && shift && key_matches(key, keycode, PhysicalKey::E, &['e']) {
        return Some(KeyAction::SplitPane);
    }
    if ctrl && !shift {
        if key_matches(key, keycode, PhysicalKey::X, &['x']) {
            return Some(KeyAction::CutSelection);
        }
        if key_matches(key, keycode, PhysicalKey::V, &['v']) {
            return Some(KeyAction::PasteClipboard);
        }
        if key_matches(key, keycode, PhysicalKey::Z, &['z']) {
            return Some(KeyAction::UndoInput);
        }
    }
    if ctrl && shift && key_matches(key, keycode, PhysicalKey::Z, &['z']) {
        return Some(KeyAction::RedoInput);
    }
    if state.contains(gdk::ModifierType::SHIFT_MASK) {
        match key {
            gdk::Key::Page_Up => return Some(KeyAction::ScrollDisplay(10)),
            gdk::Key::Page_Down => return Some(KeyAction::ScrollDisplay(-10)),
            _ => {}
        }
    }
    key_to_terminal_bytes(key, keycode, state).map(KeyAction::Write)
}

pub fn key_to_terminal_bytes(
    key: gdk::Key,
    keycode: u32,
    state: gdk::ModifierType,
) -> Option<Vec<u8>> {
    let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
    let alt = state.intersects(
        gdk::ModifierType::ALT_MASK | gdk::ModifierType::META_MASK | gdk::ModifierType::SUPER_MASK,
    );
    if ctrl {
        match key {
            gdk::Key::BackSpace => return Some(vec![0x17]),
            gdk::Key::Delete => return Some(b"\x1bd".to_vec()),
            _ => {}
        }
    }
    if alt {
        match key {
            gdk::Key::BackSpace => return Some(b"\x1b\x7f".to_vec()),
            gdk::Key::Delete => return Some(b"\x1bd".to_vec()),
            _ => {}
        }
    }
    if let Some(bytes) = named_key_to_bytes(key, state) {
        return Some(bytes);
    }
    match key {
        gdk::Key::Escape => Some(vec![0x1b]),
        gdk::Key::Return | gdk::Key::KP_Enter => Some(b"\r".to_vec()),
        gdk::Key::BackSpace => Some(vec![0x7f]),
        gdk::Key::Tab => Some(b"\t".to_vec()),
        _ => printable_key_to_bytes(key, keycode, state),
    }
}

fn named_key_to_bytes(key: gdk::Key, state: gdk::ModifierType) -> Option<Vec<u8>> {
    let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
    if key == gdk::Key::ISO_Left_Tab || (key == gdk::Key::Tab && shift) {
        return Some(b"\x1b[Z".to_vec());
    }
    let (number, suffix, ss3) = match key {
        gdk::Key::Up | gdk::Key::KP_Up => (1, 'A', false),
        gdk::Key::Down | gdk::Key::KP_Down => (1, 'B', false),
        gdk::Key::Right | gdk::Key::KP_Right => (1, 'C', false),
        gdk::Key::Left | gdk::Key::KP_Left => (1, 'D', false),
        gdk::Key::Home | gdk::Key::KP_Home => (1, 'H', false),
        gdk::Key::End | gdk::Key::KP_End => (1, 'F', false),
        gdk::Key::Insert | gdk::Key::KP_Insert => (2, '~', false),
        gdk::Key::Delete | gdk::Key::KP_Delete => (3, '~', false),
        gdk::Key::Page_Up | gdk::Key::KP_Page_Up => (5, '~', false),
        gdk::Key::Page_Down | gdk::Key::KP_Page_Down => (6, '~', false),
        gdk::Key::F1 => (1, 'P', true),
        gdk::Key::F2 => (1, 'Q', true),
        gdk::Key::F3 => (1, 'R', true),
        gdk::Key::F4 => (1, 'S', true),
        gdk::Key::F5 => (15, '~', false),
        gdk::Key::F6 => (17, '~', false),
        gdk::Key::F7 => (18, '~', false),
        gdk::Key::F8 => (19, '~', false),
        gdk::Key::F9 => (20, '~', false),
        gdk::Key::F10 => (21, '~', false),
        gdk::Key::F11 => (23, '~', false),
        gdk::Key::F12 => (24, '~', false),
        _ => return None,
    };
    let alt = state.intersects(gdk::ModifierType::ALT_MASK | gdk::ModifierType::META_MASK);
    let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
    let sequence = if modifier != 1 {
        format!("\x1b[{number};{modifier}{suffix}")
    } else if ss3 {
        format!("\x1bO{suffix}")
    } else if suffix == '~' {
        format!("\x1b[{number}~")
    } else {
        format!("\x1b[{suffix}")
    };
    Some(sequence.into_bytes())
}

fn printable_key_to_bytes(
    key: gdk::Key,
    keycode: u32,
    state: gdk::ModifierType,
) -> Option<Vec<u8>> {
    let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
    let alt = state.intersects(
        gdk::ModifierType::ALT_MASK | gdk::ModifierType::META_MASK | gdk::ModifierType::SUPER_MASK,
    );
    let mut buf = [0u8; 4];
    let mut out = Vec::new();
    if ctrl {
        let control = physical_control_byte(keycode)
            .or_else(|| key.to_unicode().and_then(control_byte_for_char))?;
        out.push(control);
    } else {
        let ch = key.to_unicode()?;
        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
    }
    if alt {
        let mut prefixed = b"\x1b".to_vec();
        prefixed.extend_from_slice(&out);
        out = prefixed;
    }
    Some(out)
}

fn key_matches(
    key: gdk::Key,
    keycode: u32,
    physical: PhysicalKey,
    fallback_chars: &[char],
) -> bool {
    physical_key(keycode) == Some(physical)
        || key.to_unicode().is_some_and(|ch| {
            fallback_chars
                .iter()
                .any(|fallback| ch.eq_ignore_ascii_case(fallback))
        })
}

fn control_byte_for_char(ch: char) -> Option<u8> {
    if ch.is_ascii_alphabetic() {
        Some((ch.to_ascii_uppercase() as u8) & 0x1f)
    } else {
        match ch {
            ' ' | '@' => Some(0),
            '[' => Some(0x1b),
            '\\' => Some(0x1c),
            ']' => Some(0x1d),
            '^' => Some(0x1e),
            '_' => Some(0x1f),
            '?' => Some(0x7f),
            _ => None,
        }
    }
}

fn physical_control_byte(keycode: u32) -> Option<u8> {
    match physical_key(keycode)? {
        PhysicalKey::A => Some(0x01),
        PhysicalKey::C => Some(0x03),
        PhysicalKey::E => Some(0x05),
        PhysicalKey::N => Some(0x0e),
        PhysicalKey::T => Some(0x14),
        PhysicalKey::V => Some(0x16),
        PhysicalKey::W => Some(0x17),
        PhysicalKey::X => Some(0x18),
        PhysicalKey::Z => Some(0x1a),
        PhysicalKey::Comma | PhysicalKey::Minus | PhysicalKey::Equal | PhysicalKey::Digit0 => None,
    }
}

fn physical_key(keycode: u32) -> Option<PhysicalKey> {
    match keycode {
        38 => Some(PhysicalKey::A),
        54 => Some(PhysicalKey::C),
        26 => Some(PhysicalKey::E),
        57 => Some(PhysicalKey::N),
        28 => Some(PhysicalKey::T),
        55 => Some(PhysicalKey::V),
        25 => Some(PhysicalKey::W),
        53 => Some(PhysicalKey::X),
        52 => Some(PhysicalKey::Z),
        59 => Some(PhysicalKey::Comma),
        20 => Some(PhysicalKey::Minus),
        21 => Some(PhysicalKey::Equal),
        19 => Some(PhysicalKey::Digit0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_to_action(key: gdk::Key, state: gdk::ModifierType) -> Option<KeyAction> {
        super::key_to_action(key, 0, state)
    }

    fn key_to_terminal_bytes(key: gdk::Key, state: gdk::ModifierType) -> Option<Vec<u8>> {
        super::key_to_terminal_bytes(key, 0, state)
    }

    #[test]
    fn maps_enter_backspace_and_tab() {
        assert_eq!(
            key_to_terminal_bytes(gdk::Key::Return, gdk::ModifierType::empty()).as_deref(),
            Some(&b"\r"[..])
        );
        assert_eq!(
            key_to_terminal_bytes(gdk::Key::KP_Enter, gdk::ModifierType::empty()).as_deref(),
            Some(&b"\r"[..])
        );
        assert_eq!(
            key_to_terminal_bytes(gdk::Key::BackSpace, gdk::ModifierType::empty()).as_deref(),
            Some(&[0x7f][..])
        );
        assert_eq!(
            key_to_terminal_bytes(gdk::Key::Tab, gdk::ModifierType::empty()).as_deref(),
            Some(&b"\t"[..])
        );
    }

    #[test]
    fn maps_navigation_keys() {
        assert_eq!(
            key_to_terminal_bytes(gdk::Key::Left, gdk::ModifierType::empty()).as_deref(),
            Some(&b"\x1b[D"[..])
        );
        assert_eq!(
            key_to_terminal_bytes(gdk::Key::Delete, gdk::ModifierType::empty()).as_deref(),
            Some(&b"\x1b[3~"[..])
        );
    }

    #[test]
    fn maps_cursor_move_terminal_fallback_bytes() {
        assert_eq!(
            cursor_move_terminal_bytes(CursorDirection::Right, CursorUnit::Cell),
            b"\x1b[C"
        );
        assert_eq!(
            cursor_move_terminal_bytes(CursorDirection::Right, CursorUnit::Word),
            b"\x1b[1;5C"
        );
    }

    #[test]
    fn maps_arrow_selection_and_word_movement() {
        assert_eq!(
            key_to_action(gdk::Key::Left, gdk::ModifierType::empty()),
            Some(KeyAction::CursorMove {
                direction: CursorDirection::Left,
                unit: CursorUnit::Cell,
                selecting: false,
            })
        );
        assert_eq!(
            key_to_action(gdk::Key::Left, gdk::ModifierType::CONTROL_MASK),
            Some(KeyAction::CursorMove {
                direction: CursorDirection::Left,
                unit: CursorUnit::Word,
                selecting: false,
            })
        );
        assert_eq!(
            key_to_action(
                gdk::Key::Right,
                gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK
            ),
            Some(KeyAction::CursorMove {
                direction: CursorDirection::Right,
                unit: CursorUnit::Word,
                selecting: true,
            })
        );
        assert_eq!(
            key_to_action(gdk::Key::Left, gdk::ModifierType::SHIFT_MASK),
            Some(KeyAction::CursorMove {
                direction: CursorDirection::Left,
                unit: CursorUnit::Cell,
                selecting: true,
            })
        );
    }

    #[test]
    fn maps_command_block_output_selection_shortcuts() {
        assert_eq!(
            key_to_action(
                gdk::Key::Up,
                gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK
            ),
            Some(KeyAction::SelectCommandBlockOutput(
                CommandBlockDirection::Previous
            ))
        );
        assert_eq!(
            key_to_action(
                gdk::Key::Down,
                gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK
            ),
            Some(KeyAction::SelectCommandBlockOutput(
                CommandBlockDirection::Next
            ))
        );
    }

    #[test]
    fn maps_word_delete_shortcuts() {
        assert_eq!(
            key_to_terminal_bytes(gdk::Key::BackSpace, gdk::ModifierType::CONTROL_MASK).as_deref(),
            Some(&[0x17][..])
        );
        assert_eq!(
            key_to_terminal_bytes(gdk::Key::Delete, gdk::ModifierType::CONTROL_MASK).as_deref(),
            Some(&b"\x1bd"[..])
        );
        assert_eq!(
            key_to_terminal_bytes(gdk::Key::BackSpace, gdk::ModifierType::ALT_MASK).as_deref(),
            Some(&b"\x1b\x7f"[..])
        );
    }

    #[test]
    fn maps_printable_unicode() {
        assert_eq!(
            key_to_terminal_bytes(gdk::Key::Cyrillic_ya, gdk::ModifierType::empty()).as_deref(),
            Some("я".as_bytes())
        );
    }

    #[test]
    fn maps_ctrl_and_alt_printable_keys() {
        assert_eq!(
            key_to_terminal_bytes(gdk::Key::c, gdk::ModifierType::CONTROL_MASK).as_deref(),
            Some(&[0x03][..])
        );
        assert_eq!(
            key_to_terminal_bytes(gdk::Key::a, gdk::ModifierType::ALT_MASK).as_deref(),
            Some(&b"\x1ba"[..])
        );
    }

    #[test]
    fn maps_shift_page_navigation_to_scrollback_actions() {
        assert_eq!(
            key_to_action(gdk::Key::Page_Up, gdk::ModifierType::SHIFT_MASK),
            Some(KeyAction::ScrollDisplay(10))
        );
        assert_eq!(
            key_to_action(gdk::Key::Page_Down, gdk::ModifierType::SHIFT_MASK),
            Some(KeyAction::ScrollDisplay(-10))
        );
    }

    #[test]
    fn maps_ctrl_shift_c_to_copy_selection_without_intercepting_ctrl_c() {
        assert_eq!(
            key_to_action(
                gdk::Key::c,
                gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK
            ),
            Some(KeyAction::CopySelection)
        );
        assert_eq!(
            key_to_action(gdk::Key::c, gdk::ModifierType::CONTROL_MASK),
            Some(KeyAction::Write(vec![0x03]))
        );
    }

    #[test]
    fn maps_workspace_tab_shortcuts() {
        assert_eq!(
            key_to_action(
                gdk::Key::t,
                gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK
            ),
            Some(KeyAction::NewTab)
        );
        assert_eq!(
            key_to_action(
                gdk::Key::w,
                gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK
            ),
            Some(KeyAction::CloseTab)
        );
        assert_eq!(
            key_to_action(gdk::Key::Page_Down, gdk::ModifierType::CONTROL_MASK),
            Some(KeyAction::NextTab)
        );
        assert_eq!(
            key_to_action(gdk::Key::Page_Up, gdk::ModifierType::CONTROL_MASK),
            Some(KeyAction::PreviousTab)
        );
        assert_eq!(
            key_to_action(
                gdk::Key::e,
                gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK
            ),
            Some(KeyAction::SplitPane)
        );
    }

    #[test]
    fn maps_modern_terminal_editing_shortcuts() {
        assert_eq!(
            key_to_action(gdk::Key::a, gdk::ModifierType::CONTROL_MASK),
            Some(KeyAction::SelectInput)
        );
        assert_eq!(
            key_to_action(gdk::Key::x, gdk::ModifierType::CONTROL_MASK),
            Some(KeyAction::CutSelection)
        );
        assert_eq!(
            key_to_action(gdk::Key::v, gdk::ModifierType::CONTROL_MASK),
            Some(KeyAction::PasteClipboard)
        );
        assert_eq!(
            key_to_action(gdk::Key::z, gdk::ModifierType::CONTROL_MASK),
            Some(KeyAction::UndoInput)
        );
        assert_eq!(
            key_to_action(
                gdk::Key::z,
                gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK
            ),
            Some(KeyAction::RedoInput)
        );
        assert_eq!(
            key_to_action(gdk::Key::plus, gdk::ModifierType::CONTROL_MASK),
            Some(KeyAction::ZoomIn)
        );
        assert_eq!(
            key_to_action(gdk::Key::minus, gdk::ModifierType::CONTROL_MASK),
            Some(KeyAction::ZoomOut)
        );
    }

    #[test]
    fn forwards_function_and_navigation_keys_with_xterm_modifiers() {
        let cases: [(gdk::Key, &[u8]); 18] = [
            (gdk::Key::F1, b"\x1bOP"),
            (gdk::Key::F2, b"\x1bOQ"),
            (gdk::Key::F3, b"\x1bOR"),
            (gdk::Key::F4, b"\x1bOS"),
            (gdk::Key::F5, b"\x1b[15~"),
            (gdk::Key::F6, b"\x1b[17~"),
            (gdk::Key::F7, b"\x1b[18~"),
            (gdk::Key::F8, b"\x1b[19~"),
            (gdk::Key::F9, b"\x1b[20~"),
            (gdk::Key::F10, b"\x1b[21~"),
            (gdk::Key::F11, b"\x1b[23~"),
            (gdk::Key::F12, b"\x1b[24~"),
            (gdk::Key::Insert, b"\x1b[2~"),
            (gdk::Key::Page_Up, b"\x1b[5~"),
            (gdk::Key::Page_Down, b"\x1b[6~"),
            (gdk::Key::ISO_Left_Tab, b"\x1b[Z"),
            (gdk::Key::KP_Home, b"\x1b[H"),
            (gdk::Key::KP_End, b"\x1b[F"),
        ];
        for (key, expected) in cases {
            assert_eq!(
                key_to_action(key, gdk::ModifierType::empty()),
                Some(KeyAction::Write(expected.to_vec())),
                "{key:?}",
            );
        }
        for (key, modifiers, expected) in [
            (gdk::Key::Tab, gdk::ModifierType::SHIFT_MASK, &b"\x1b[Z"[..]),
            (
                gdk::Key::F2,
                gdk::ModifierType::CONTROL_MASK,
                &b"\x1b[1;5Q"[..],
            ),
            (
                gdk::Key::F12,
                gdk::ModifierType::ALT_MASK,
                &b"\x1b[24;3~"[..],
            ),
            (
                gdk::Key::Left,
                gdk::ModifierType::CONTROL_MASK,
                &b"\x1b[1;5D"[..],
            ),
            (
                gdk::Key::Up,
                gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::ALT_MASK,
                &b"\x1b[1;4A"[..],
            ),
        ] {
            assert_eq!(
                key_to_terminal_bytes(key, modifiers).as_deref(),
                Some(expected)
            );
        }
    }

    #[test]
    fn forwards_control_punctuation_for_terminal_applications() {
        for (key, expected) in [
            (gdk::Key::space, 0),
            (gdk::Key::at, 0),
            (gdk::Key::bracketleft, 0x1b),
            (gdk::Key::backslash, 0x1c),
            (gdk::Key::bracketright, 0x1d),
            (gdk::Key::asciicircum, 0x1e),
            (gdk::Key::underscore, 0x1f),
            (gdk::Key::question, 0x7f),
        ] {
            assert_eq!(
                key_to_terminal_bytes(key, gdk::ModifierType::CONTROL_MASK),
                Some(vec![expected]),
            );
        }
    }

    #[test]
    fn maps_settings_shortcut() {
        assert_eq!(
            key_to_action(gdk::Key::comma, gdk::ModifierType::CONTROL_MASK),
            Some(KeyAction::OpenSettings)
        );
    }

    #[test]
    fn maps_window_and_about_shortcuts() {
        assert_eq!(
            key_to_action(
                gdk::Key::n,
                gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK
            ),
            Some(KeyAction::NewWindow)
        );
        assert_eq!(
            key_to_action(
                gdk::Key::F1,
                gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK
            ),
            Some(KeyAction::OpenAbout)
        );
    }

    #[test]
    fn maps_physical_shortcuts_under_cyrillic_layout() {
        assert_eq!(
            super::key_to_action(
                gdk::Key::Cyrillic_es,
                54,
                gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK
            ),
            Some(KeyAction::CopySelection)
        );
        assert_eq!(
            super::key_to_action(gdk::Key::Cyrillic_ef, 38, gdk::ModifierType::CONTROL_MASK),
            Some(KeyAction::SelectInput)
        );
        assert_eq!(
            super::key_to_action(gdk::Key::Cyrillic_che, 53, gdk::ModifierType::CONTROL_MASK),
            Some(KeyAction::CutSelection)
        );
        assert_eq!(
            super::key_to_action(gdk::Key::Cyrillic_em, 55, gdk::ModifierType::CONTROL_MASK),
            Some(KeyAction::PasteClipboard)
        );
        assert_eq!(
            super::key_to_action(gdk::Key::Cyrillic_ya, 52, gdk::ModifierType::CONTROL_MASK),
            Some(KeyAction::UndoInput)
        );
        assert_eq!(
            super::key_to_action(
                gdk::Key::Cyrillic_ya,
                52,
                gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK
            ),
            Some(KeyAction::RedoInput)
        );
        assert_eq!(
            super::key_to_action(
                gdk::Key::Cyrillic_ie,
                28,
                gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK
            ),
            Some(KeyAction::NewTab)
        );
        assert_eq!(
            super::key_to_action(
                gdk::Key::Cyrillic_tse,
                25,
                gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK
            ),
            Some(KeyAction::CloseTab)
        );
        assert_eq!(
            super::key_to_action(
                gdk::Key::Cyrillic_u,
                26,
                gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK
            ),
            Some(KeyAction::SplitPane)
        );
        assert_eq!(
            super::key_to_action(gdk::Key::Cyrillic_be, 59, gdk::ModifierType::CONTROL_MASK),
            Some(KeyAction::OpenSettings)
        );
        assert_eq!(
            super::key_to_terminal_bytes(
                gdk::Key::Cyrillic_es,
                54,
                gdk::ModifierType::CONTROL_MASK
            )
            .as_deref(),
            Some(&[0x03][..])
        );
        assert_eq!(
            super::key_to_terminal_bytes(
                gdk::Key::Cyrillic_che,
                53,
                gdk::ModifierType::CONTROL_MASK
            )
            .as_deref(),
            Some(&[0x18][..])
        );
    }
}
