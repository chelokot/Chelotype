use gtk::gdk;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeyAction {
    Write(Vec<u8>),
    ScrollDisplay(i32),
}

pub fn key_to_action(key: gdk::Key, state: gdk::ModifierType) -> Option<KeyAction> {
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
}
