use crate::logging::debug_log;
use gtk::gdk::ModifierType;
use gtk::{Entry, Label, glib, pango};
use regex::Regex;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use vte::Terminal;
use vte::prelude::*;

#[derive(Clone)]
pub struct GtkEntryHandle {
    widget: Entry,
}

impl GtkEntryHandle {
    pub fn new(widget: Entry) -> Self {
        Self { widget }
    }

    pub fn widget(&self) -> &Entry {
        &self.widget
    }
}

#[derive(Clone)]
pub struct GtkLabelHandle {
    widget: Label,
}

impl GtkLabelHandle {
    pub fn new(widget: Label) -> Self {
        Self { widget }
    }
}

pub trait EntryHandle: Clone {
    fn set_text(&self, text: &str);
    fn set_position(&self, pos: i32);
}

pub trait LabelHandle: Clone {
    fn set_markup(&self, markup: &str);
    fn index_at_x(&self, x: f64, y: f64) -> Option<usize>;
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

    fn index_at_x(&self, x: f64, y: f64) -> Option<usize> {
        let layout = self.widget.layout();
        let (_, byte_idx, trailing) = layout.xy_to_index(
            (x * pango::SCALE as f64) as i32,
            (y * pango::SCALE as f64) as i32,
        );
        let text = layout.text();
        let clamped = byte_idx.min(i32::try_from(text.len()).unwrap_or(0)) as usize;
        let caret_chars = text[..clamped].chars().count() + trailing as usize;
        Some(caret_chars)
    }
}

#[derive(Clone)]
pub struct VteTerminalAdapter {
    terminal: Terminal,
}

impl VteTerminalAdapter {
    pub fn new(terminal: Terminal) -> Self {
        Self { terminal }
    }

    pub fn widget(&self) -> &Terminal {
        &self.terminal
    }
}

pub trait TerminalAdapter: Clone + 'static {
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
pub struct InputBridge<E: EntryHandle + 'static, L: LabelHandle + 'static, T: TerminalAdapter> {
    entry: E,
    ghost: L,
    terminal: T,
    syncing: Rc<Cell<bool>>,
    suppress_cursor_notify: Rc<Cell<bool>>,
    skip_next_insert: Rc<Cell<bool>>,
    suppress_insert: Rc<Cell<bool>>,
    caret_visible: Rc<Cell<bool>>,
    last_markup: Rc<RefCell<Option<String>>>,
    #[cfg(not(test))]
    blink_source: Rc<RefCell<Option<glib::SourceId>>>,
}

impl<E: EntryHandle + 'static, L: LabelHandle + 'static, T: TerminalAdapter> InputBridge<E, L, T> {
    pub fn new(entry: E, ghost: L, terminal: T) -> Self {
        Self {
            entry,
            ghost,
            terminal,
            syncing: Rc::new(Cell::new(false)),
            suppress_cursor_notify: Rc::new(Cell::new(false)),
            skip_next_insert: Rc::new(Cell::new(false)),
            suppress_insert: Rc::new(Cell::new(false)),
            caret_visible: Rc::new(Cell::new(true)),
            last_markup: Rc::new(RefCell::new(None)),
            #[cfg(not(test))]
            blink_source: Rc::new(RefCell::new(None)),
        }
    }

    pub fn set_syncing_for_test(&self, value: bool) {
        self.syncing.set(value);
    }

    pub fn set_suppress_insert_for_test(&self, value: bool) {
        self.suppress_insert.set(value);
    }

    pub fn cursor_notify_suppressed(&self) -> bool {
        self.suppress_cursor_notify.get()
    }

    pub fn attach_to_terminal(&self, term_widget: &Terminal) {
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

        #[cfg(not(test))]
        {
            self.restart_blink();
        }
    }

    pub fn handle_key(&self, key: gtk::gdk::Key, state: ModifierType) -> bool {
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
                let ctrl = state.contains(ModifierType::CONTROL_MASK);
                let alt = state.intersects(
                    ModifierType::ALT_MASK | ModifierType::META_MASK | ModifierType::SUPER_MASK,
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

    pub fn handle_insert_text(&self, text: &str) -> bool {
        if self.suppress_insert.get() {
            debug_log("insert:skip-syncing");
            return false;
        }
        if self.skip_next_insert.replace(false) {
            debug_log("insert:skip");
            return false;
        }
        if text.is_empty() {
            return false;
        }
        debug_log(&format!("insert:{text}"));
        self.terminal.feed_child(text.as_bytes());
        true
    }

    pub fn sync_from_terminal(&self) {
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
        let caret_pos = i32::try_from(self.terminal.cursor_position().0).unwrap_or(0);
        let base_markup = html_to_pango(&html);
        self.last_markup.replace(Some(base_markup.clone()));
        self.reset_caret_visible();
        self.render_ghost_with_caret(&base_markup, caret_pos as usize);
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

    pub fn move_cursor_to(&self, target: i32) {
        if self.syncing.get() || self.suppress_cursor_notify.get() {
            return;
        }
        self.reset_caret_visible();
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

    pub fn move_cursor_from_point(&self, x: f64, y: f64) {
        if let Some(idx) = self.ghost.index_at_x(x, y) {
            let idx_i32 = i32::try_from(idx).unwrap_or(0);
            self.move_cursor_to(idx_i32);
            self.reset_caret_visible();
        }
    }

    fn render_ghost_with_caret(&self, base_markup: &str, caret_pos: usize) {
        let markup = if self.caret_visible.get() {
            insert_caret(base_markup, caret_pos)
        } else {
            base_markup.to_string()
        };
        self.ghost.set_markup(markup.as_str());
    }

    fn render_cached_markup(&self) {
        if let Some(base) = self.last_markup.borrow().as_ref() {
            let caret_pos = i32::try_from(self.terminal.cursor_position().0).unwrap_or(0);
            self.render_ghost_with_caret(base, caret_pos as usize);
        }
    }

    fn reset_caret_visible(&self) {
        self.caret_visible.set(true);
        self.render_cached_markup();
        #[cfg(not(test))]
        self.restart_blink();
    }

    #[cfg(not(test))]
    fn restart_blink(&self) {
        self.blink_source.borrow_mut().take().map(|id| id.remove());
        let bridge = self.clone();
        let id = glib::timeout_add_local(std::time::Duration::from_millis(530), move || {
            bridge.caret_visible.set(!bridge.caret_visible.get());
            bridge.render_cached_markup();
            glib::ControlFlow::Continue
        });
        self.blink_source.replace(Some(id));
    }
}

pub fn wire_keys(
    entry: &Entry,
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
        if insert_bridge.handle_insert_text(text) {
            entry_widget.stop_signal_emission_by_name("insert-text");
        }
    });
}

pub fn html_to_pango(input: &str) -> String {
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
    let underline_any = Regex::new(r#"(?i)<u[^>]*>"#).unwrap();
    output = underline_any.replace_all(&output, "<u>").to_string();
    output
}

pub fn insert_caret(markup: &str, caret_pos: usize) -> String {
    let caret = r##"<span foreground="#7dd3fc" letter_spacing="-9000">|</span>"##;
    let mut result = String::new();
    let mut in_tag = false;
    let mut pos = 0usize;
    for ch in markup.chars() {
        if ch == '<' {
            in_tag = true;
        }
        if !in_tag && pos == caret_pos {
            result.push_str(caret);
        }
        if !in_tag {
            pos += 1;
        }
        result.push(ch);
        if ch == '>' {
            in_tag = false;
        }
    }
    if caret_pos >= pos {
        result.push_str(caret);
    }
    result
}
