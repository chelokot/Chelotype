#![cfg(feature = "legacy-vte-bridge")]

use chelotype::bridge::{
    CaretHandle, EntryHandle, InputBridge, LabelHandle, TerminalAdapter, html_to_pango,
    insert_caret,
};
use gtk::gdk;
use serial_test::serial;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone, Default)]
struct FakeEntry {
    text: Rc<RefCell<String>>,
    position: Rc<Cell<i32>>,
}

impl FakeEntry {
    fn text_value(&self) -> String {
        self.text.borrow().clone()
    }

    fn position_value(&self) -> i32 {
        self.position.get()
    }
}

impl EntryHandle for FakeEntry {
    fn set_text(&self, text: &str) {
        self.text.borrow_mut().clear();
        self.text.borrow_mut().push_str(text);
    }

    fn set_position(&self, pos: i32) {
        self.position.set(pos);
    }
}

#[derive(Clone, Default)]
struct FakeLabel {
    markup: Rc<RefCell<String>>,
}

impl LabelHandle for FakeLabel {
    fn set_markup(&self, markup: &str) {
        self.markup.borrow_mut().clear();
        self.markup.borrow_mut().push_str(markup);
    }

    fn index_at_x(&self, x: f64, _y: f64) -> Option<usize> {
        if x < 0.5 { Some(0) } else { Some(1) }
    }

    fn caret_offset(&self, caret_pos: usize) -> f64 {
        caret_pos as f64
    }
}

#[derive(Clone, Default)]
struct FakeCaret {
    visible: Rc<Cell<bool>>,
    offset: Rc<Cell<f64>>,
}

impl FakeCaret {
    fn visible_value(&self) -> bool {
        self.visible.get()
    }
}

impl CaretHandle for FakeCaret {
    fn set_offset(&self, x: f64) {
        self.offset.set(x);
    }

    fn set_visible(&self, visible: bool) {
        self.visible.set(visible);
    }
}

#[derive(Clone, Default)]
struct NotifyingEntry {
    text: Rc<RefCell<String>>,
    position: Rc<Cell<i32>>,
    on_position: Rc<RefCell<Option<Box<dyn Fn(i32)>>>>,
}

impl NotifyingEntry {
    fn set_on_position<F: Fn(i32) + 'static>(&self, handler: F) {
        self.on_position.replace(Some(Box::new(handler)));
    }

    fn position_value(&self) -> i32 {
        self.position.get()
    }
}

impl EntryHandle for NotifyingEntry {
    fn set_text(&self, text: &str) {
        self.text.borrow_mut().clear();
        self.text.borrow_mut().push_str(text);
    }

    fn set_position(&self, pos: i32) {
        self.position.set(pos);
        if let Some(cb) = self.on_position.borrow().as_ref() {
            cb(pos);
        }
    }
}

#[derive(Clone, Default)]
struct FakeTerminal {
    fed: Rc<RefCell<Vec<u8>>>,
    line_html: Rc<RefCell<String>>,
    line_text: Rc<RefCell<String>>,
    cursor: Rc<RefCell<(i64, i64)>>,
    columns: i64,
}

impl FakeTerminal {
    fn with_line(text: &str) -> Self {
        Self {
            fed: Rc::new(RefCell::new(Vec::new())),
            line_html: Rc::new(RefCell::new(text.to_string())),
            line_text: Rc::new(RefCell::new(text.to_string())),
            cursor: Rc::new(RefCell::new((text.len() as i64, 0))),
            columns: text.len() as i64 + 10,
        }
    }

    fn set_line(&self, text: &str, html: Option<&str>, cursor: i64) {
        *self.line_text.borrow_mut() = text.to_string();
        *self.line_html.borrow_mut() = html.unwrap_or(text).to_string();
        *self.cursor.borrow_mut() = (cursor, 0);
    }
}

impl TerminalAdapter for FakeTerminal {
    fn feed_child(&self, bytes: &[u8]) {
        self.fed.borrow_mut().extend_from_slice(bytes);
    }

    fn cursor_position(&self) -> (i64, i64) {
        *self.cursor.borrow()
    }

    fn line_html(&self, _row: i64) -> String {
        self.line_html.borrow().clone()
    }

    fn line_text(&self, _row: i64) -> String {
        self.line_text.borrow().clone()
    }

    fn column_count(&self) -> i64 {
        self.columns
    }
}

#[test]
#[serial]
fn converts_font_color_to_span() {
    let input = r##"<pre><font color="#ff0000">abc</font></pre>"##;
    let output = html_to_pango(input);
    assert_eq!(output, r##"<span foreground="#ff0000">abc</span>"##);
}

#[test]
#[serial]
fn converts_font_color_case_insensitive() {
    let input = r##"<PRE><FONT COLOR="#00ffcc">ok</FONT></PRE>"##;
    let output = html_to_pango(input);
    assert_eq!(output, r##"<span foreground="#00ffcc">ok</span>"##);
}

#[test]
#[serial]
fn strips_div_and_pre() {
    let input = "<div><pre>text</pre></div>";
    let output = html_to_pango(input);
    assert_eq!(output, "text");
}

#[test]
#[serial]
fn strips_pre_with_attributes() {
    let input = r#"<pre style="padding:4px"><span style="color:#00ff00">ok</span></pre>"#;
    let output = html_to_pango(input);
    assert_eq!(output, r##"<span foreground="#00ff00">ok</span>"##);
}

#[test]
#[serial]
fn normalizes_underline_style() {
    let input = r#"<u style="text-decoration-style:solid">u</u>"#;
    let output = html_to_pango(input);
    assert_eq!(output, "<u>u</u>");
}

#[test]
#[serial]
fn caret_injected_at_position() {
    let markup = html_to_pango(r##"<font color="#ff0000">ab</font>"##);
    let with_caret = insert_caret(&markup, 1);
    assert!(with_caret.contains("▏"));
    assert!(with_caret.contains("a"));
    assert!(with_caret.contains("b"));
}

#[test]
#[serial]
fn caret_injected_past_end() {
    let markup = html_to_pango("ab");
    let with_caret = insert_caret(&markup, 5);
    assert!(with_caret.contains("▏"));
}

#[test]
#[serial]
fn converts_span_style_color() {
    let input = r##"<span style="color:#123456">val</span>"##;
    let output = html_to_pango(input);
    assert_eq!(output, r##"<span foreground="#123456">val</span>"##);
}

#[test]
#[serial]
fn sync_applies_markup_and_text() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let caret = FakeCaret::default();
    let shadow = FakeTerminal::with_line("x");
    shadow.set_line("x", Some("<font color=\"#ff0000\">x</font>"), 1);
    let output = FakeTerminal::with_line("");
    let bridge = InputBridge::new(
        entry.clone(),
        ghost.clone(),
        caret.clone(),
        shadow.clone(),
        output,
    );
    bridge.sync_from_terminal();
    let markup = ghost.markup.borrow();
    assert!(markup.contains("foreground=\"#ff0000\">x</span>"));
    assert_eq!(entry.text_value().as_str(), "x");
    assert!(caret.visible_value());
}

#[test]
#[serial]
fn handle_key_forwards_enter() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let caret = FakeCaret::default();
    let shadow = FakeTerminal::with_line("");
    let output = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry, ghost, caret, shadow, output.clone());
    assert!(bridge.handle_key(gdk::Key::Return, gdk::ModifierType::empty()));
    assert_eq!(output.fed.borrow().as_slice(), b"\n");
}

#[test]
#[serial]
fn handle_key_allows_ctrl_c() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let caret = FakeCaret::default();
    let shadow = FakeTerminal::with_line("");
    let output = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry, ghost, caret, shadow.clone(), output.clone());
    assert!(bridge.handle_key(gdk::Key::c, gdk::ModifierType::CONTROL_MASK));
    assert_eq!(shadow.fed.borrow().as_slice(), &[0x03]);
    assert!(output.fed.borrow().is_empty());
}

#[test]
#[serial]
fn handle_key_plain_text_is_not_consumed() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let caret = FakeCaret::default();
    let shadow = FakeTerminal::with_line("");
    let output = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry, ghost, caret, shadow.clone(), output.clone());
    assert!(!bridge.handle_key(gdk::Key::a, gdk::ModifierType::empty()));
    assert!(shadow.fed.borrow().is_empty());
    assert!(output.fed.borrow().is_empty());
}

#[test]
#[serial]
fn handle_key_alt_prefix() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let caret = FakeCaret::default();
    let shadow = FakeTerminal::with_line("");
    let output = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry, ghost, caret, shadow.clone(), output.clone());
    assert!(bridge.handle_key(gdk::Key::a, gdk::ModifierType::ALT_MASK));
    assert_eq!(shadow.fed.borrow().as_slice(), b"\x1ba");
    assert!(output.fed.borrow().is_empty());
}

#[test]
#[serial]
fn alt_key_sets_skip_for_insert() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let caret = FakeCaret::default();
    let shadow = FakeTerminal::with_line("");
    let output = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry, ghost, caret, shadow.clone(), output.clone());
    assert!(bridge.handle_key(gdk::Key::b, gdk::ModifierType::ALT_MASK));
    bridge.handle_insert_text("b");
    assert_eq!(shadow.fed.borrow().as_slice(), b"\x1bb");
    assert!(output.fed.borrow().is_empty());
}

#[test]
#[serial]
fn insert_text_feeds_terminal() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let caret = FakeCaret::default();
    let shadow = FakeTerminal::with_line("");
    let output = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry, ghost, caret, shadow.clone(), output.clone());
    assert!(bridge.handle_insert_text("abc"));
    assert_eq!(shadow.fed.borrow().as_slice(), b"abc");
    assert!(output.fed.borrow().is_empty());
}

#[test]
#[serial]
fn insert_text_is_skipped_when_syncing() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let caret = FakeCaret::default();
    let shadow = FakeTerminal::with_line("");
    let output = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry, ghost, caret, shadow.clone(), output.clone());
    bridge.set_syncing_for_test(true);
    assert!(bridge.handle_insert_text("abc"));
    assert_eq!(shadow.fed.borrow().as_slice(), b"abc");
    assert!(output.fed.borrow().is_empty());
}

#[test]
#[serial]
fn insert_text_is_skipped_when_suppressed() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let caret = FakeCaret::default();
    let shadow = FakeTerminal::with_line("");
    let output = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry, ghost, caret, shadow.clone(), output.clone());
    bridge.set_suppress_insert_for_test(true);
    assert!(!bridge.handle_insert_text("abc"));
    assert!(shadow.fed.borrow().is_empty());
    assert!(output.fed.borrow().is_empty());
}

#[test]
#[serial]
fn sync_sets_cursor_position() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let caret = FakeCaret::default();
    let term = FakeTerminal::with_line("abc");
    term.set_line("abc", None, 2);
    let output = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry.clone(), ghost, caret, term, output);
    bridge.sync_from_terminal();
    assert_eq!(entry.position_value(), 2);
}

#[test]
#[serial]
fn sync_empty_line_safe() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let caret = FakeCaret::default();
    let term = FakeTerminal::with_line("");
    let output = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry.clone(), ghost.clone(), caret.clone(), term, output);
    bridge.sync_from_terminal();
    assert_eq!(entry.text_value().as_str(), "");
    assert!(caret.visible_value());
}

#[test]
#[serial]
fn arrow_keys_send_escape() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let caret = FakeCaret::default();
    let shadow = FakeTerminal::with_line("");
    let output = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry, ghost, caret, shadow.clone(), output.clone());
    bridge.handle_key(gdk::Key::Left, gdk::ModifierType::empty());
    assert_eq!(shadow.fed.borrow().as_slice(), b"\x1b[D");
    assert!(output.fed.borrow().is_empty());
}

#[test]
#[serial]
fn delete_sends_sequence() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let caret = FakeCaret::default();
    let shadow = FakeTerminal::with_line("");
    let output = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry, ghost, caret, shadow.clone(), output.clone());
    bridge.handle_key(gdk::Key::Delete, gdk::ModifierType::empty());
    assert_eq!(shadow.fed.borrow().as_slice(), b"\x1b[3~");
    assert!(output.fed.borrow().is_empty());
}

#[test]
#[serial]
fn sync_after_typing_updates_markup() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let caret = FakeCaret::default();
    let term = FakeTerminal::with_line("");
    let output = FakeTerminal::with_line("");
    let bridge = InputBridge::new(
        entry.clone(),
        ghost.clone(),
        caret.clone(),
        term.clone(),
        output,
    );
    bridge.handle_key(gdk::Key::t, gdk::ModifierType::empty());
    term.set_line("t", Some("<font color=\"#00ff00\">t</font>"), 1);
    bridge.sync_from_terminal();
    let markup = ghost.markup.borrow();
    assert!(markup.contains("foreground=\"#00ff00\">t</span>"));
    assert_eq!(entry.text_value().as_str(), "t");
}

#[test]
#[serial]
#[serial]
fn cursor_notify_from_programmatic_position_does_not_spin() {
    let entry = NotifyingEntry::default();
    let ghost = FakeLabel::default();
    let caret = FakeCaret::default();
    let term = FakeTerminal::with_line("abc");
    let output = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry.clone(), ghost, caret, term.clone(), output);
    let loop_counter = Rc::new(Cell::new(0));
    entry.set_on_position({
        let bridge = bridge.clone();
        let counter = loop_counter.clone();
        move |pos| {
            let count = counter.get();
            if count > 32 {
                panic!("cursor notify loop");
            }
            counter.set(count + 1);
            bridge.move_cursor_to(pos);
        }
    });
    term.set_line("abc", None, 1);
    bridge.sync_from_terminal();
    assert_eq!(loop_counter.get(), 1);
    assert_eq!(entry.position_value(), 1);
}

#[test]
#[serial]
fn move_cursor_to_clamps_and_moves() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let caret = FakeCaret::default();
    let term = FakeTerminal::with_line("abc");
    term.set_line("abc", None, 1);
    let output = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry, ghost, caret, term.clone(), output.clone());
    bridge.move_cursor_to(5);
    assert_eq!(term.fed.borrow().as_slice(), b"\x1b[C\x1b[C");
    assert!(output.fed.borrow().is_empty());
}

#[test]
#[serial]
fn move_cursor_to_ignores_same_position() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let caret = FakeCaret::default();
    let term = FakeTerminal::with_line("abc");
    term.set_line("abc", None, 2);
    let output = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry, ghost, caret, term.clone(), output);
    bridge.move_cursor_to(2);
    assert!(term.fed.borrow().is_empty());
}

#[test]
#[serial]
#[serial]
fn click_right_half_moves_after_char() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let caret = FakeCaret::default();
    let term = FakeTerminal::with_line("ab");
    let output = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry, ghost, caret, term.clone(), output);
    bridge.move_cursor_from_point(1.0, 0.0);
    assert_eq!(term.fed.borrow().as_slice(), b"\x1b[D");
}

#[test]
#[serial]
#[serial]
fn click_left_half_stays_before_char() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let caret = FakeCaret::default();
    let term = FakeTerminal::with_line("ab");
    let output = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry, ghost, caret, term.clone(), output);
    bridge.move_cursor_from_point(0.0, 0.0);
    assert_eq!(term.fed.borrow().as_slice(), b"\x1b[D\x1b[D");
}
