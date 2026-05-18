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
    ScrollDisplay(i32),
    CopySelection,
    CutSelection,
    PasteClipboard,
    NewPane,
    NextPane,
    PreviousPane,
    ZoomIn,
    ZoomOut,
    ZoomReset,
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

pub fn key_to_action(key: gdk::Key, state: gdk::ModifierType) -> Option<KeyAction> {
    let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
    let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
    if matches!(key, gdk::Key::Left | gdk::Key::Right) && (ctrl || shift) {
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
    if ctrl
        && !shift
        && key
            .to_unicode()
            .is_some_and(|ch| ch.eq_ignore_ascii_case(&'a'))
    {
        return Some(KeyAction::SelectInput);
    }
    if ctrl {
        match key {
            gdk::Key::Page_Down => return Some(KeyAction::NextPane),
            gdk::Key::Page_Up => return Some(KeyAction::PreviousPane),
            gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add => return Some(KeyAction::ZoomIn),
            gdk::Key::minus | gdk::Key::KP_Subtract => return Some(KeyAction::ZoomOut),
            gdk::Key::_0 | gdk::Key::KP_0 => return Some(KeyAction::ZoomReset),
            _ => {}
        }
    }
    if ctrl
        && shift
        && key
            .to_unicode()
            .is_some_and(|ch| ch.eq_ignore_ascii_case(&'c'))
    {
        return Some(KeyAction::CopySelection);
    }
    if ctrl
        && shift
        && key
            .to_unicode()
            .is_some_and(|ch| ch.eq_ignore_ascii_case(&'t'))
    {
        return Some(KeyAction::NewPane);
    }
    if ctrl && !shift {
        if key
            .to_unicode()
            .is_some_and(|ch| ch.eq_ignore_ascii_case(&'x'))
        {
            return Some(KeyAction::CutSelection);
        }
        if key
            .to_unicode()
            .is_some_and(|ch| ch.eq_ignore_ascii_case(&'v'))
        {
            return Some(KeyAction::PasteClipboard);
        }
    }
    if state.contains(gdk::ModifierType::SHIFT_MASK) {
        match key {
            gdk::Key::Page_Up => return Some(KeyAction::ScrollDisplay(10)),
            gdk::Key::Page_Down => return Some(KeyAction::ScrollDisplay(-10)),
            _ => {}
        }
    }
    key_to_terminal_bytes(key, state).map(KeyAction::Write)
}

pub fn key_to_terminal_bytes(key: gdk::Key, state: gdk::ModifierType) -> Option<Vec<u8>> {
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
        gdk::Key::Return => Some(b"\n".to_vec()),
        gdk::Key::BackSpace => Some(vec![0x7f]),
        gdk::Key::Tab => Some(b"\t".to_vec()),
        gdk::Key::Left => Some(b"\x1b[D".to_vec()),
        gdk::Key::Right => Some(b"\x1b[C".to_vec()),
        gdk::Key::Up => Some(b"\x1b[A".to_vec()),
        gdk::Key::Down => Some(b"\x1b[B".to_vec()),
        gdk::Key::Home => Some(b"\x1b[H".to_vec()),
        gdk::Key::End => Some(b"\x1b[F".to_vec()),
        gdk::Key::Delete => Some(b"\x1b[3~".to_vec()),
        _ => printable_key_to_bytes(key, state),
    }
}

fn printable_key_to_bytes(key: gdk::Key, state: gdk::ModifierType) -> Option<Vec<u8>> {
    let ch = key.to_unicode()?;
    let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
    let alt = state.intersects(
        gdk::ModifierType::ALT_MASK | gdk::ModifierType::META_MASK | gdk::ModifierType::SUPER_MASK,
    );
    let mut buf = [0u8; 4];
    let mut out = Vec::new();
    if ctrl {
        let upper = ch.to_ascii_uppercase();
        out.push((upper as u8) & 0x1f);
    } else {
        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
    }
    if alt {
        let mut prefixed = b"\x1b".to_vec();
        prefixed.extend_from_slice(&out);
        out = prefixed;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_enter_backspace_and_tab() {
        assert_eq!(
            key_to_terminal_bytes(gdk::Key::Return, gdk::ModifierType::empty()).as_deref(),
            Some(&b"\n"[..])
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
            Some(KeyAction::NewPane)
        );
        assert_eq!(
            key_to_action(gdk::Key::Page_Down, gdk::ModifierType::CONTROL_MASK),
            Some(KeyAction::NextPane)
        );
        assert_eq!(
            key_to_action(gdk::Key::Page_Up, gdk::ModifierType::CONTROL_MASK),
            Some(KeyAction::PreviousPane)
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
            key_to_action(gdk::Key::plus, gdk::ModifierType::CONTROL_MASK),
            Some(KeyAction::ZoomIn)
        );
        assert_eq!(
            key_to_action(gdk::Key::minus, gdk::ModifierType::CONTROL_MASK),
            Some(KeyAction::ZoomOut)
        );
    }
}
