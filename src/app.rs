use crate::backend::{ScreenSize, TerminalBackend};
use crate::input::{KeyAction, key_to_action};
use crate::interaction::{InteractionEffect, PointerInteraction};
use crate::mouse::{MouseButton, MouseGridPosition};
use crate::render::Renderer;
use crate::selection::SelectionRange;
use crate::snapshot::write_snapshot;
use adw::Application;
use adw::prelude::*;
use gtk::pango;
use gtk::{Align, glib};

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

    let history = gtk::Label::builder()
        .halign(Align::Start)
        .valign(Align::Start)
        .use_markup(true)
        .wrap(true)
        .wrap_mode(pango::WrapMode::Char)
        .build();
    history.add_css_class("monospace");
    history.add_css_class("term-history");
    history.set_wrap(false);

    let input = gtk::Label::builder()
        .halign(Align::Start)
        .valign(Align::Start)
        .use_markup(true)
        .build();
    input.add_css_class("monospace");
    input.add_css_class("term-input");
    input.set_wrap(false);

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&history));
    overlay.add_overlay(&input);
    let caret = gtk::Label::new(Some("│"));
    caret.add_css_class("monospace");
    caret.add_css_class("overlay-caret");
    caret.set_visible(false);
    overlay.add_overlay(&caret);
    overlay.set_margin_start(14);
    overlay.set_margin_end(10);
    overlay.set_margin_top(8);
    overlay.set_margin_bottom(12);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("term-root");
    content.append(&header);
    content.append(&overlay);

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Chelotype Terminal")
        .default_width(1100)
        .default_height(760)
        .content(&content)
        .build();

    let backend = TerminalBackend::spawn_shell().expect("spawn shell");
    let backend_rc = std::rc::Rc::new(std::cell::RefCell::new(backend));
    let caret_widget = std::rc::Rc::new(caret);
    apply_style(&history, &input, caret_widget.as_ref());
    let snapshot_enabled = std::env::var("CHELOTYPE_SNAPSHOT").ok().as_deref() == Some("1");
    let last_snapshot = std::rc::Rc::new(std::cell::RefCell::new(std::time::Instant::now()));
    let last_size = std::rc::Rc::new(std::cell::Cell::new(None::<ScreenSize>));
    let cell_metrics = std::rc::Rc::new(std::cell::Cell::new(None::<CellMetrics>));
    let mouse_mode = std::rc::Rc::new(std::cell::Cell::new(crate::backend::MouseMode::default()));
    let pointer_interaction =
        std::rc::Rc::new(std::cell::RefCell::new(PointerInteraction::default()));
    let selection = std::rc::Rc::new(std::cell::Cell::new(None::<SelectionRange>));

    let key_controller = gtk::EventControllerKey::new();
    key_controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let backend = backend_rc.clone();
        key_controller.connect_key_pressed(move |_ctrl, key, _code, state| {
            if let Some(action) = key_to_action(key, state) {
                match action {
                    KeyAction::Write(data) => {
                        let _ = backend.borrow_mut().write(&data);
                    }
                    KeyAction::ScrollDisplay(lines) => {
                        let _ = backend.borrow().scroll_display(lines);
                    }
                }
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    window.add_controller(key_controller);

    let click_controller = gtk::GestureClick::new();
    click_controller.set_button(0);
    {
        let backend = backend_rc.clone();
        let metrics = cell_metrics.clone();
        let mode = mouse_mode.clone();
        let selection = selection.clone();
        let pointer_interaction = pointer_interaction.clone();
        click_controller.connect_pressed(move |gesture, _press_count, x, y| {
            if let Some(position) = pointer_grid_position(metrics.get(), x, y) {
                let button = mouse_button_from_gesture(gesture).unwrap_or(MouseButton::Left);
                let effects = pointer_interaction
                    .borrow_mut()
                    .press(mode.get(), button, position);
                apply_interaction_effects(effects, &backend, &selection);
            }
        });
    }
    {
        let backend = backend_rc.clone();
        let metrics = cell_metrics.clone();
        let mode = mouse_mode.clone();
        let selection = selection.clone();
        let pointer_interaction = pointer_interaction.clone();
        click_controller.connect_released(move |_gesture, _press_count, x, y| {
            if let Some(position) = pointer_grid_position(metrics.get(), x, y) {
                let effects = pointer_interaction
                    .borrow_mut()
                    .release(mode.get(), position);
                apply_interaction_effects(effects, &backend, &selection);
            }
        });
    }
    overlay.add_controller(click_controller);

    let motion_controller = gtk::EventControllerMotion::new();
    {
        let backend = backend_rc.clone();
        let metrics = cell_metrics.clone();
        let mode = mouse_mode.clone();
        let selection = selection.clone();
        let pointer_interaction = pointer_interaction.clone();
        motion_controller.connect_motion(move |_controller, x, y| {
            if let Some(position) = pointer_grid_position(metrics.get(), x, y) {
                let effects = pointer_interaction
                    .borrow_mut()
                    .motion(mode.get(), position);
                apply_interaction_effects(effects, &backend, &selection);
            }
        });
    }
    overlay.add_controller(motion_controller);

    let scroll_controller =
        gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    {
        let backend = backend_rc.clone();
        scroll_controller.connect_scroll(move |_controller, _dx, dy| {
            if let Some(lines) = crate::interaction::wheel_scroll_lines(dy) {
                let _ = backend.borrow().scroll_display(lines);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    overlay.add_controller(scroll_controller);

    glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
        let measured_metrics = terminal_metrics_for_overlay(&overlay, &input);
        cell_metrics.set(measured_metrics.map(|metrics| metrics.cell));
        if let Some(size) = measured_metrics.map(|metrics| metrics.size)
            && last_size.get() != Some(size)
        {
            if backend_rc.borrow().resize(size).is_ok() {
                last_size.set(Some(size));
            }
        }
        if let Some(content) = backend_rc.borrow().snapshot_renderable() {
            mouse_mode.set(content.mouse);
            let rendered = Renderer::render_with_selection(content.clone(), selection.get());
            history.set_markup(&rendered.history_markup);
            input.set_markup(&rendered.input_markup);
            if rendered.cursor.visible {
                caret_widget.set_visible(true);
                let layout = input.layout();
                let logical = layout.extents().1;
                let line_height = logical.height() as f64 / pango::SCALE as f64;
                let top = (rendered.cursor.line as f64 * line_height).round() as i32;
                input.set_margin_top(top);
                let byte_index = byte_index_for_char(
                    &rendered.input_text,
                    rendered.cursor.column.max(0) as usize,
                );
                let caret_pos = layout.index_to_pos(byte_index as i32);
                let caret_x = caret_pos.x() as f64 / pango::SCALE as f64;
                caret_widget.set_margin_start(caret_x.round() as i32);
                caret_widget.set_margin_top(top);
                caret_widget.set_markup(&format!(
                    "<span foreground=\"{}\">│</span>",
                    rendered.cursor.color
                ));
            } else {
                caret_widget.set_visible(false);
            }
            if snapshot_enabled
                && last_snapshot.borrow().elapsed() >= std::time::Duration::from_secs(1)
            {
                *last_snapshot.borrow_mut() = std::time::Instant::now();
                let _ = write_snapshot(content, "frame");
            }
        }
        glib::ControlFlow::Continue
    });

    window.present();
}

fn apply_style(history: &gtk::Label, _input: &gtk::Label, _caret: &gtk::Label) {
    let css = "
        .term-root {
            background: #0f1115;
        }
        label.term-history, label.term-input {
            font-family: 'JetBrains Mono', monospace;
            font-size: 13px;
            color: #e5e7eb;
            background: transparent;
        }
        label.overlay-caret {
            font-family: 'JetBrains Mono', monospace;
            font-size: 13px;
            color: #7dd3fc;
            padding: 0;
            margin: 0;
        }
    ";
    let provider = gtk::CssProvider::new();
    provider.load_from_data(css);
    gtk::style_context_add_provider_for_display(
        &history.display(),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn apply_interaction_effects(
    effects: Vec<InteractionEffect>,
    backend: &std::rc::Rc<std::cell::RefCell<TerminalBackend>>,
    selection: &std::rc::Rc<std::cell::Cell<Option<SelectionRange>>>,
) {
    for effect in effects {
        match effect {
            InteractionEffect::Write(bytes) => {
                let _ = backend.borrow_mut().write(&bytes);
            }
            InteractionEffect::SelectionChanged(range) => selection.set(range),
        }
    }
}

fn byte_index_for_char(text: &str, caret_col: usize) -> usize {
    let mut idx = 0;
    let mut chars_seen = 0;
    for (byte_idx, _) in text.char_indices() {
        if chars_seen == caret_col {
            idx = byte_idx;
            break;
        }
        chars_seen += 1;
        idx = byte_idx + 1;
    }
    if chars_seen < caret_col {
        text.len()
    } else {
        idx
    }
}

#[derive(Clone, Copy)]
struct TerminalMetrics {
    size: ScreenSize,
    cell: CellMetrics,
}

#[derive(Clone, Copy)]
struct CellMetrics {
    width: i32,
    height: i32,
}

fn terminal_metrics_for_overlay(
    overlay: &gtk::Overlay,
    input: &gtk::Label,
) -> Option<TerminalMetrics> {
    let width = overlay.allocated_width();
    let height = overlay.allocated_height();
    if width <= 0 || height <= 0 {
        return None;
    }
    let metrics = input.pango_context().metrics(
        Some(&pango::FontDescription::from_string("JetBrains Mono 13")),
        None,
    );
    let char_width = metrics.approximate_char_width() / pango::SCALE;
    let line_height = metrics.height() / pango::SCALE;
    if char_width <= 0 || line_height <= 0 {
        return None;
    }
    let cols = (width / char_width).clamp(1, u16::MAX as i32) as u16;
    let rows = (height / line_height).clamp(1, u16::MAX as i32) as u16;
    Some(TerminalMetrics {
        size: ScreenSize::new(cols, rows).ok()?,
        cell: CellMetrics {
            width: char_width,
            height: line_height,
        },
    })
}

fn pointer_grid_position(
    metrics: Option<CellMetrics>,
    x: f64,
    y: f64,
) -> Option<MouseGridPosition> {
    let metrics = metrics?;
    if x < 0.0 || y < 0.0 || metrics.width <= 0 || metrics.height <= 0 {
        return None;
    }
    Some(MouseGridPosition {
        column: (x as i32 / metrics.width).clamp(0, u16::MAX as i32) as u16,
        row: (y as i32 / metrics.height).clamp(0, u16::MAX as i32) as u16,
    })
}

fn mouse_button_from_gesture(gesture: &gtk::GestureClick) -> Option<MouseButton> {
    match gesture.current_button() {
        1 => Some(MouseButton::Left),
        2 => Some(MouseButton::Middle),
        3 => Some(MouseButton::Right),
        _ => None,
    }
}
