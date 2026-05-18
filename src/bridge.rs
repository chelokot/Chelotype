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
    fn caret_offset(&self, caret_pos: usize) -> f64;
}

#[derive(Clone)]
pub struct GtkCaretHandle {
    widget: Label,
}

impl GtkCaretHandle {
    pub fn new(widget: Label) -> Self {
        Self { widget }
    }
}

pub trait CaretHandle: Clone {
    fn set_offset(&self, x: f64);
    fn set_visible(&self, visible: bool);
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

    fn caret_offset(&self, caret_pos: usize) -> f64 {
        let layout = self.widget.layout();
        let text = layout.text();
        let mut byte_index = text.len();
        let mut chars_seen = 0;
        for (idx, _) in text.char_indices() {
            if chars_seen == caret_pos {
                byte_index = idx;
                break;
            }
            chars_seen += 1;
        }
        let pos = layout.index_to_pos(byte_index as i32);
        let half_width = pos.width() as f64 / pango::SCALE as f64 / 2.0;
        let raw = pos.x() as f64 / pango::SCALE as f64 - half_width;
        raw.max(0.0)
    }
}

impl CaretHandle for GtkCaretHandle {
    fn set_offset(&self, x: f64) {
        self.widget.set_margin_start(x.round() as i32);
    }

    fn set_visible(&self, visible: bool) {
        self.widget.set_visible(visible);
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
    fn column_count(&self) -> i64;
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

    fn column_count(&self) -> i64 {
        self.terminal.column_count()
    }
}

#[derive(Clone)]
pub struct InputBridge<
    E: EntryHandle + 'static,
    L: LabelHandle + 'static,
    C: CaretHandle + 'static,
    T: TerminalAdapter,
> {
    entry: E,
    ghost: L,
    caret: C,
    shadow_terminal: T,
    output_terminal: T,
    syncing: Rc<Cell<bool>>,
    suppress_cursor_notify: Rc<Cell<bool>>,
    skip_next_insert: Rc<Cell<bool>>,
    suppress_insert: Rc<Cell<bool>>,
    caret_visible: Rc<Cell<bool>>,
    last_markup: Rc<RefCell<Option<String>>>,
    recorded: Rc<RefCell<Vec<u8>>>,
    sync_pending: Rc<Cell<bool>>,
}

impl<
    E: EntryHandle + 'static,
    L: LabelHandle + 'static,
    C: CaretHandle + 'static,
    T: TerminalAdapter,
> InputBridge<E, L, C, T>
{
    pub fn new(entry: E, ghost: L, caret: C, shadow_terminal: T, output_terminal: T) -> Self {
        Self {
            entry,
            ghost,
            caret,
            shadow_terminal,
            output_terminal,
            syncing: Rc::new(Cell::new(false)),
            suppress_cursor_notify: Rc::new(Cell::new(false)),
            skip_next_insert: Rc::new(Cell::new(false)),
            suppress_insert: Rc::new(Cell::new(false)),
            caret_visible: Rc::new(Cell::new(true)),
            last_markup: Rc::new(RefCell::new(None)),
            recorded: Rc::new(RefCell::new(Vec::new())),
            sync_pending: Rc::new(Cell::new(false)),
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
            contents_self.queue_sync();
        });
        let cursor_self = self.clone();
        term_widget.connect_cursor_moved(move |_| {
            debug_log("terminal:cursor-moved");
            cursor_self.queue_sync();
        });

        #[cfg(not(test))]
        {
            let bridge = self.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(530), move || {
                bridge.caret_visible.set(!bridge.caret_visible.get());
                bridge.render_cached_markup();
                glib::ControlFlow::Continue
            });
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
            if data == b"\n" {
                self.submit_command();
                return true;
            }
            debug_log(&format!("feed:{data:?}"));
            self.recorded.borrow_mut().extend_from_slice(&data);
            self.shadow_terminal.feed_child(&data);
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
        self.recorded
            .borrow_mut()
            .extend_from_slice(text.as_bytes());
        self.shadow_terminal.feed_child(text.as_bytes());
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
        self.caret_visible.set(true);
        let (_, row) = self.shadow_terminal.cursor_position();
        let html = self.shadow_terminal.line_html(row);
        let text = self.shadow_terminal.line_text(row);
        let caret_pos = i32::try_from(self.shadow_terminal.cursor_position().0).unwrap_or(0);
        let base_markup = html_to_pango(&html);
        self.last_markup.replace(Some(base_markup.clone()));
        self.render_ghost_with_caret(&base_markup, caret_pos as usize);
        self.entry.set_text(&text);

        let (cursor_col, _) = self.shadow_terminal.cursor_position();
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
        debug_log(&format!("cursor:move:{target}"));
        let (_, row) = self.shadow_terminal.cursor_position();
        let max_len = self.shadow_terminal.line_text(row).chars().count() as i32;
        let clamped = target.clamp(0, max_len);
        let current = self
            .shadow_terminal
            .cursor_position()
            .0
            .try_into()
            .unwrap_or(0);
        if clamped == current {
            return;
        }
        let delta = clamped - current;
        let move_left = b"\x1b[D";
        let move_right = b"\x1b[C";
        let seq = if delta > 0 { move_right } else { move_left };
        let steps = delta.abs();
        for _ in 0..steps {
            self.shadow_terminal.feed_child(seq);
            self.recorded.borrow_mut().extend_from_slice(seq);
        }
        self.sync_from_terminal();
    }

    pub fn move_cursor_from_point(&self, x: f64, y: f64) {
        if let Some(idx) = self.ghost.index_at_x(x, y) {
            let idx_i32 = i32::try_from(idx).unwrap_or(0);
            self.move_cursor_to(idx_i32);
        }
    }

    fn render_ghost_with_caret(&self, base_markup: &str, caret_pos: usize) {
        self.ghost.set_markup(base_markup);
        let offset = self.ghost.caret_offset(caret_pos);
        self.caret.set_offset(offset);
        self.caret.set_visible(self.caret_visible.get());
    }

    #[allow(dead_code)]
    fn render_cached_markup(&self) {
        if let Some(base) = self.last_markup.borrow().as_ref() {
            let caret_pos = i32::try_from(self.shadow_terminal.cursor_position().0).unwrap_or(0);
            self.render_ghost_with_caret(base, caret_pos as usize);
        }
    }

    fn submit_command(&self) {
        let payload = self.recorded.borrow().clone();
        if !payload.is_empty() {
            self.output_terminal.feed_child(&payload);
            self.recorded.borrow_mut().clear();
        }
        self.output_terminal.feed_child(b"\n");
        self.shadow_terminal.feed_child(b"\n");
        self.caret_visible.set(true);
    }

    fn queue_sync(&self) {
        if self.sync_pending.replace(true) {
            return;
        }
        let bridge = self.clone();
        glib::idle_add_local(move || {
            bridge.sync_pending.set(false);
            bridge.sync_from_terminal();
            glib::ControlFlow::Break
        });
    }
}

pub fn wire_keys(
    entry: &Entry,
    bridge: InputBridge<GtkEntryHandle, GtkLabelHandle, GtkCaretHandle, VteTerminalAdapter>,
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
    let caret = r##"<span foreground="#7dd3fc">▏</span>"##;
    let mut plain = String::new();
    let mut in_tag = false;
    for ch in markup.chars() {
        if ch == '<' {
            in_tag = true;
        }
        if !in_tag {
            plain.push(ch);
        }
        if ch == '>' {
            in_tag = false;
        }
    }
    let chars: Vec<char> = plain.chars().collect();
    let split = caret_pos.min(chars.len());
    let (left, right) = chars.split_at(split);
    let mut rendered = String::new();
    rendered.push_str(&left.iter().collect::<String>());
    rendered.push_str(caret);
    rendered.push_str(&left.iter().collect::<String>());
    rendered.push_str(&right.iter().collect::<String>());
    rendered
}
