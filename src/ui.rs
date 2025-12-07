use crate::bridge::{GtkEntryHandle, GtkLabelHandle, InputBridge, VteTerminalAdapter, wire_keys};
use crate::terminal::start_shell;
use adw::Application;
use adw::prelude::*;
use gtk::{Align, glib};
use vte::Terminal;
use vte::prelude::*;

pub fn run_app() -> glib::ExitCode {
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
        .halign(Align::Start)
        .valign(Align::Center)
        .use_markup(true)
        .build();
    ghost.set_can_target(false);
    ghost.add_css_class("monospace");
    ghost.set_hexpand(true);
    ghost.set_halign(Align::Fill);

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
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    wire_keys(&entry, bridge.clone());
    attach_global_key_forwarder(&entry, &content, bridge.clone());
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

fn apply_overlay_style(entry: &gtk::Entry) {
    let css = "
        entry.overlay-input {
            background: transparent;
            border-radius: 8px;
            padding: 10px 12px;
            color: transparent;
            caret-color: transparent;
            -gtk-secondary-caret-color: transparent;
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

fn attach_global_key_forwarder(
    entry: &gtk::Entry,
    container: &gtk::Box,
    bridge: InputBridge<GtkEntryHandle, GtkLabelHandle, VteTerminalAdapter>,
) {
    let controller = gtk::EventControllerKey::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    let entry_clone = entry.clone();
    controller.connect_key_pressed(move |_ctrl, key, _code, state| {
        if entry_clone.has_focus() {
            return glib::Propagation::Proceed;
        }
        if bridge.handle_key(key, state) {
            return glib::Propagation::Stop;
        }
        if let Some(ch) = key.to_unicode() {
            let mut buf = [0u8; 4];
            let text = ch.encode_utf8(&mut buf);
            if bridge.handle_insert_text(text) {
                return glib::Propagation::Stop;
            }
        }
        glib::Propagation::Proceed
    });
    container.add_controller(controller);
}
