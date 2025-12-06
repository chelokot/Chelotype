use adw::Application;
use adw::prelude::*;
use gtk::{gio, glib};
use regex::Regex;
use std::cell::Cell;
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
        .input_enabled(false)
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

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&entry));
    overlay.add_overlay(&ghost);
    overlay.set_margin_start(10);
    overlay.set_margin_end(10);
    overlay.set_margin_top(8);
    overlay.set_margin_bottom(12);

    apply_overlay_style(&entry);
    wire_input(&entry, &ghost, &terminal);

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

    terminal.spawn_async(
        PtyFlags::DEFAULT,
        None::<&str>,
        &argv,
        &[],
        glib::SpawnFlags::SEARCH_PATH,
        || {},
        -1,
        None::<&gio::Cancellable>,
        |result| {
            if let Err(error) = result {
                eprintln!("Failed to start shell: {error}");
            }
        },
    );
}

fn wire_input(entry: &gtk::Entry, ghost: &gtk::Label, terminal: &Terminal) {
    let syncing = std::rc::Rc::new(Cell::new(false));
    sync_input(entry, ghost, terminal, &syncing);

    let entry_key_terminal = terminal.clone();
    let entry_key_entry = entry.clone();
    let entry_key_ghost = ghost.clone();
    let entry_key_sync = syncing.clone();
    entry.add_controller({
        let controller = gtk::EventControllerKey::new();
        controller.connect_key_pressed(move |_controller, key, _, state| {
            if forward_key(&entry_key_terminal, key, state) {
                sync_input(
                    &entry_key_entry,
                    &entry_key_ghost,
                    &entry_key_terminal,
                    &entry_key_sync,
                );
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        controller
    });

    let click_sync = syncing.clone();
    let click_terminal = terminal.clone();
    let click_ghost = ghost.clone();
    let click_entry = entry.clone();
    entry.connect_cursor_position_notify(move |field| {
        if click_sync.get() {
            return;
        }
        let target = field.position();
        let (current, _) = click_terminal.cursor_position();
        let current_i = i32::try_from(current).unwrap_or(0);
        let delta = target - current_i;
        if delta > 0 {
            for _ in 0..delta {
                click_terminal.feed_child(b"\x1b[C");
            }
        } else if delta < 0 {
            for _ in 0..(-delta) {
                click_terminal.feed_child(b"\x1b[D");
            }
        }
        sync_input(&click_entry, &click_ghost, &click_terminal, &click_sync);
    });

    let terminal_change_entry = entry.clone();
    let terminal_change_ghost = ghost.clone();
    let terminal_change_terminal = terminal.clone();
    let terminal_change_sync = syncing.clone();
    terminal.connect_contents_changed(move |_| {
        sync_input(
            &terminal_change_entry,
            &terminal_change_ghost,
            &terminal_change_terminal,
            &terminal_change_sync,
        );
    });
    let terminal_cursor_entry = entry.clone();
    let terminal_cursor_ghost = ghost.clone();
    let terminal_cursor_terminal = terminal.clone();
    let terminal_cursor_sync = syncing.clone();
    terminal.connect_cursor_moved(move |_| {
        sync_input(
            &terminal_cursor_entry,
            &terminal_cursor_ghost,
            &terminal_cursor_terminal,
            &terminal_cursor_sync,
        );
    });
}

fn forward_key(terminal: &Terminal, key: gtk::gdk::Key, state: gtk::gdk::ModifierType) -> bool {
    if state.contains(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::ALT_MASK) {
        return false;
    }
    let text = match key {
        gtk::gdk::Key::Return => "\n".to_string(),
        gtk::gdk::Key::BackSpace => "\u{7f}".to_string(),
        gtk::gdk::Key::Tab => "\t".to_string(),
        gtk::gdk::Key::Left => "\u{1b}[D".to_string(),
        gtk::gdk::Key::Right => "\u{1b}[C".to_string(),
        gtk::gdk::Key::Up => "\u{1b}[A".to_string(),
        gtk::gdk::Key::Down => "\u{1b}[B".to_string(),
        gtk::gdk::Key::Home => "\u{1b}[H".to_string(),
        gtk::gdk::Key::End => "\u{1b}[F".to_string(),
        gtk::gdk::Key::Delete => "\u{1b}[3~".to_string(),
        _ => {
            if let Some(ch) = key.to_unicode() {
                ch.to_string()
            } else {
                return false;
            }
        }
    };
    terminal.feed_child(text.as_bytes());
    true
}

fn sync_input(
    entry: &gtk::Entry,
    ghost: &gtk::Label,
    terminal: &Terminal,
    syncing: &std::rc::Rc<Cell<bool>>,
) {
    syncing.set(true);
    let (_, row) = terminal.cursor_position();
    let col_limit = terminal.column_count();
    let end_col = if col_limit > 0 { col_limit - 1 } else { 0 };
    let (line_html, _) = terminal.text_range_format(vte::Format::Html, row, 0, row, end_col);
    let (line_text, _) = terminal.text_range_format(vte::Format::Text, row, 0, row, end_col);

    let markup = html_to_pango(&line_html.unwrap_or_else(|| "".into()));
    ghost.set_markup(markup.as_str());

    let text = line_text.unwrap_or_else(|| "".into());
    entry.set_text(&text);

    let (cursor_col, _) = terminal.cursor_position();
    let cursor_pos = i32::try_from(cursor_col).unwrap_or(0);
    entry.set_position(cursor_pos);
    syncing.set(false);
}

fn apply_overlay_style(entry: &gtk::Entry) {
    let css = "
        entry.overlay-input {
            background: transparent;
            border-radius: 8px;
            padding: 10px 12px;
            color: @theme_fg_color;
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

fn html_to_pango(input: &str) -> String {
    let mut output = input.replace("<pre>", "").replace("</pre>", "");
    output = output.replace("<div>", "").replace("</div>", "");
    let font_open = Regex::new(r#"<font\s+color="([^"]+)">"#).unwrap();
    output = font_open
        .replace_all(&output, |caps: &regex::Captures| {
            format!(r#"<span foreground="{}">"#, &caps[1])
        })
        .to_string();
    output = output.replace("</font>", "</span>");
    let span_style = Regex::new(r#"<span\s+style="[^"]*color:\s*([^;"\s]+)[^"]*">"#).unwrap();
    output = span_style
        .replace_all(&output, |caps: &regex::Captures| {
            format!(r#"<span foreground="{}">"#, &caps[1])
        })
        .to_string();
    let span_foreground = Regex::new(r#"<span[^>]*foreground="([^"]+)"[^>]*>"#).unwrap();
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

    #[test]
    fn converts_font_color_to_span() {
        let input = r##"<pre><font color="#ff0000">abc</font></pre>"##;
        let output = html_to_pango(input);
        assert_eq!(output, r##"<span foreground="#ff0000">abc</span>"##);
    }

    #[test]
    fn strips_div_and_pre() {
        let input = "<div><pre>text</pre></div>";
        let output = html_to_pango(input);
        assert_eq!(output, "text");
    }

    #[test]
    fn converts_span_style_color() {
        let input = r##"<span style="color:#123456">val</span>"##;
        let output = html_to_pango(input);
        assert_eq!(output, r##"<span foreground="#123456">val</span>"##);
    }
}
