use adw::Application;
use adw::prelude::*;
use gtk::{gio, glib};
use regex::Regex;
use std::cell::Cell;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};
use vte::prelude::*;
use vte::{PtyFlags, Terminal};

fn main() -> glib::ExitCode {
    let app = Application::builder()
        .application_id("com.chelotype.Terminal")
        .build();
    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &Application) {
    let header = adw::HeaderBar::builder()
        .title_widget(&gtk::Label::new(Some("Chelotype")))
        .show_start_title_buttons(true)
        .show_end_title_buttons(true)
        .build();

    let terminal = Terminal::builder()
        .input_enabled(true)
        .can_focus(false)
        .build();
    terminal.set_scrollback_lines(20000);
    terminal.set_hexpand(true);
    terminal.set_vexpand(true);

    let entry = gtk::Entry::builder().hexpand(true).build();
    entry.set_editable(true);
    entry.add_css_class("monospace");
    entry.add_css_class("overlay-input");
    entry.set_placeholder_text(Some("Type command"));

    let ghost = gtk::Label::builder()
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Center)
        .use_markup(true)
        .build();
    ghost.set_can_target(false);
    ghost.add_css_class("monospace");
    ghost.set_hexpand(true);
    ghost.set_halign(gtk::Align::Fill);

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&entry));
    overlay.add_overlay(&ghost);
    overlay.set_margin_start(10);
    overlay.set_margin_end(10);
    overlay.set_margin_top(8);
    overlay.set_margin_bottom(12);

    apply_overlay_style(&entry);

    let entry_handle = GtkEntryHandle::new(entry.clone());
    let label_handle = GtkLabelHandle::new(ghost.clone());
    let terminal_adapter = VteTerminalAdapter::new(terminal.clone());
    let bridge = InputBridge::new(entry_handle, label_handle, terminal_adapter);
    bridge.sync_from_terminal();
    bridge.attach_to_terminal(&terminal);
    wire_keys(&entry, bridge.clone());

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&header);
    content.append(&terminal);
    content.append(&overlay);
    start_shell(&terminal);

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Chelotype Terminal")
        .default_width(1100)
        .default_height(760)
        .content(&content)
        .build();

    window.present();
    entry.grab_focus();
}

fn start_shell(terminal: &Terminal) {
    let shell_path = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    let argv = [shell_path.as_str()];
    spawn_process(terminal, &argv);
}

fn spawn_process(terminal: &Terminal, argv: &[&str]) {
    let envv_owned: Vec<String> = std::env::vars()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    let envv: Vec<&str> = envv_owned.iter().map(String::as_str).collect();

    terminal.spawn_async(
        PtyFlags::DEFAULT,
        None::<&str>,
        argv,
        &envv,
        glib::SpawnFlags::SEARCH_PATH,
        || {},
        -1,
        None::<&gio::Cancellable>,
        |result| {
            if let Err(error) = result {
                eprintln!("Failed to start process: {error}");
            }
        },
    );
}
fn apply_overlay_style(entry: &gtk::Entry) {
    let css = "
        entry.overlay-input {
            background: transparent;
            border-radius: 8px;
            padding: 10px 12px;
            color: transparent;
            caret-color: @theme_fg_color;
            box-shadow: inset 0 0 0 1px rgba(255,255,255,0.08);
        }
        entry.overlay-input:focus {
            box-shadow: inset 0 0 0 1px @accent_color, 0 0 0 1px rgba(0,0,0,0.25);
        }
    ";
    let provider = gtk::CssProvider::new();
    provider.load_from_data(css);
    gtk::style_context_add_provider_for_display(
        &entry.display(),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn debug_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("CHELOTYPE_DEBUG") == Ok("1".to_string()))
}

fn debug_file() -> Option<&'static Mutex<File>> {
    static FILE: OnceLock<Option<Mutex<File>>> = OnceLock::new();
    FILE.get_or_init(|| {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/chelotype.log")
            .map(Mutex::new)
            .ok()
    })
    .as_ref()
}

fn debug_log(event: &str) {
    if !debug_enabled() {
        return;
    }
    if let Some(file) = debug_file() {
        if let Ok(mut guard) = file.lock() {
            let _ = writeln!(guard, "{event}");
        }
    }
}

#[derive(Clone)]
struct GtkEntryHandle {
    widget: gtk::Entry,
}

impl GtkEntryHandle {
    fn new(widget: gtk::Entry) -> Self {
        Self { widget }
    }
}

#[derive(Clone)]
struct GtkLabelHandle {
    widget: gtk::Label,
}

impl GtkLabelHandle {
    fn new(widget: gtk::Label) -> Self {
        Self { widget }
    }
}

trait EntryHandle: Clone {
    fn set_text(&self, text: &str);
    fn set_position(&self, pos: i32);
}

trait LabelHandle: Clone {
    fn set_markup(&self, markup: &str);
}

impl EntryHandle for GtkEntryHandle {
    fn set_text(&self, text: &str) {
        self.widget.set_text(text);
    }

    fn set_position(&self, pos: i32) {
        self.widget.set_position(pos);
    }
}

impl LabelHandle for GtkLabelHandle {
    fn set_markup(&self, markup: &str) {
        self.widget.set_markup(markup);
    }
}

#[derive(Clone)]
struct VteTerminalAdapter {
    terminal: Terminal,
}

impl VteTerminalAdapter {
    fn new(terminal: Terminal) -> Self {
        Self { terminal }
    }
}

trait TerminalAdapter: Clone + 'static {
    fn feed_child(&self, bytes: &[u8]);
    fn cursor_position(&self) -> (i64, i64);
    fn line_html(&self, row: i64) -> String;
    fn line_text(&self, row: i64) -> String;
}

impl TerminalAdapter for VteTerminalAdapter {
    fn feed_child(&self, bytes: &[u8]) {
        self.terminal.feed_child(bytes);
    }

    fn cursor_position(&self) -> (i64, i64) {
        self.terminal.cursor_position()
    }

    fn line_html(&self, row: i64) -> String {
        let (html, _) = self.terminal.text_range_format(
            vte::Format::Html,
            row,
            0,
            row,
            self.terminal.column_count(),
        );
        html.map(|v| v.to_string()).unwrap_or_default()
    }

    fn line_text(&self, row: i64) -> String {
        let (text, _) = self.terminal.text_range_format(
            vte::Format::Text,
            row,
            0,
            row,
            self.terminal.column_count(),
        );
        text.map(|v| v.to_string()).unwrap_or_default()
    }
}

#[derive(Clone)]
struct InputBridge<E: EntryHandle + 'static, L: LabelHandle + 'static, T: TerminalAdapter> {
    entry: E,
    ghost: L,
    terminal: T,
    syncing: Rc<Cell<bool>>,
    suppress_cursor_notify: Rc<Cell<bool>>,
    skip_next_insert: Rc<Cell<bool>>,
    suppress_insert: Rc<Cell<bool>>,
}

impl<E: EntryHandle + 'static, L: LabelHandle + 'static, T: TerminalAdapter> InputBridge<E, L, T> {
    fn new(entry: E, ghost: L, terminal: T) -> Self {
        Self {
            entry,
            ghost,
            terminal,
            syncing: Rc::new(Cell::new(false)),
            suppress_cursor_notify: Rc::new(Cell::new(false)),
            skip_next_insert: Rc::new(Cell::new(false)),
            suppress_insert: Rc::new(Cell::new(false)),
        }
    }

    #[cfg(test)]
    fn set_syncing_for_test(&self, value: bool) {
        self.syncing.set(value);
    }

    #[cfg(test)]
    fn set_suppress_insert_for_test(&self, value: bool) {
        self.suppress_insert.set(value);
    }

    fn cursor_notify_suppressed(&self) -> bool {
        self.suppress_cursor_notify.get()
    }

    fn attach_to_terminal(&self, term_widget: &Terminal) {
        let contents_self = self.clone();
        term_widget.connect_contents_changed(move |_| {
            debug_log("terminal:contents-changed");
            contents_self.sync_from_terminal();
        });
        let cursor_self = self.clone();
        term_widget.connect_cursor_moved(move |_| {
            debug_log("terminal:cursor-moved");
            cursor_self.sync_from_terminal();
        });
    }

    fn handle_key(&self, key: gtk::gdk::Key, state: gtk::gdk::ModifierType) -> bool {
        debug_log(&format!("key:{key:?}:state:{state:?}"));
        let bytes = match key {
            gtk::gdk::Key::Return => Some(b"\n".to_vec()),
            gtk::gdk::Key::BackSpace => Some(vec![0x7f]),
            gtk::gdk::Key::Tab => Some(b"\t".to_vec()),
            gtk::gdk::Key::Left => Some(b"\x1b[D".to_vec()),
            gtk::gdk::Key::Right => Some(b"\x1b[C".to_vec()),
            gtk::gdk::Key::Up => Some(b"\x1b[A".to_vec()),
            gtk::gdk::Key::Down => Some(b"\x1b[B".to_vec()),
            gtk::gdk::Key::Home => Some(b"\x1b[H".to_vec()),
            gtk::gdk::Key::End => Some(b"\x1b[F".to_vec()),
            gtk::gdk::Key::Delete => Some(b"\x1b[3~".to_vec()),
            _ => {
                let ctrl = state.contains(gtk::gdk::ModifierType::CONTROL_MASK);
                let alt = state.intersects(
                    gtk::gdk::ModifierType::ALT_MASK
                        | gtk::gdk::ModifierType::META_MASK
                        | gtk::gdk::ModifierType::SUPER_MASK,
                );
                if (ctrl || alt) && key.to_unicode().is_some() {
                    let ch = key.to_unicode().unwrap();
                    let mut out = Vec::new();
                    if ctrl {
                        let upper = ch.to_ascii_uppercase();
                        let ctrl_byte = (upper as u8) & 0x1f;
                        out.push(ctrl_byte);
                    } else {
                        let mut buf = [0u8; 4];
                        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                    }
                    if alt {
                        let mut prefixed = b"\x1b".to_vec();
                        prefixed.extend_from_slice(&out);
                        out = prefixed;
                    }
                    self.skip_next_insert.set(true);
                    Some(out)
                } else {
                    None
                }
            }
        };
        if let Some(data) = bytes {
            debug_log(&format!("feed:{data:?}"));
            self.terminal.feed_child(&data);
            return true;
        }
        false
    }

    fn handle_insert_text(&self, text: &str) {
        if self.syncing.get() || self.suppress_insert.get() {
            debug_log("insert:skip-syncing");
            return;
        }
        if self.skip_next_insert.replace(false) {
            debug_log("insert:skip");
            return;
        }
        if text.is_empty() {
            return;
        }
        debug_log(&format!("insert:{text}"));
        self.terminal.feed_child(text.as_bytes());
    }

    fn sync_from_terminal(&self) {
        if self.syncing.get() {
            return;
        }
        self.syncing.set(true);
        self.suppress_cursor_notify.set(true);
        self.suppress_insert.set(true);
        debug_log("sync:start");
        let (_, row) = self.terminal.cursor_position();
        let html = self.terminal.line_html(row);
        let text = self.terminal.line_text(row);
        let markup = html_to_pango(&html);
        self.ghost.set_markup(markup.as_str());
        self.entry.set_text(&text);

        let (cursor_col, _) = self.terminal.cursor_position();
        let cursor_pos = i32::try_from(cursor_col).unwrap_or(0);
        self.entry.set_position(cursor_pos);
        self.suppress_insert.set(false);
        #[cfg(test)]
        {
            self.suppress_cursor_notify.set(false);
        }
        #[cfg(not(test))]
        {
            let flag = self.suppress_cursor_notify.clone();
            glib::idle_add_local(move || {
                flag.set(false);
                glib::ControlFlow::Break
            });
        }
        self.syncing.set(false);
    }

    fn move_cursor_to(&self, target: i32) {
        if self.syncing.get() || self.suppress_cursor_notify.get() {
            return;
        }
        debug_log(&format!("cursor:move:{target}"));
        let (_, row) = self.terminal.cursor_position();
        let max_len = self.terminal.line_text(row).chars().count() as i32;
        let clamped = target.clamp(0, max_len);
        let current = self.terminal.cursor_position().0.try_into().unwrap_or(0);
        if clamped == current {
            return;
        }
        let delta = clamped - current;
        let move_left = b"\x1b[D";
        let move_right = b"\x1b[C";
        let seq = if delta > 0 { move_right } else { move_left };
        let steps = delta.abs();
        for _ in 0..steps {
            self.terminal.feed_child(seq);
        }
        self.sync_from_terminal();
    }
}

fn wire_keys(
    entry: &gtk::Entry,
    bridge: InputBridge<GtkEntryHandle, GtkLabelHandle, VteTerminalAdapter>,
) {
    entry.add_controller({
        let controller = gtk::EventControllerKey::new();
        let bridge_clone = bridge.clone();
        controller.connect_key_pressed(move |_controller, key, _code, state| {
            match bridge_clone.handle_key(key, state) {
                true => glib::Propagation::Stop,
                false => glib::Propagation::Proceed,
            }
        });
        controller
    });

    let cursor_bridge = bridge.clone();
    entry.connect_cursor_position_notify(move |entry_widget| {
        if cursor_bridge.cursor_notify_suppressed() {
            return;
        }
        cursor_bridge.move_cursor_to(entry_widget.position());
    });

    let insert_bridge = bridge.clone();
    entry.connect_insert_text(move |entry_widget, text, _| {
        insert_bridge.handle_insert_text(text);
        entry_widget.stop_signal_emission_by_name("insert-text");
    });
}

fn html_to_pango(input: &str) -> String {
    let strip_tag = |value: String, tag: &str| {
        let open = Regex::new(&format!(r#"(?i)<{tag}[^>]*>"#)).unwrap();
        let close = Regex::new(&format!(r#"(?i)</{tag}>"#)).unwrap();
        let without_open = open.replace_all(&value, "");
        close.replace_all(&without_open, "").to_string()
    };

    let mut output = strip_tag(input.to_string(), "pre");
    output = strip_tag(output, "div");
    let font_open = Regex::new(r#"(?i)<font\s+color="([^"]+)">"#).unwrap();
    output = font_open
        .replace_all(&output, |caps: &regex::Captures| {
            format!(r#"<span foreground="{}">"#, &caps[1])
        })
        .to_string();
    let font_close = Regex::new(r"(?i)</font>").unwrap();
    output = font_close.replace_all(&output, "</span>").to_string();
    let span_style = Regex::new(r#"(?i)<span\s+style="[^"]*color:\s*([^;"\s]+)[^"]*">"#).unwrap();
    output = span_style
        .replace_all(&output, |caps: &regex::Captures| {
            format!(r#"<span foreground="{}">"#, &caps[1])
        })
        .to_string();
    let span_foreground = Regex::new(r#"(?i)<span[^>]*foreground="([^"]+)"[^>]*>"#).unwrap();
    output = span_foreground
        .replace_all(&output, |caps: &regex::Captures| {
            format!(r#"<span foreground="{}">"#, &caps[1])
        })
        .to_string();
    let span_any = Regex::new(r#"<span[^>]*>"#).unwrap();
    output = span_any
        .replace_all(&output, |caps: &regex::Captures| {
            let raw = &caps[0];
            if raw.contains("foreground=") {
                raw.to_string()
            } else {
                "<span>".to_string()
            }
        })
        .to_string();
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

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
            "<span foreground=\"#ff0000\">x</span>"
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
        assert!(bridge.handle_key(gtk::gdk::Key::Return, gtk::gdk::ModifierType::empty()));
        assert_eq!(term.fed.borrow().as_slice(), b"\n");
    }

    #[test]
    fn handle_key_allows_ctrl_c() {
        let entry = FakeEntry::default();
        let ghost = FakeLabel::default();
        let term = FakeTerminal::with_line("");
        let bridge = InputBridge::new(entry, ghost, term.clone());
        assert!(bridge.handle_key(gtk::gdk::Key::c, gtk::gdk::ModifierType::CONTROL_MASK));
        assert_eq!(term.fed.borrow().as_slice(), &[0x03]);
    }

    #[test]
    fn handle_key_plain_text_is_not_consumed() {
        let entry = FakeEntry::default();
        let ghost = FakeLabel::default();
        let term = FakeTerminal::with_line("");
        let bridge = InputBridge::new(entry, ghost, term.clone());
        assert!(!bridge.handle_key(gtk::gdk::Key::a, gtk::gdk::ModifierType::empty()));
        assert!(term.fed.borrow().is_empty());
    }

    #[test]
    fn handle_key_alt_prefix() {
        let entry = FakeEntry::default();
        let ghost = FakeLabel::default();
        let term = FakeTerminal::with_line("");
        let bridge = InputBridge::new(entry, ghost, term.clone());
        assert!(bridge.handle_key(gtk::gdk::Key::a, gtk::gdk::ModifierType::ALT_MASK));
        assert_eq!(term.fed.borrow().as_slice(), b"\x1ba");
    }

    #[test]
    fn alt_key_sets_skip_for_insert() {
        let entry = FakeEntry::default();
        let ghost = FakeLabel::default();
        let term = FakeTerminal::with_line("");
        let bridge = InputBridge::new(entry, ghost, term.clone());
        assert!(bridge.handle_key(gtk::gdk::Key::b, gtk::gdk::ModifierType::ALT_MASK));
        bridge.handle_insert_text("b");
        assert_eq!(term.fed.borrow().as_slice(), b"\x1bb");
    }

    #[test]
    fn insert_text_feeds_terminal() {
        let entry = FakeEntry::default();
        let ghost = FakeLabel::default();
        let term = FakeTerminal::with_line("");
        let bridge = InputBridge::new(entry, ghost, term.clone());
        bridge.handle_insert_text("abc");
        assert_eq!(term.fed.borrow().as_slice(), b"abc");
    }

    #[test]
    fn insert_text_is_skipped_when_syncing() {
        let entry = FakeEntry::default();
        let ghost = FakeLabel::default();
        let term = FakeTerminal::with_line("");
        let bridge = InputBridge::new(entry, ghost, term.clone());
        bridge.set_syncing_for_test(true);
        bridge.handle_insert_text("abc");
        assert!(term.fed.borrow().is_empty());
    }

    #[test]
    fn insert_text_is_skipped_when_suppressed() {
        let entry = FakeEntry::default();
        let ghost = FakeLabel::default();
        let term = FakeTerminal::with_line("");
        let bridge = InputBridge::new(entry, ghost, term.clone());
        bridge.set_suppress_insert_for_test(true);
        bridge.handle_insert_text("abc");
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
        assert_eq!(ghost.markup.borrow().as_str(), "");
    }

    #[test]
    fn arrow_keys_send_escape() {
        let entry = FakeEntry::default();
        let ghost = FakeLabel::default();
        let term = FakeTerminal::with_line("");
        let bridge = InputBridge::new(entry, ghost, term.clone());
        bridge.handle_key(gtk::gdk::Key::Left, gtk::gdk::ModifierType::empty());
        assert_eq!(term.fed.borrow().as_slice(), b"\x1b[D");
    }

    #[test]
    fn delete_sends_sequence() {
        let entry = FakeEntry::default();
        let ghost = FakeLabel::default();
        let term = FakeTerminal::with_line("");
        let bridge = InputBridge::new(entry, ghost, term.clone());
        bridge.handle_key(gtk::gdk::Key::Delete, gtk::gdk::ModifierType::empty());
        assert_eq!(term.fed.borrow().as_slice(), b"\x1b[3~");
    }

    #[test]
    fn sync_after_typing_updates_markup() {
        let entry = FakeEntry::default();
        let ghost = FakeLabel::default();
        let term = FakeTerminal::with_line("");
        let bridge = InputBridge::new(entry.clone(), ghost.clone(), term.clone());
        bridge.handle_key(gtk::gdk::Key::t, gtk::gdk::ModifierType::empty());
        term.set_line("t", Some("<font color=\"#00ff00\">t</font>"), 1);
        bridge.sync_from_terminal();
        assert_eq!(
            ghost.markup.borrow().as_str(),
            "<span foreground=\"#00ff00\">t</span>"
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

    #[test]
    #[ignore = "needs live VTE feed; keep as integration probe"]
    fn headless_cat_roundtrip() {
        gtk::init().expect("gtk init failed");
        let ctx = glib::MainContext::default();
        let _ = ctx.with_thread_default(|| {
            let terminal = Terminal::builder()
                .input_enabled(true)
                .can_focus(false)
                .build();
            terminal.set_size(80, 24);
            terminal.set_scrollback_lines(2000);
            let entry = gtk::Entry::new();
            let ghost = gtk::Label::new(None);
            let bridge = InputBridge::new(
                GtkEntryHandle::new(entry.clone()),
                GtkLabelHandle::new(ghost.clone()),
                VteTerminalAdapter::new(terminal.clone()),
            );
            bridge.attach_to_terminal(&terminal);
            wire_keys(&entry, bridge.clone());
            bridge.handle_insert_text("abc");
            bridge.handle_key(gtk::gdk::Key::Return, gtk::gdk::ModifierType::empty());
            terminal.feed(b"abc\n");
            while ctx.iteration(false) {}
            let (text, _) =
                terminal.text_range_format(vte::Format::Text, 0, 0, 0, terminal.column_count());
            let content = text.map(|t| t.to_string()).unwrap_or_default();
            assert!(content.contains("abc"), "terminal did not echo text");
        });
    }
}
