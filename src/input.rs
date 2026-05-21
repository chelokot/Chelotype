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
    if key == gdk::Key::F1 {
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
    match key {
        gdk::Key::Return | gdk::Key::KP_Enter => Some(b"\r".to_vec()),
        gdk::Key::BackSpace => Some(vec![0x7f]),
        gdk::Key::Tab => Some(b"\t".to_vec()),
        gdk::Key::Left => Some(b"\x1b[D".to_vec()),
        gdk::Key::Right => Some(b"\x1b[C".to_vec()),
        gdk::Key::Up => Some(b"\x1b[A".to_vec()),
        gdk::Key::Down => Some(b"\x1b[B".to_vec()),
        gdk::Key::Home => Some(b"\x1b[H".to_vec()),
        gdk::Key::End => Some(b"\x1b[F".to_vec()),
        gdk::Key::Delete => Some(b"\x1b[3~".to_vec()),
        _ => printable_key_to_bytes(key, keycode, state),
    }
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
        None
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
            key_to_action(gdk::Key::F1, gdk::ModifierType::empty()),
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
