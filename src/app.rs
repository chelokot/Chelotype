use crate::backend::{ScreenSize, TerminalBackend};
use crate::canvas::TerminalCanvas;
use crate::input::{KeyAction, key_to_action};
use crate::interaction::{InteractionEffect, PointerInteraction};
use crate::mouse::{MouseButton, MouseGridPosition};
use crate::render::Renderer;
use crate::selection::SelectionRange;
use adw::Application;
use adw::prelude::*;
use gtk::glib;
use gtk::pango;

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

    let canvas = TerminalCanvas::new();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("term-root");
    content.append(&header);
    content.append(canvas.widget());

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Chelotype Terminal")
        .default_width(1100)
        .default_height(760)
        .content(&content)
        .build();

    let backend = TerminalBackend::spawn_shell().expect("spawn shell");
    let backend_rc = std::rc::Rc::new(std::cell::RefCell::new(backend));
    apply_style(canvas.widget());
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
                        let _ = backend.borrow_mut().scroll_display(lines);
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
    canvas.widget().add_controller(click_controller);

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
    canvas.widget().add_controller(motion_controller);

    let scroll_controller =
        gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    {
        let backend = backend_rc.clone();
        scroll_controller.connect_scroll(move |_controller, _dx, dy| {
            if let Some(lines) = crate::interaction::wheel_scroll_lines(dy) {
                let _ = backend.borrow_mut().scroll_display(lines);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    canvas.widget().add_controller(scroll_controller);

    glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
        let measured_metrics = terminal_metrics_for_widget(canvas.widget());
        cell_metrics.set(measured_metrics.map(|metrics| metrics.cell));
        if let Some(size) = measured_metrics.map(|metrics| metrics.size)
            && last_size.get() != Some(size)
            && backend_rc.borrow_mut().resize(size).is_ok()
        {
            last_size.set(Some(size));
        }
        if let Some(content) = backend_rc.borrow_mut().snapshot_renderable() {
            mouse_mode.set(content.mouse);
            let rendered = Renderer::render_frame_with_selection(content.clone(), selection.get());
            canvas.set_render(rendered);
            if snapshot_enabled
                && last_snapshot.borrow().elapsed() >= std::time::Duration::from_secs(1)
            {
                *last_snapshot.borrow_mut() = std::time::Instant::now();
                let _ = crate::snapshot::write_snapshot(content, "frame");
            }
        }
        glib::ControlFlow::Continue
    });

    window.present();
}

fn apply_style(canvas: &gtk::DrawingArea) {
    let css = "
        .term-root {
            background: #0f1115;
        }
        drawingarea.term-canvas {
            color: #e5e7eb;
            background: transparent;
        }
    ";
    let provider = gtk::CssProvider::new();
    provider.load_from_data(css);
    gtk::style_context_add_provider_for_display(
        &canvas.display(),
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

fn terminal_metrics_for_widget(widget: &gtk::DrawingArea) -> Option<TerminalMetrics> {
    let width = widget.allocated_width();
    let height = widget.allocated_height();
    if width <= 0 || height <= 0 {
        return None;
    }
    let metrics = widget.pango_context().metrics(
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
