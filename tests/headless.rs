use chelotype::bridge::{
    GtkEntryHandle, GtkLabelHandle, InputBridge, VteTerminalAdapter, wire_keys,
};
use chelotype::terminal::spawn_process;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use vte::prelude::*;

#[test]
fn headless_cat_roundtrip() {
    gtk::init().expect("gtk init failed");
    let ctx = glib::MainContext::default();
    let _ = ctx.with_thread_default(|| {
        let terminal = vte::Terminal::builder()
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
        spawn_process(&terminal, &["/bin/cat"]);

        let captured = Rc::new(RefCell::new(String::new()));
        let captured_clone = captured.clone();
        let done = Rc::new(Cell::new(false));
        let done_clone = done.clone();
        terminal.connect_contents_changed(move |term| {
            let (text, _) =
                term.text_range_format(vte::Format::Text, 0, 0, 10, term.column_count());
            if let Some(txt) = text {
                *captured_clone.borrow_mut() = txt.to_string();
                if captured_clone.borrow().contains("abc") {
                    done_clone.set(true);
                }
            }
        });

        glib::idle_add_local({
            let bridge = bridge.clone();
            let terminal = terminal.clone();
            move || {
                let fed = bridge.handle_insert_text("abc");
                assert!(fed, "insert was skipped");
                bridge.handle_key(gdk::Key::Return, gdk::ModifierType::empty());
                terminal.feed(b"abc\n");
                glib::ControlFlow::Break
            }
        });

        let loop_ref = glib::MainLoop::new(Some(&ctx), false);
        let loop_clone = loop_ref.clone();
        let done_check = done.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            if done_check.get() {
                loop_clone.quit();
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
        glib::timeout_add_local(std::time::Duration::from_secs(3), {
            let loop_clone = loop_ref.clone();
            move || {
                loop_clone.quit();
                glib::ControlFlow::Break
            }
        });
        loop_ref.run();

        assert!(
            done.get(),
            "terminal did not echo text; buffer: {}",
            captured.borrow()
        );
    });
}
