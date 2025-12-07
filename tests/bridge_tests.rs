use chelotype::bridge::{
    EntryHandle, InputBridge, LabelHandle, TerminalAdapter, html_to_pango, insert_caret,
};
use gtk::gdk;
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
}

impl FakeTerminal {
    fn with_line(text: &str) -> Self {
        Self {
            fed: Rc::new(RefCell::new(Vec::new())),
            line_html: Rc::new(RefCell::new(text.to_string())),
            line_text: Rc::new(RefCell::new(text.to_string())),
            cursor: Rc::new(RefCell::new((text.len() as i64, 0))),
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
}

#[test]
fn converts_font_color_to_span() {
    let input = r##"<pre><font color="#ff0000">abc</font></pre>"##;
    let output = html_to_pango(input);
    assert_eq!(output, r##"<span foreground="#ff0000">abc</span>"##);
}

#[test]
fn converts_font_color_case_insensitive() {
    let input = r##"<PRE><FONT COLOR="#00ffcc">ok</FONT></PRE>"##;
    let output = html_to_pango(input);
    assert_eq!(output, r##"<span foreground="#00ffcc">ok</span>"##);
}

#[test]
fn strips_div_and_pre() {
    let input = "<div><pre>text</pre></div>";
    let output = html_to_pango(input);
    assert_eq!(output, "text");
}

#[test]
fn strips_pre_with_attributes() {
    let input = r#"<pre style="padding:4px"><span style="color:#00ff00">ok</span></pre>"#;
    let output = html_to_pango(input);
    assert_eq!(output, r##"<span foreground="#00ff00">ok</span>"##);
}

#[test]
fn normalizes_underline_style() {
    let input = r#"<u style="text-decoration-style:solid">u</u>"#;
    let output = html_to_pango(input);
    assert_eq!(output, "<u>u</u>");
}

#[test]
fn caret_injected_at_position() {
    let markup = html_to_pango(r##"<font color="#ff0000">ab</font>"##);
    let with_caret = insert_caret(&markup, 1);
    assert!(with_caret.contains("▏"));
    assert!(with_caret.contains("a"));
    assert!(with_caret.contains("b"));
}

#[test]
fn caret_injected_past_end() {
    let markup = html_to_pango("ab");
    let with_caret = insert_caret(&markup, 5);
    assert!(with_caret.ends_with("▏</span>") || with_caret.ends_with("▏"));
}

#[test]
fn converts_span_style_color() {
    let input = r##"<span style="color:#123456">val</span>"##;
    let output = html_to_pango(input);
    assert_eq!(output, r##"<span foreground="#123456">val</span>"##);
}

#[test]
fn sync_applies_markup_and_text() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let term = FakeTerminal::with_line("<font color=\"#ff0000\">x</font>");
    let bridge = InputBridge::new(entry.clone(), ghost.clone(), term.clone());
    bridge.sync_from_terminal();
    assert_eq!(
        ghost.markup.borrow().as_str(),
        "<span foreground=\"#ff0000\">x</span><span foreground=\"#7dd3fc\">▏</span>"
    );
    assert_eq!(
        entry.text_value().as_str(),
        "<font color=\"#ff0000\">x</font>"
    );
}

#[test]
fn handle_key_forwards_enter() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let term = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry, ghost, term.clone());
    assert!(bridge.handle_key(gdk::Key::Return, gdk::ModifierType::empty()));
    assert_eq!(term.fed.borrow().as_slice(), b"\n");
}

#[test]
fn handle_key_allows_ctrl_c() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let term = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry, ghost, term.clone());
    assert!(bridge.handle_key(gdk::Key::c, gdk::ModifierType::CONTROL_MASK));
    assert_eq!(term.fed.borrow().as_slice(), &[0x03]);
}

#[test]
fn handle_key_plain_text_is_not_consumed() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let term = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry, ghost, term.clone());
    assert!(!bridge.handle_key(gdk::Key::a, gdk::ModifierType::empty()));
    assert!(term.fed.borrow().is_empty());
}

#[test]
fn handle_key_alt_prefix() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let term = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry, ghost, term.clone());
    assert!(bridge.handle_key(gdk::Key::a, gdk::ModifierType::ALT_MASK));
    assert_eq!(term.fed.borrow().as_slice(), b"\x1ba");
}

#[test]
fn alt_key_sets_skip_for_insert() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let term = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry, ghost, term.clone());
    assert!(bridge.handle_key(gdk::Key::b, gdk::ModifierType::ALT_MASK));
    bridge.handle_insert_text("b");
    assert_eq!(term.fed.borrow().as_slice(), b"\x1bb");
}

#[test]
fn insert_text_feeds_terminal() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let term = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry, ghost, term.clone());
    assert!(bridge.handle_insert_text("abc"));
    assert_eq!(term.fed.borrow().as_slice(), b"abc");
}

#[test]
fn insert_text_is_skipped_when_syncing() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let term = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry, ghost, term.clone());
    bridge.set_syncing_for_test(true);
    assert!(bridge.handle_insert_text("abc"));
    assert_eq!(term.fed.borrow().as_slice(), b"abc");
}

#[test]
fn insert_text_is_skipped_when_suppressed() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let term = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry, ghost, term.clone());
    bridge.set_suppress_insert_for_test(true);
    assert!(!bridge.handle_insert_text("abc"));
    assert!(term.fed.borrow().is_empty());
}

#[test]
fn sync_sets_cursor_position() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let term = FakeTerminal::with_line("abc");
    term.set_line("abc", None, 2);
    let bridge = InputBridge::new(entry.clone(), ghost, term);
    bridge.sync_from_terminal();
    assert_eq!(entry.position_value(), 2);
}

#[test]
fn sync_empty_line_safe() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let term = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry.clone(), ghost.clone(), term);
    bridge.sync_from_terminal();
    assert_eq!(entry.text_value().as_str(), "");
    assert_eq!(
        ghost.markup.borrow().as_str(),
        "<span foreground=\"#7dd3fc\">▏</span>"
    );
}

#[test]
fn arrow_keys_send_escape() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let term = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry, ghost, term.clone());
    bridge.handle_key(gdk::Key::Left, gdk::ModifierType::empty());
    assert_eq!(term.fed.borrow().as_slice(), b"\x1b[D");
}

#[test]
fn delete_sends_sequence() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let term = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry, ghost, term.clone());
    bridge.handle_key(gdk::Key::Delete, gdk::ModifierType::empty());
    assert_eq!(term.fed.borrow().as_slice(), b"\x1b[3~");
}

#[test]
fn sync_after_typing_updates_markup() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let term = FakeTerminal::with_line("");
    let bridge = InputBridge::new(entry.clone(), ghost.clone(), term.clone());
    bridge.handle_key(gdk::Key::t, gdk::ModifierType::empty());
    term.set_line("t", Some("<font color=\"#00ff00\">t</font>"), 1);
    bridge.sync_from_terminal();
    assert_eq!(
        ghost.markup.borrow().as_str(),
        "<span foreground=\"#00ff00\">t</span><span foreground=\"#7dd3fc\">▏</span>"
    );
    assert_eq!(entry.text_value().as_str(), "t");
}

#[test]
fn cursor_notify_from_programmatic_position_does_not_spin() {
    let entry = NotifyingEntry::default();
    let ghost = FakeLabel::default();
    let term = FakeTerminal::with_line("abc");
    let bridge = InputBridge::new(entry.clone(), ghost, term.clone());
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
fn move_cursor_to_clamps_and_moves() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let term = FakeTerminal::with_line("abc");
    term.set_line("abc", None, 1);
    let bridge = InputBridge::new(entry, ghost, term.clone());
    bridge.move_cursor_to(5);
    assert_eq!(term.fed.borrow().as_slice(), b"\x1b[C\x1b[C");
}

#[test]
fn move_cursor_to_ignores_same_position() {
    let entry = FakeEntry::default();
    let ghost = FakeLabel::default();
    let term = FakeTerminal::with_line("abc");
    term.set_line("abc", None, 2);
    let bridge = InputBridge::new(entry, ghost, term.clone());
    bridge.move_cursor_to(2);
    assert!(term.fed.borrow().is_empty());
}
