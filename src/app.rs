use crate::backend::{MouseMode, RenderableContentOwned, ScreenSize};
use crate::canvas::TerminalCanvas;
use crate::cell_text::lines_to_text;
use crate::containers::{LaunchTarget, available_launch_targets};
use crate::input::{CursorDirection, CursorUnit, KeyAction, key_to_action};
use crate::interaction::{
    InteractionEffect, PointerInteraction, cursor_movement_bytes_for_content,
};
use crate::mouse::{MouseButton, MouseGridPosition};
use crate::render::Renderer;
use crate::selection::{
    GridPoint, SelectionRange, anchor_range_to_display, line_range, line_significant_len,
    selected_text, viewport_range_for_display, word_range_at,
};
use crate::snapshot::write_snapshot_with_selection;
use crate::terminal_font::metrics_for_widget;
use crate::workspace::{PaneId, TerminalWorkspace};
use adw::Application;
use adw::prelude::*;
use gtk::{gio, glib};

pub fn run_app() -> glib::ExitCode {
    let app = Application::builder()
        .application_id("com.chelotype.Terminal")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &Application) {
    crate::allocation_trace::configure_from_env();
    crate::terminal_font::load_configured_size();
    let workspace = TerminalWorkspace::spawn_shell().expect("spawn terminal workspace");
    let workspace_rc = std::rc::Rc::new(std::cell::RefCell::new(workspace));
    let force_snapshot = std::rc::Rc::new(std::cell::Cell::new(true));

    let tab_view = adw::TabView::new();
    tab_view.set_visible(false);
    tab_view.set_menu_model(Some(&tab_menu_model()));
    let tab_bar = adw::TabBar::new();
    tab_bar.add_css_class("term-tab-bar");
    tab_bar.set_autohide(true);
    tab_bar.set_expand_tabs(true);
    tab_bar.set_hexpand(true);
    tab_bar.set_halign(gtk::Align::Fill);
    tab_bar.set_view(Some(&tab_view));
    let tab_pages = std::rc::Rc::new(std::cell::RefCell::new(TabPages::default()));
    let new_tab_button = gtk::Button::builder()
        .icon_name("tab-new-symbolic")
        .tooltip_text("New terminal")
        .build();
    new_tab_button.set_valign(gtk::Align::Center);
    new_tab_button.add_css_class("image-button");
    let launch_menu_button = gtk::MenuButton::builder()
        .icon_name("pan-down-symbolic")
        .tooltip_text("Open terminal in container")
        .build();
    launch_menu_button.set_valign(gtk::Align::Center);
    launch_menu_button.add_css_class("image-button");
    launch_menu_button.add_css_class("terminal-launch-menu-button");
    let launcher = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    launcher.set_valign(gtk::Align::Center);
    launcher.add_css_class("terminal-launcher");
    launcher.append(&new_tab_button);
    launcher.append(&launch_menu_button);
    let header_content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header_content.add_css_class("terminal-header");
    header_content.append(&launcher);
    header_content.append(&tab_bar);
    header_content.append(&gtk::WindowControls::new(gtk::PackType::End));

    let header = gtk::WindowHandle::new();
    header.set_child(Some(&header_content));

    let canvas = TerminalCanvas::new();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("term-root");
    content.append(&header);
    content.append(canvas.widget());
    content.append(&tab_view);

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Chelotype Terminal")
        .default_width(1100)
        .default_height(760)
        .content(&content)
        .build();

    apply_style(canvas.widget());
    let snapshot_enabled = std::env::var("CHELOTYPE_SNAPSHOT").ok().as_deref() == Some("1");
    let last_snapshot = std::rc::Rc::new(std::cell::RefCell::new(std::time::Instant::now()));
    let ui_e2e = UiE2eScenario::from_env();
    let ui_e2e_deadline = ui_e2e
        .as_ref()
        .map(|scenario| std::time::Instant::now() + scenario.timeout);
    let last_size = std::rc::Rc::new(std::cell::Cell::new(None::<ScreenSize>));
    let cell_metrics = std::rc::Rc::new(std::cell::Cell::new(None::<CellMetrics>));
    let geometry_trace = std::env::var("CHELOTYPE_GEOMETRY_TRACE")
        .ok()
        .map(std::path::PathBuf::from);
    let mouse_mode = std::rc::Rc::new(std::cell::Cell::new(crate::backend::MouseMode::default()));
    let pointer_interaction =
        std::rc::Rc::new(std::cell::RefCell::new(PointerInteraction::default()));
    let drag_gesture_moved = std::rc::Rc::new(std::cell::Cell::new(false));
    let selection = std::rc::Rc::new(std::cell::Cell::new(None::<SelectionRange>));
    let selection_text = std::rc::Rc::new(std::cell::RefCell::new(None::<String>));
    let selection_dirty = std::rc::Rc::new(std::cell::Cell::new(false));
    let last_content = std::rc::Rc::new(std::cell::RefCell::new(None::<RenderableContentOwned>));
    add_existing_workspace_pages(&tab_view, &tab_pages, &workspace_rc.borrow());
    let tab_context = TabContext {
        workspace: workspace_rc.clone(),
        tab_view: tab_view.clone(),
        tab_pages: tab_pages.clone(),
        force_snapshot: force_snapshot.clone(),
        selection: selection.clone(),
        selection_text: selection_text.clone(),
        selection_dirty: selection_dirty.clone(),
    };

    configure_launch_menu(&launch_menu_button, tab_context.clone(), last_size.clone());
    install_tab_actions(&window, &tab_bar, tab_context.clone(), last_size.clone());
    connect_native_tabs(&tab_view, tab_context.clone(), last_size.clone());
    {
        let tabs = tab_context.clone();
        let last_size = last_size.clone();
        new_tab_button.connect_clicked(move |_| {
            add_launch_target_pane(
                LaunchTarget::Host,
                &LaunchMenuContext {
                    tabs: tabs.clone(),
                    last_size: last_size.clone(),
                    popover: gtk::Popover::new(),
                },
            );
        });
    }

    let key_controller = gtk::EventControllerKey::new();
    key_controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let workspace = workspace_rc.clone();
        let content = last_content.clone();
        let selection = selection.clone();
        let selection_text = selection_text.clone();
        let selection_dirty = selection_dirty.clone();
        let canvas_widget = canvas.widget().clone();
        let force_snapshot = force_snapshot.clone();
        let tab_view = tab_view.clone();
        let tab_pages = tab_pages.clone();
        let last_size = last_size.clone();
        key_controller.connect_key_pressed(move |_ctrl, key, _code, state| {
            if let Some(action) = key_to_action(key, state) {
                match action {
                    KeyAction::Write(data) => {
                        if data.as_slice() == [0x03] && selection_text.borrow().is_some() {
                            copy_selection_to_clipboard(&canvas_widget, &selection_text);
                        } else {
                            write_key_with_selection(
                                &workspace,
                                &content,
                                &selection,
                                &selection_text,
                                &selection_dirty,
                                data,
                            );
                        }
                    }
                    KeyAction::CursorMove {
                        direction,
                        unit,
                        selecting,
                    } => {
                        move_cursor_from_keyboard(
                            &workspace,
                            &content,
                            &selection,
                            &selection_text,
                            &selection_dirty,
                            KeyboardMove {
                                direction,
                                unit,
                                selecting,
                            },
                        );
                    }
                    KeyAction::SelectInput => {
                        select_active_input(
                            &content,
                            &selection,
                            &selection_text,
                            &selection_dirty,
                        );
                    }
                    KeyAction::ScrollDisplay(lines) => {
                        let _ = workspace.borrow_mut().scroll_active(lines);
                    }
                    KeyAction::CopySelection => {
                        copy_selection_to_clipboard(&canvas_widget, &selection_text);
                    }
                    KeyAction::CutSelection => {
                        if copy_selection_to_clipboard(&canvas_widget, &selection_text) {
                            write_key_with_selection(
                                &workspace,
                                &content,
                                &selection,
                                &selection_text,
                                &selection_dirty,
                                b"\x1b[3~".to_vec(),
                            );
                        }
                    }
                    KeyAction::PasteClipboard => {
                        paste_clipboard_text(
                            &canvas_widget,
                            workspace.clone(),
                            content.clone(),
                            selection.clone(),
                            selection_text.clone(),
                            selection_dirty.clone(),
                        );
                    }
                    KeyAction::ZoomIn => {
                        crate::terminal_font::zoom_in();
                        last_size.set(None);
                        force_snapshot.set(true);
                        canvas_widget.queue_draw();
                    }
                    KeyAction::ZoomOut => {
                        crate::terminal_font::zoom_out();
                        last_size.set(None);
                        force_snapshot.set(true);
                        canvas_widget.queue_draw();
                    }
                    KeyAction::ZoomReset => {
                        crate::terminal_font::zoom_reset();
                        last_size.set(None);
                        force_snapshot.set(true);
                        canvas_widget.queue_draw();
                    }
                    KeyAction::NewPane => {
                        add_launch_target_pane(
                            LaunchTarget::Host,
                            &LaunchMenuContext {
                                tabs: TabContext {
                                    workspace: workspace.clone(),
                                    tab_view: tab_view.clone(),
                                    tab_pages: tab_pages.clone(),
                                    force_snapshot: force_snapshot.clone(),
                                    selection: selection.clone(),
                                    selection_text: selection_text.clone(),
                                    selection_dirty: selection_dirty.clone(),
                                },
                                last_size: last_size.clone(),
                                popover: gtk::Popover::new(),
                            },
                        );
                    }
                    KeyAction::NextPane => {
                        workspace.borrow_mut().activate_next();
                        let active = workspace.borrow().active_pane_id();
                        select_tab_for_pane(&tab_view, &tab_pages, active);
                        force_active_workspace_snapshot(
                            &force_snapshot,
                            &selection,
                            &selection_text,
                            &selection_dirty,
                            last_size.get(),
                            &workspace,
                        );
                    }
                    KeyAction::PreviousPane => {
                        workspace.borrow_mut().activate_previous();
                        let active = workspace.borrow().active_pane_id();
                        select_tab_for_pane(&tab_view, &tab_pages, active);
                        force_active_workspace_snapshot(
                            &force_snapshot,
                            &selection,
                            &selection_text,
                            &selection_dirty,
                            last_size.get(),
                            &workspace,
                        );
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
        let workspace = workspace_rc.clone();
        let metrics = cell_metrics.clone();
        let mode = mouse_mode.clone();
        let selection = selection.clone();
        let selection_text = selection_text.clone();
        let selection_dirty = selection_dirty.clone();
        let content = last_content.clone();
        let pointer_interaction = pointer_interaction.clone();
        let drag_gesture_moved = drag_gesture_moved.clone();
        let canvas_widget = canvas.widget().clone();
        click_controller.connect_pressed(move |gesture, press_count, x, y| {
            canvas_widget.grab_focus();
            drag_gesture_moved.set(false);
            crate::logging::debug_log(&format!("mouse press x={x:.1} y={y:.1}"));
            if let Some(position) = pointer_grid_position(metrics.get(), x, y) {
                crate::logging::debug_log(&format!(
                    "mouse press grid col={} row={}",
                    position.column, position.row
                ));
                if !mode.get().sends_press_release()
                    && select_mouse_click_range(
                        &content,
                        &selection,
                        &selection_text,
                        &selection_dirty,
                        position,
                        press_count,
                    )
                {
                    drag_gesture_moved.set(true);
                    let _ = pointer_interaction.borrow_mut().cancel();
                    return;
                }
                let button = mouse_button_from_gesture(gesture).unwrap_or(MouseButton::Left);
                let effects = pointer_interaction
                    .borrow_mut()
                    .press(mode.get(), button, position);
                crate::logging::debug_log(&format!("mouse press effects={effects:?}"));
                apply_interaction_effects(
                    effects,
                    &workspace,
                    &selection,
                    &selection_text,
                    &selection_dirty,
                    &content,
                );
            }
        });
    }
    {
        let workspace = workspace_rc.clone();
        let metrics = cell_metrics.clone();
        let mode = mouse_mode.clone();
        let selection = selection.clone();
        let selection_text = selection_text.clone();
        let selection_dirty = selection_dirty.clone();
        let content = last_content.clone();
        let pointer_interaction = pointer_interaction.clone();
        let drag_gesture_moved = drag_gesture_moved.clone();
        click_controller.connect_released(move |_gesture, _press_count, x, y| {
            crate::logging::debug_log(&format!("mouse release x={x:.1} y={y:.1}"));
            if drag_gesture_moved.get() && !mode.get().sends_press_release() {
                let effects = pointer_interaction.borrow_mut().cancel();
                crate::logging::debug_log(&format!(
                    "mouse release after drag gesture effects={effects:?}"
                ));
                apply_interaction_effects(
                    effects,
                    &workspace,
                    &selection,
                    &selection_text,
                    &selection_dirty,
                    &content,
                );
                return;
            }
            if let Some(cell_position) = pointer_grid_position(metrics.get(), x, y) {
                let position = if mode.get().sends_press_release() {
                    cell_position
                } else {
                    pointer_cursor_position(metrics.get(), x, y).unwrap_or(cell_position)
                };
                crate::logging::debug_log(&format!(
                    "mouse release grid col={} row={}",
                    position.column, position.row
                ));
                let effects = pointer_interaction
                    .borrow_mut()
                    .release(mode.get(), position);
                crate::logging::debug_log(&format!("mouse release effects={effects:?}"));
                apply_interaction_effects(
                    effects,
                    &workspace,
                    &selection,
                    &selection_text,
                    &selection_dirty,
                    &content,
                );
            } else {
                let effects = pointer_interaction.borrow_mut().cancel();
                crate::logging::debug_log(&format!("mouse release outside effects={effects:?}"));
                apply_interaction_effects(
                    effects,
                    &workspace,
                    &selection,
                    &selection_text,
                    &selection_dirty,
                    &content,
                );
            }
        });
    }
    {
        let workspace = workspace_rc.clone();
        let selection = selection.clone();
        let selection_text = selection_text.clone();
        let selection_dirty = selection_dirty.clone();
        let content = last_content.clone();
        let pointer_interaction = pointer_interaction.clone();
        gtk::prelude::GestureExt::connect_cancel(&click_controller, move |_gesture, _sequence| {
            let effects = pointer_interaction.borrow_mut().cancel();
            crate::logging::debug_log(&format!("mouse gesture cancelled effects={effects:?}"));
            apply_interaction_effects(
                effects,
                &workspace,
                &selection,
                &selection_text,
                &selection_dirty,
                &content,
            );
        });
    }
    canvas.widget().add_controller(click_controller);

    let drag_controller = gtk::GestureDrag::new();
    drag_controller.set_button(1);
    {
        let workspace = workspace_rc.clone();
        let metrics = cell_metrics.clone();
        let mode = mouse_mode.clone();
        let selection = selection.clone();
        let selection_text = selection_text.clone();
        let selection_dirty = selection_dirty.clone();
        let content = last_content.clone();
        let pointer_interaction = pointer_interaction.clone();
        let drag_gesture_moved = drag_gesture_moved.clone();
        let canvas_widget = canvas.widget().clone();
        drag_controller.connect_drag_begin(move |_gesture, x, y| {
            if mode.get().sends_press_release() {
                return;
            }
            canvas_widget.grab_focus();
            drag_gesture_moved.set(false);
            if let Some(position) = pointer_grid_position(metrics.get(), x, y) {
                let effects = pointer_interaction.borrow_mut().press(
                    MouseMode::default(),
                    MouseButton::Left,
                    position,
                );
                crate::logging::debug_log(&format!("drag begin effects={effects:?}"));
                apply_interaction_effects(
                    effects,
                    &workspace,
                    &selection,
                    &selection_text,
                    &selection_dirty,
                    &content,
                );
            }
        });
    }
    {
        let workspace = workspace_rc.clone();
        let metrics = cell_metrics.clone();
        let mode = mouse_mode.clone();
        let selection = selection.clone();
        let selection_text = selection_text.clone();
        let selection_dirty = selection_dirty.clone();
        let content = last_content.clone();
        let pointer_interaction = pointer_interaction.clone();
        let drag_gesture_moved = drag_gesture_moved.clone();
        drag_controller.connect_drag_update(move |gesture, offset_x, offset_y| {
            if mode.get().sends_press_release() {
                return;
            }
            let Some((start_x, start_y)) = gesture.start_point() else {
                return;
            };
            if let Some(position) =
                pointer_grid_position(metrics.get(), start_x + offset_x, start_y + offset_y)
            {
                let effects = pointer_interaction
                    .borrow_mut()
                    .motion(MouseMode::default(), position);
                if effects
                    .iter()
                    .any(|effect| matches!(effect, InteractionEffect::SelectionChanged(Some(_))))
                {
                    drag_gesture_moved.set(true);
                }
                crate::logging::debug_log(&format!("drag update effects={effects:?}"));
                apply_interaction_effects(
                    effects,
                    &workspace,
                    &selection,
                    &selection_text,
                    &selection_dirty,
                    &content,
                );
            }
        });
    }
    {
        let workspace = workspace_rc.clone();
        let metrics = cell_metrics.clone();
        let mode = mouse_mode.clone();
        let selection = selection.clone();
        let selection_text = selection_text.clone();
        let selection_dirty = selection_dirty.clone();
        let content = last_content.clone();
        let pointer_interaction = pointer_interaction.clone();
        let drag_gesture_moved = drag_gesture_moved.clone();
        drag_controller.connect_drag_end(move |gesture, offset_x, offset_y| {
            if mode.get().sends_press_release() {
                return;
            }
            let Some((start_x, start_y)) = gesture.start_point() else {
                return;
            };
            let effects = if let Some(position) =
                pointer_grid_position(metrics.get(), start_x + offset_x, start_y + offset_y)
            {
                pointer_interaction
                    .borrow_mut()
                    .release(MouseMode::default(), position)
            } else {
                pointer_interaction.borrow_mut().cancel()
            };
            let effects = if drag_gesture_moved.get() {
                effects
                    .into_iter()
                    .filter(|effect| !matches!(effect, InteractionEffect::MoveCursorTo(_)))
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            crate::logging::debug_log(&format!("drag end effects={effects:?}"));
            apply_interaction_effects(
                effects,
                &workspace,
                &selection,
                &selection_text,
                &selection_dirty,
                &content,
            );
        });
    }
    {
        let workspace = workspace_rc.clone();
        let selection = selection.clone();
        let selection_text = selection_text.clone();
        let selection_dirty = selection_dirty.clone();
        let content = last_content.clone();
        let pointer_interaction = pointer_interaction.clone();
        let drag_gesture_moved = drag_gesture_moved.clone();
        gtk::prelude::GestureExt::connect_cancel(&drag_controller, move |_gesture, _sequence| {
            drag_gesture_moved.set(false);
            let effects = pointer_interaction.borrow_mut().cancel();
            crate::logging::debug_log(&format!("drag gesture cancelled effects={effects:?}"));
            apply_interaction_effects(
                effects,
                &workspace,
                &selection,
                &selection_text,
                &selection_dirty,
                &content,
            );
        });
    }
    canvas.widget().add_controller(drag_controller);

    let motion_controller = gtk::EventControllerMotion::new();
    {
        let workspace = workspace_rc.clone();
        let metrics = cell_metrics.clone();
        let mode = mouse_mode.clone();
        let selection = selection.clone();
        let selection_text = selection_text.clone();
        let selection_dirty = selection_dirty.clone();
        let content = last_content.clone();
        let pointer_interaction = pointer_interaction.clone();
        motion_controller.connect_motion(move |_controller, x, y| {
            crate::logging::debug_log(&format!("mouse motion x={x:.1} y={y:.1}"));
            if !mode.get().sends_drag() {
                return;
            }
            if let Some(position) = pointer_grid_position(metrics.get(), x, y) {
                crate::logging::debug_log(&format!(
                    "mouse motion grid col={} row={}",
                    position.column, position.row
                ));
                let effects = pointer_interaction
                    .borrow_mut()
                    .motion(mode.get(), position);
                crate::logging::debug_log(&format!("mouse motion effects={effects:?}"));
                apply_interaction_effects(
                    effects,
                    &workspace,
                    &selection,
                    &selection_text,
                    &selection_dirty,
                    &content,
                );
            }
        });
    }
    canvas.widget().add_controller(motion_controller);

    let scroll_controller =
        gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    {
        let workspace = workspace_rc.clone();
        let force_snapshot = force_snapshot.clone();
        let last_size = last_size.clone();
        let canvas_widget = canvas.widget().clone();
        scroll_controller.connect_scroll(move |controller, _dx, dy| {
            if controller
                .current_event_state()
                .contains(gtk::gdk::ModifierType::CONTROL_MASK)
            {
                if dy < 0.0 {
                    crate::terminal_font::zoom_in();
                } else if dy > 0.0 {
                    crate::terminal_font::zoom_out();
                }
                last_size.set(None);
                force_snapshot.set(true);
                canvas_widget.queue_draw();
                return glib::Propagation::Stop;
            }
            if let Some(lines) = crate::interaction::wheel_scroll_lines(dy) {
                let _ = workspace.borrow_mut().scroll_active(lines);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    canvas.widget().add_controller(scroll_controller);

    if let Some(scenario) = ui_e2e.clone() {
        let workspace = workspace_rc.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
            let _ = workspace
                .borrow_mut()
                .write_active(scenario.input.as_bytes());
        });
    }

    {
        let canvas = canvas.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(80), move || {
            canvas.tick_cursor_blink();
            glib::ControlFlow::Continue
        });
    }

    let app_for_tick = app.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
        let measured_metrics = terminal_metrics_for_widget(canvas.widget());
        cell_metrics.set(measured_metrics.map(|metrics| metrics.cell));
        if let (Some(path), Some(metrics)) = (&geometry_trace, measured_metrics) {
            trace_geometry(path, canvas.widget(), metrics);
        }
        if let Some(size) = measured_metrics.map(|metrics| metrics.size)
            && last_size.get() != Some(size)
            && workspace_rc.borrow_mut().resize_active(size).is_ok()
        {
            last_size.set(Some(size));
        }
        let terminal_content = if force_snapshot.replace(false) {
            workspace_rc.borrow_mut().snapshot_active_renderable()
        } else {
            workspace_rc
                .borrow_mut()
                .snapshot_active_renderable_if_dirty()
        };
        let terminal_changed = terminal_content.is_some();
        if let Some(content) = terminal_content {
            mouse_mode.set(content.mouse);
            *last_content.borrow_mut() = Some(content);
        }
        let selection_changed = selection_dirty.replace(false);
        let render_content = if terminal_changed || selection_changed {
            last_content.borrow().clone()
        } else {
            None
        };
        if let Some(content) = render_content {
            let visible_selection = visible_selection_matching_text(
                &content,
                selection.get(),
                selection_text.borrow().as_deref(),
            );
            if selection_text.borrow().is_none()
                && let Some(visible_selection) = visible_selection
            {
                *selection_text.borrow_mut() =
                    text_for_viewport_selection(&content, visible_selection);
            }
            let allocations_before = crate::allocation_trace::snapshot();
            let render_started = std::time::Instant::now();
            let rendered =
                Renderer::render_frame_with_selection(content.clone(), visible_selection);
            crate::perf_trace::record_duration("gtk_render", render_started.elapsed());
            let allocations_after = crate::allocation_trace::snapshot();
            crate::perf_trace::record_counter(
                "gtk_render_allocs",
                allocations_after
                    .allocations
                    .saturating_sub(allocations_before.allocations),
            );
            crate::perf_trace::record_counter(
                "gtk_render_alloc_bytes",
                allocations_after
                    .bytes
                    .saturating_sub(allocations_before.bytes),
            );
            canvas.set_render(rendered);
            if selection_changed {
                copy_selection_to_primary(canvas.widget(), &selection_text);
            }
            if let Some(scenario) = &ui_e2e {
                let text = lines_to_text(&content.lines);
                if scenario
                    .expected
                    .iter()
                    .all(|expected| text.contains(expected))
                {
                    gtk::test_widget_wait_for_draw(canvas.widget());
                    let _ = write_snapshot_with_selection(content, "gtk_e2e", visible_selection);
                    app_for_tick.quit();
                    return glib::ControlFlow::Break;
                }
                if ui_e2e_deadline.is_some_and(|deadline| std::time::Instant::now() > deadline) {
                    eprintln!("gtk e2e expected content did not appear: {text}");
                    std::process::exit(1);
                }
            }
        }
        if snapshot_enabled
            && (last_snapshot.borrow().elapsed() >= std::time::Duration::from_secs(1)
                || selection_changed)
            && let Some(content) = last_content.borrow().clone()
        {
            *last_snapshot.borrow_mut() = std::time::Instant::now();
            crate::logging::debug_log(&format!("snapshot selection {:?}", selection.get()));
            let visible_selection = visible_selection_matching_text(
                &content,
                selection.get(),
                selection_text.borrow().as_deref(),
            );
            let _ = write_snapshot_with_selection(content, "frame", visible_selection);
        }
        glib::ControlFlow::Continue
    });

    window.present();
}

fn configure_launch_menu(
    menu_button: &gtk::MenuButton,
    tabs: TabContext,
    last_size: std::rc::Rc<std::cell::Cell<Option<ScreenSize>>>,
) {
    let popover = gtk::Popover::new();
    let panel = gtk::Box::new(gtk::Orientation::Vertical, 8);
    panel.add_css_class("terminal-launch-popover");
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Filter...")
        .build();
    panel.append(&search);
    let list = gtk::ListBox::new();
    list.add_css_class("terminal-launch-list");
    panel.append(&list);
    popover.set_child(Some(&panel));
    menu_button.set_popover(Some(&popover));

    let targets = std::rc::Rc::new(available_launch_targets());
    rebuild_launch_list(
        &list,
        "",
        targets.as_ref(),
        LaunchMenuContext {
            tabs: tabs.clone(),
            last_size: last_size.clone(),
            popover: popover.clone(),
        },
    );
    {
        let list = list.clone();
        let targets = targets.clone();
        let tabs = tabs.clone();
        let last_size = last_size.clone();
        let popover = popover.clone();
        search.connect_search_changed(move |entry| {
            rebuild_launch_list(
                &list,
                entry.text().as_str(),
                targets.as_ref(),
                LaunchMenuContext {
                    tabs: tabs.clone(),
                    last_size: last_size.clone(),
                    popover: popover.clone(),
                },
            );
        });
    }
}

#[derive(Clone)]
struct TabContext {
    workspace: std::rc::Rc<std::cell::RefCell<TerminalWorkspace>>,
    tab_view: adw::TabView,
    tab_pages: std::rc::Rc<std::cell::RefCell<TabPages>>,
    force_snapshot: std::rc::Rc<std::cell::Cell<bool>>,
    selection: std::rc::Rc<std::cell::Cell<Option<SelectionRange>>>,
    selection_text: std::rc::Rc<std::cell::RefCell<Option<String>>>,
    selection_dirty: std::rc::Rc<std::cell::Cell<bool>>,
}

#[derive(Clone)]
struct LaunchMenuContext {
    tabs: TabContext,
    last_size: std::rc::Rc<std::cell::Cell<Option<ScreenSize>>>,
    popover: gtk::Popover,
}

#[derive(Default)]
struct TabPages {
    entries: Vec<TabPageEntry>,
}

struct TabPageEntry {
    id: PaneId,
    page: adw::TabPage,
}

impl TabPages {
    fn push(&mut self, id: PaneId, page: adw::TabPage) {
        self.entries.push(TabPageEntry { id, page });
    }

    fn id_for_page(&self, page: &adw::TabPage) -> Option<PaneId> {
        self.entries
            .iter()
            .find(|entry| entry.page == *page)
            .map(|entry| entry.id)
    }

    fn page_for_id(&self, id: PaneId) -> Option<adw::TabPage> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.page.clone())
    }

    fn remove_page(&mut self, page: &adw::TabPage) {
        self.entries.retain(|entry| entry.page != *page);
    }

    fn sync_title(&self, workspace: &TerminalWorkspace, id: PaneId) {
        let Some(page) = self.page_for_id(id) else {
            return;
        };
        if let Some(title) = workspace.pane_title(id) {
            page.set_title(&title);
            page.set_tooltip(&title);
        }
    }
}

fn rebuild_launch_list(
    list: &gtk::ListBox,
    filter: &str,
    targets: &[LaunchTarget],
    context: LaunchMenuContext,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let filter = filter.trim().to_lowercase();
    let mut appended_containers_header = false;
    for target in targets.iter().filter(|target| {
        filter.is_empty() || target.title().to_lowercase().contains(filter.as_str())
    }) {
        if !matches!(target, LaunchTarget::Host) && !appended_containers_header {
            let header = gtk::Label::builder()
                .label("Containers")
                .halign(gtk::Align::Center)
                .build();
            header.add_css_class("terminal-launch-section");
            let row = gtk::ListBoxRow::new();
            row.set_selectable(false);
            row.set_activatable(false);
            row.add_css_class("terminal-launch-section-row");
            row.set_child(Some(&header));
            list.append(&row);
            appended_containers_header = true;
        }
        let row = gtk::ListBoxRow::new();
        row.add_css_class("terminal-launch-row");
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        content.add_css_class("terminal-launch-row-content");
        let label = gtk::Label::builder()
            .label(target.title())
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        content.append(&label);
        row.set_child(Some(&content));
        let target = target.clone();
        let context = context.clone();
        row.connect_activate(move |_| {
            add_launch_target_pane(target.clone(), &context);
            context.popover.popdown();
        });
        list.append(&row);
    }
}

fn add_launch_target_pane(target: LaunchTarget, context: &LaunchMenuContext) {
    let title = target.title();
    let Ok(id) = context
        .tabs
        .workspace
        .borrow_mut()
        .add_titled_pane_with(&title, target.command())
    else {
        return;
    };
    let page = append_tab_page(&context.tabs.tab_view, id, &title);
    context.tabs.tab_pages.borrow_mut().push(id, page.clone());
    context.tabs.tab_view.set_selected_page(&page);
    activate_workspace_pane(
        &context.tabs.workspace,
        id,
        &context.tabs.force_snapshot,
        &context.tabs.selection,
        &context.tabs.selection_text,
        &context.tabs.selection_dirty,
        context.last_size.get(),
    );
}

fn add_existing_workspace_pages(
    tab_view: &adw::TabView,
    tab_pages: &std::rc::Rc<std::cell::RefCell<TabPages>>,
    workspace: &TerminalWorkspace,
) {
    for pane in workspace.panes() {
        let title = workspace
            .pane_title(pane.id)
            .unwrap_or_else(|| format!("Terminal {}", pane.index + 1));
        let page = append_tab_page(tab_view, pane.id, &title);
        tab_pages.borrow_mut().push(pane.id, page.clone());
        if pane.active {
            tab_view.set_selected_page(&page);
        }
    }
}

fn append_tab_page(tab_view: &adw::TabView, id: PaneId, title: &str) -> adw::TabPage {
    let child = gtk::Box::new(gtk::Orientation::Vertical, 0);
    child.set_widget_name(&format!("chelotype-pane-{:?}", id));
    let page = tab_view.append(&child);
    page.set_title(title);
    page.set_tooltip(title);
    page
}

fn select_tab_for_pane(
    tab_view: &adw::TabView,
    tab_pages: &std::rc::Rc<std::cell::RefCell<TabPages>>,
    id: PaneId,
) {
    let Some(page) = tab_pages.borrow().page_for_id(id) else {
        return;
    };
    tab_view.set_selected_page(&page);
}

fn tab_menu_model() -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append(Some("Rename Tab"), Some("win.rename-tab"));
    menu.append(Some("Move Tab Left"), Some("win.move-tab-left"));
    menu.append(Some("Move Tab Right"), Some("win.move-tab-right"));
    menu.append(Some("Close Other Tabs"), Some("win.close-other-tabs"));
    menu.append(Some("Close Tab"), Some("win.close-tab"));
    menu
}

fn install_tab_actions(
    window: &adw::ApplicationWindow,
    tab_bar: &adw::TabBar,
    tabs: TabContext,
    last_size: std::rc::Rc<std::cell::Cell<Option<ScreenSize>>>,
) {
    let menu_target = std::rc::Rc::new(std::cell::Cell::new(None::<PaneId>));
    {
        let menu_target = menu_target.clone();
        let tab_pages = tabs.tab_pages.clone();
        tabs.tab_view.connect_setup_menu(move |_view, page| {
            menu_target.set(page.and_then(|page| tab_pages.borrow().id_for_page(page)));
        });
    }
    let rename = gio::SimpleAction::new("rename-tab", None);
    {
        let tabs = tabs.clone();
        let menu_target = menu_target.clone();
        let parent = tab_bar.clone().upcast::<gtk::Widget>();
        rename.connect_activate(move |_action, _parameter| {
            if let Some(id) = menu_target.get().or_else(|| active_tab_id(&tabs)) {
                open_rename_popover(id, &parent, &tabs);
            }
        });
    }
    window.add_action(&rename);

    let move_left = gio::SimpleAction::new("move-tab-left", None);
    {
        let tabs = tabs.clone();
        let menu_target = menu_target.clone();
        move_left.connect_activate(move |_action, _parameter| {
            if let Some(id) = menu_target.get().or_else(|| active_tab_id(&tabs)) {
                move_tab(&tabs, id, TabMoveDirection::Left);
            }
        });
    }
    window.add_action(&move_left);

    let move_right = gio::SimpleAction::new("move-tab-right", None);
    {
        let tabs = tabs.clone();
        let menu_target = menu_target.clone();
        move_right.connect_activate(move |_action, _parameter| {
            if let Some(id) = menu_target.get().or_else(|| active_tab_id(&tabs)) {
                move_tab(&tabs, id, TabMoveDirection::Right);
            }
        });
    }
    window.add_action(&move_right);

    let close_others = gio::SimpleAction::new("close-other-tabs", None);
    {
        let tabs = tabs.clone();
        let menu_target = menu_target.clone();
        let last_size = last_size.clone();
        close_others.connect_activate(move |_action, _parameter| {
            if let Some(id) = menu_target.get().or_else(|| active_tab_id(&tabs)) {
                close_other_tabs(&tabs, id, last_size.get());
            }
        });
    }
    window.add_action(&close_others);

    let close = gio::SimpleAction::new("close-tab", None);
    {
        let tabs = tabs.clone();
        let menu_target = menu_target.clone();
        close.connect_activate(move |_action, _parameter| {
            if let Some(id) = menu_target.get().or_else(|| active_tab_id(&tabs)) {
                close_tab(&tabs, id);
            }
        });
    }
    window.add_action(&close);

    let double_click = gtk::GestureClick::new();
    double_click.set_button(1);
    {
        let tabs = tabs.clone();
        let parent = tab_bar.clone().upcast::<gtk::Widget>();
        double_click.connect_released(move |_gesture, count, _x, _y| {
            if count == 2
                && let Some(id) = active_tab_id(&tabs)
            {
                open_rename_popover(id, &parent, &tabs);
            }
        });
    }
    tab_bar.add_controller(double_click);
}

fn connect_native_tabs(
    tab_view: &adw::TabView,
    tabs: TabContext,
    last_size: std::rc::Rc<std::cell::Cell<Option<ScreenSize>>>,
) {
    {
        let tabs = tabs.clone();
        let last_size = last_size.clone();
        tab_view.connect_selected_page_notify(move |view| {
            let Some(page) = view.selected_page() else {
                return;
            };
            let Some(id) = tabs.tab_pages.borrow().id_for_page(&page) else {
                return;
            };
            activate_workspace_pane(
                &tabs.workspace,
                id,
                &tabs.force_snapshot,
                &tabs.selection,
                &tabs.selection_text,
                &tabs.selection_dirty,
                last_size.get(),
            );
        });
    }
    {
        let tabs = tabs.clone();
        tab_view.connect_page_reordered(move |_view, page, position| {
            let Some(id) = tabs.tab_pages.borrow().id_for_page(page) else {
                return;
            };
            tabs.workspace
                .borrow_mut()
                .reorder(id, position.max(0) as usize);
        });
    }
    {
        let tabs = tabs.clone();
        tab_view.connect_page_detached(move |_view, page, _position| {
            tabs.tab_pages.borrow_mut().remove_page(page);
            let active = tabs.workspace.borrow().active_pane_id();
            select_tab_for_pane(&tabs.tab_view, &tabs.tab_pages, active);
        });
    }
    {
        let tabs = tabs.clone();
        tab_view.connect_close_page(move |_view, page| {
            let Some(id) = tabs.tab_pages.borrow().id_for_page(page) else {
                return glib::Propagation::Stop;
            };
            if tabs.workspace.borrow_mut().close(id) {
                force_active_workspace_snapshot(
                    &tabs.force_snapshot,
                    &tabs.selection,
                    &tabs.selection_text,
                    &tabs.selection_dirty,
                    None,
                    &tabs.workspace,
                );
                glib::Propagation::Proceed
            } else {
                glib::Propagation::Stop
            }
        });
    }
}

fn active_tab_id(tabs: &TabContext) -> Option<PaneId> {
    let page = tabs.tab_view.selected_page()?;
    tabs.tab_pages.borrow().id_for_page(&page)
}

#[derive(Clone, Copy)]
enum TabMoveDirection {
    Left,
    Right,
}

fn move_tab(tabs: &TabContext, id: PaneId, direction: TabMoveDirection) {
    let Some(page) = tabs.tab_pages.borrow().page_for_id(id) else {
        return;
    };
    let moved = match direction {
        TabMoveDirection::Left => {
            tabs.workspace.borrow_mut().move_left(id) && tabs.tab_view.reorder_backward(&page)
        }
        TabMoveDirection::Right => {
            tabs.workspace.borrow_mut().move_right(id) && tabs.tab_view.reorder_forward(&page)
        }
    };
    if moved {
        tabs.tab_view.set_selected_page(&page);
    }
}

fn close_tab(tabs: &TabContext, id: PaneId) {
    let Some(page) = tabs.tab_pages.borrow().page_for_id(id) else {
        return;
    };
    tabs.tab_view.close_page(&page);
}

fn close_other_tabs(tabs: &TabContext, id: PaneId, size: Option<ScreenSize>) {
    let Some(page) = tabs.tab_pages.borrow().page_for_id(id) else {
        return;
    };
    let other_pages = tabs
        .tab_pages
        .borrow()
        .entries
        .iter()
        .filter(|entry| entry.id != id)
        .map(|entry| entry.page.clone())
        .collect::<Vec<_>>();
    for other_page in other_pages {
        tabs.tab_view.close_page(&other_page);
    }
    tabs.tab_view.set_selected_page(&page);
    activate_workspace_pane(
        &tabs.workspace,
        id,
        &tabs.force_snapshot,
        &tabs.selection,
        &tabs.selection_text,
        &tabs.selection_dirty,
        size,
    );
}

fn open_rename_popover(id: PaneId, parent: &gtk::Widget, tabs: &TabContext) {
    let popover = gtk::Popover::new();
    popover.set_parent(parent);
    let entry = gtk::Entry::builder()
        .text(
            tabs.workspace
                .borrow()
                .pane_title(id)
                .unwrap_or_else(|| "Terminal".to_string()),
        )
        .activates_default(true)
        .build();
    entry.add_css_class("terminal-rename-entry");
    popover.set_child(Some(&entry));
    {
        let tabs = tabs.clone();
        let popover = popover.clone();
        entry.connect_activate(move |entry| {
            if tabs
                .workspace
                .borrow_mut()
                .rename(id, entry.text().to_string())
            {
                tabs.tab_pages
                    .borrow()
                    .sync_title(&tabs.workspace.borrow(), id);
            }
            popover.popdown();
        });
    }
    popover.popup();
    entry.grab_focus();
    entry.select_region(0, -1);
}

fn activate_workspace_pane(
    workspace: &std::rc::Rc<std::cell::RefCell<TerminalWorkspace>>,
    id: PaneId,
    force_snapshot: &std::rc::Rc<std::cell::Cell<bool>>,
    selection: &std::rc::Rc<std::cell::Cell<Option<SelectionRange>>>,
    selection_text: &std::rc::Rc<std::cell::RefCell<Option<String>>>,
    selection_dirty: &std::rc::Rc<std::cell::Cell<bool>>,
    size: Option<ScreenSize>,
) {
    let activated = workspace.borrow_mut().activate(id);
    if activated {
        force_active_workspace_snapshot(
            force_snapshot,
            selection,
            selection_text,
            selection_dirty,
            size,
            workspace,
        );
    }
}

fn force_active_workspace_snapshot(
    force_snapshot: &std::rc::Rc<std::cell::Cell<bool>>,
    selection: &std::rc::Rc<std::cell::Cell<Option<SelectionRange>>>,
    selection_text: &std::rc::Rc<std::cell::RefCell<Option<String>>>,
    selection_dirty: &std::rc::Rc<std::cell::Cell<bool>>,
    size: Option<ScreenSize>,
    workspace: &std::rc::Rc<std::cell::RefCell<TerminalWorkspace>>,
) {
    if let Some(size) = size {
        let _ = workspace.borrow_mut().resize_active(size);
    }
    clear_selection(selection, selection_text, selection_dirty);
    force_snapshot.set(true);
}

#[derive(Clone)]
struct UiE2eScenario {
    input: String,
    expected: Vec<String>,
    timeout: std::time::Duration,
}

impl UiE2eScenario {
    fn from_env() -> Option<Self> {
        if std::env::var("CHELOTYPE_UI_E2E").ok().as_deref() != Some("1") {
            return None;
        }
        let input = std::env::var("CHELOTYPE_UI_E2E_INPUT")
            .unwrap_or_else(|_| "printf 'CHELOTYPE_GTK_E2E_OK\\n'\n".to_string());
        let expected = std::env::var("CHELOTYPE_UI_E2E_EXPECT")
            .unwrap_or_else(|_| "CHELOTYPE_GTK_E2E_OK".to_string())
            .split('|')
            .map(ToOwned::to_owned)
            .collect();
        let timeout = std::env::var("CHELOTYPE_UI_E2E_TIMEOUT_MS")
            .ok()
            .and_then(|value| value.parse().ok())
            .map(std::time::Duration::from_millis)
            .unwrap_or_else(|| std::time::Duration::from_secs(4));
        Some(Self {
            input,
            expected,
            timeout,
        })
    }
}

fn copy_selection_to_primary(
    widget: &gtk::DrawingArea,
    selection_text: &std::rc::Rc<std::cell::RefCell<Option<String>>>,
) -> bool {
    let Some(text) = selection_text.borrow().clone() else {
        return false;
    };
    widget.primary_clipboard().set_text(&text);
    trace_clipboard_export("primary", &text);
    true
}

fn copy_selection_to_clipboard(
    widget: &gtk::DrawingArea,
    selection_text: &std::rc::Rc<std::cell::RefCell<Option<String>>>,
) -> bool {
    let Some(text) = selection_text.borrow().clone() else {
        return false;
    };
    widget.clipboard().set_text(&text);
    trace_clipboard_export("clipboard", &text);
    true
}

fn paste_clipboard_text(
    widget: &gtk::DrawingArea,
    workspace: std::rc::Rc<std::cell::RefCell<TerminalWorkspace>>,
    content: std::rc::Rc<std::cell::RefCell<Option<RenderableContentOwned>>>,
    selection: std::rc::Rc<std::cell::Cell<Option<SelectionRange>>>,
    selection_text: std::rc::Rc<std::cell::RefCell<Option<String>>>,
    selection_dirty: std::rc::Rc<std::cell::Cell<bool>>,
) {
    widget
        .clipboard()
        .read_text_async(None::<&gtk::gio::Cancellable>, move |result| {
            let Ok(Some(text)) = result else {
                return;
            };
            write_key_with_selection(
                &workspace,
                &content,
                &selection,
                &selection_text,
                &selection_dirty,
                text.as_bytes().to_vec(),
            );
        });
}

fn text_for_viewport_selection(
    content: &RenderableContentOwned,
    selection: SelectionRange,
) -> Option<String> {
    let text = selected_text(&content.lines, selection);
    if text.is_empty() { None } else { Some(text) }
}

fn visible_selection_for_content(
    content: &RenderableContentOwned,
    selection: Option<SelectionRange>,
) -> Option<SelectionRange> {
    viewport_range_for_display(selection?, content.display_offset, content.lines.len())
}

fn visible_selection_matching_text(
    content: &RenderableContentOwned,
    selection: Option<SelectionRange>,
    expected_text: Option<&str>,
) -> Option<SelectionRange> {
    let visible = visible_selection_for_content(content, selection)?;
    let Some(expected_text) = expected_text else {
        return Some(visible);
    };
    if text_for_viewport_selection(content, visible).as_deref() == Some(expected_text) {
        Some(visible)
    } else {
        None
    }
}

fn select_active_input(
    content: &std::rc::Rc<std::cell::RefCell<Option<RenderableContentOwned>>>,
    selection: &std::rc::Rc<std::cell::Cell<Option<SelectionRange>>>,
    selection_text: &std::rc::Rc<std::cell::RefCell<Option<String>>>,
    selection_dirty: &std::rc::Rc<std::cell::Cell<bool>>,
) {
    let Some(content) = content.borrow().clone() else {
        return;
    };
    let Some(row) = usize::try_from(content.cursor_line).ok() else {
        return;
    };
    let Some(line) = content.lines.get(row) else {
        return;
    };
    let start = input_start_column(line);
    let end = line_significant_len(line);
    if end <= start {
        return;
    }
    set_viewport_selection(
        &content,
        SelectionRange::new(
            GridPoint { row, column: start },
            GridPoint { row, column: end },
        ),
        selection,
        selection_text,
        selection_dirty,
    );
}

fn select_mouse_click_range(
    content: &std::rc::Rc<std::cell::RefCell<Option<RenderableContentOwned>>>,
    selection: &std::rc::Rc<std::cell::Cell<Option<SelectionRange>>>,
    selection_text: &std::rc::Rc<std::cell::RefCell<Option<String>>>,
    selection_dirty: &std::rc::Rc<std::cell::Cell<bool>>,
    position: MouseGridPosition,
    press_count: i32,
) -> bool {
    if press_count < 2 {
        return false;
    }
    let Some(content) = content.borrow().clone() else {
        return false;
    };
    let row = usize::from(position.row);
    let column = usize::from(position.column);
    let range = if press_count >= 3 {
        line_range(&content.lines, row)
    } else {
        word_range_at(&content.lines, row, column)
    };
    let Some(range) = range else {
        return false;
    };
    set_viewport_selection(&content, range, selection, selection_text, selection_dirty);
    true
}

fn set_viewport_selection(
    content: &RenderableContentOwned,
    range: SelectionRange,
    selection: &std::rc::Rc<std::cell::Cell<Option<SelectionRange>>>,
    selection_text: &std::rc::Rc<std::cell::RefCell<Option<String>>>,
    selection_dirty: &std::rc::Rc<std::cell::Cell<bool>>,
) {
    selection.set(Some(anchor_range_to_display(range, content.display_offset)));
    *selection_text.borrow_mut() = text_for_viewport_selection(content, range);
    selection_dirty.set(true);
}

fn clear_selection(
    selection: &std::rc::Rc<std::cell::Cell<Option<SelectionRange>>>,
    selection_text: &std::rc::Rc<std::cell::RefCell<Option<String>>>,
    selection_dirty: &std::rc::Rc<std::cell::Cell<bool>>,
) {
    crate::logging::debug_log("clear selection");
    selection.set(None);
    *selection_text.borrow_mut() = None;
    selection_dirty.set(true);
}

fn write_key_with_selection(
    workspace: &std::rc::Rc<std::cell::RefCell<TerminalWorkspace>>,
    content: &std::rc::Rc<std::cell::RefCell<Option<RenderableContentOwned>>>,
    selection: &std::rc::Rc<std::cell::Cell<Option<SelectionRange>>>,
    selection_text: &std::rc::Rc<std::cell::RefCell<Option<String>>>,
    selection_dirty: &std::rc::Rc<std::cell::Cell<bool>>,
    data: Vec<u8>,
) {
    let Some(selection_range) = selection.get() else {
        let _ = workspace.borrow_mut().write_active(&data);
        return;
    };
    let Some(content) = content.borrow().clone() else {
        clear_selection(selection, selection_text, selection_dirty);
        let _ = workspace.borrow_mut().write_active(&data);
        return;
    };
    let Some(viewport_selection) = visible_selection_matching_text(
        &content,
        Some(selection_range),
        selection_text.borrow().as_deref(),
    ) else {
        clear_selection(selection, selection_text, selection_dirty);
        let _ = workspace.borrow_mut().write_active(&data);
        return;
    };
    let Some(selected) = text_for_viewport_selection(&content, viewport_selection) else {
        clear_selection(selection, selection_text, selection_dirty);
        let _ = workspace.borrow_mut().write_active(&data);
        return;
    };
    let target = MouseGridPosition {
        column: viewport_selection.start.column.min(u16::MAX as usize) as u16,
        row: viewport_selection.start.row.min(u16::MAX as usize) as u16,
    };
    let movement = if content.cursor_line == i32::from(target.row)
        && content.cursor_col == i32::from(target.column)
    {
        Vec::new()
    } else if let Some(bytes) = cursor_movement_bytes_for_content(&content, target) {
        bytes
    } else {
        clear_selection(selection, selection_text, selection_dirty);
        let _ = workspace.borrow_mut().write_active(&data);
        return;
    };

    let mut replacement = movement;
    for _ in selected.chars() {
        replacement.extend_from_slice(b"\x1b[3~");
    }
    if data.as_slice() != [0x7f] && data.as_slice() != b"\x1b[3~" {
        replacement.extend_from_slice(&data);
    }
    clear_selection(selection, selection_text, selection_dirty);
    let _ = workspace.borrow_mut().write_active(&replacement);
}

#[derive(Clone, Copy)]
struct KeyboardMove {
    direction: CursorDirection,
    unit: CursorUnit,
    selecting: bool,
}

fn move_cursor_from_keyboard(
    workspace: &std::rc::Rc<std::cell::RefCell<TerminalWorkspace>>,
    content: &std::rc::Rc<std::cell::RefCell<Option<RenderableContentOwned>>>,
    selection: &std::rc::Rc<std::cell::Cell<Option<SelectionRange>>>,
    selection_text: &std::rc::Rc<std::cell::RefCell<Option<String>>>,
    selection_dirty: &std::rc::Rc<std::cell::Cell<bool>>,
    cursor_move: KeyboardMove,
) {
    if let Some(fresh_content) = workspace.borrow_mut().snapshot_active_renderable() {
        *content.borrow_mut() = Some(fresh_content);
    }
    let Some(content) = content.borrow().clone() else {
        return;
    };
    let current_override = if cursor_move.selecting {
        keyboard_selection_cursor(
            selection.get(),
            content.display_offset,
            content.lines.len(),
            cursor_move.direction,
        )
    } else {
        None
    };
    let Some(target) = keyboard_cursor_target(
        &content,
        cursor_move.direction,
        cursor_move.unit,
        current_override,
    ) else {
        return;
    };
    if cursor_move.selecting {
        select_keyboard_cursor_range(
            &content,
            target,
            selection,
            selection_text,
            selection_dirty,
            cursor_move.direction,
        );
    } else if selection.get().is_some() {
        clear_selection(selection, selection_text, selection_dirty);
    }
    if let Some(bytes) = keyboard_cursor_bytes(&content, target, cursor_move) {
        let _ = workspace.borrow_mut().write_active(&bytes);
    }
}

fn keyboard_cursor_target(
    content: &RenderableContentOwned,
    direction: CursorDirection,
    unit: CursorUnit,
    current: Option<GridPoint>,
) -> Option<MouseGridPosition> {
    let row = current
        .map(|point| point.row)
        .or_else(|| usize::try_from(content.cursor_line).ok())?;
    let cursor = current
        .map(|point| point.column)
        .or_else(|| usize::try_from(content.cursor_col).ok())?;
    let line = content.lines.get(row)?;
    let start = input_start_column(line);
    let end = line_significant_len(line);
    let column = match (direction, unit) {
        (CursorDirection::Left, CursorUnit::Cell) => cursor.saturating_sub(1).max(start),
        (CursorDirection::Right, CursorUnit::Cell) => (cursor + 1).min(end),
        (CursorDirection::Left, CursorUnit::Word) => previous_word_boundary(line, cursor, start),
        (CursorDirection::Right, CursorUnit::Word) => next_word_boundary(line, cursor, end),
    };
    Some(MouseGridPosition {
        row: row.min(u16::MAX as usize) as u16,
        column: column.min(u16::MAX as usize) as u16,
    })
}

fn keyboard_selection_cursor(
    selection: Option<SelectionRange>,
    display_offset: usize,
    viewport_rows: usize,
    direction: CursorDirection,
) -> Option<GridPoint> {
    let range = viewport_range_for_display(selection?, display_offset, viewport_rows)?;
    Some(match direction {
        CursorDirection::Left => range.start,
        CursorDirection::Right => range.end,
    })
}

fn keyboard_cursor_bytes(
    content: &RenderableContentOwned,
    target: MouseGridPosition,
    cursor_move: KeyboardMove,
) -> Option<Vec<u8>> {
    match (cursor_move.direction, cursor_move.unit) {
        (CursorDirection::Left, CursorUnit::Cell) => Some(b"\x1b[D".to_vec()),
        (CursorDirection::Right, CursorUnit::Cell) => Some(b"\x1b[C".to_vec()),
        _ => cursor_movement_bytes_for_content(content, target),
    }
}

fn select_keyboard_cursor_range(
    content: &RenderableContentOwned,
    target: MouseGridPosition,
    selection: &std::rc::Rc<std::cell::Cell<Option<SelectionRange>>>,
    selection_text: &std::rc::Rc<std::cell::RefCell<Option<String>>>,
    selection_dirty: &std::rc::Rc<std::cell::Cell<bool>>,
    direction: CursorDirection,
) {
    let Some(cursor_row) = usize::try_from(content.cursor_line).ok() else {
        return;
    };
    let Some(cursor_column) = usize::try_from(content.cursor_col).ok() else {
        return;
    };
    let cursor = GridPoint {
        row: cursor_row + content.display_offset,
        column: cursor_column,
    };
    let target = GridPoint {
        row: usize::from(target.row) + content.display_offset,
        column: usize::from(target.column),
    };
    let anchor = selection
        .get()
        .map(|range| match direction {
            CursorDirection::Left => range.end,
            CursorDirection::Right => range.start,
        })
        .unwrap_or(cursor);
    let absolute = SelectionRange::new(anchor, target);
    selection.set(Some(absolute));
    let visible = visible_selection_for_content(content, Some(absolute));
    *selection_text.borrow_mut() =
        visible.and_then(|range| text_for_viewport_selection(content, range));
    selection_dirty.set(true);
}

fn input_start_column(line: &[crate::terminal_grid::TerminalCell]) -> usize {
    let significant = line_significant_len(line);
    line.iter()
        .take(significant)
        .position(|cell| cell.text == " ")
        .map(|column| column + 1)
        .unwrap_or(0)
}

fn previous_word_boundary(
    line: &[crate::terminal_grid::TerminalCell],
    cursor: usize,
    start: usize,
) -> usize {
    let mut column = cursor.min(line_significant_len(line));
    while column > start && !is_word_cell(&line[column - 1]) {
        column -= 1;
    }
    while column > start && is_word_cell(&line[column - 1]) {
        column -= 1;
    }
    column
}

fn next_word_boundary(
    line: &[crate::terminal_grid::TerminalCell],
    cursor: usize,
    end: usize,
) -> usize {
    let mut column = cursor.min(end);
    while column < end && !is_word_cell(&line[column]) {
        column += 1;
    }
    while column < end && is_word_cell(&line[column]) {
        column += 1;
    }
    column
}

fn is_word_cell(cell: &crate::terminal_grid::TerminalCell) -> bool {
    cell.text
        .chars()
        .any(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

fn trace_clipboard_export(kind: &str, text: &str) {
    let Ok(path) = std::env::var("CHELOTYPE_CLIPBOARD_TRACE") else {
        return;
    };
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let escaped = text.replace('\n', "\\n");
        let _ = writeln!(file, "{kind}\t{escaped}");
    }
}

fn apply_style(canvas: &gtk::DrawingArea) {
    let css = "
        .term-root {
            background: #0f1115;
        }
        .terminal-header {
            background: #303033;
            border-bottom: none;
            box-shadow: none;
            min-height: 2.25rem;
            padding-top: 0;
            padding-bottom: 0;
            padding-left: 0.5rem;
            padding-right: 0.5rem;
        }
        drawingarea.term-canvas {
            color: #e5e7eb;
            background: transparent;
        }
        .terminal-header-title {
            background: transparent;
            border-spacing: 0.25rem;
        }
        .terminal-launcher {
            background: #3d3d40;
            border-radius: 0.5rem;
            padding: 0;
        }
        .terminal-launcher button {
            min-width: 1.5rem;
            min-height: 1.25rem;
            padding: 0.375rem;
            margin-top: 0;
            margin-bottom: 0;
            border-radius: 0.375rem;
            -gtk-icon-size: 0.875rem;
        }
        .terminal-launch-menu-button button {
            min-width: 1rem;
            min-height: 1.25rem;
            padding-left: 0.375rem;
            padding-right: 0.375rem;
            padding-top: 0.375rem;
            padding-bottom: 0.375rem;
        }
        .term-tab-bar {
            background: transparent;
            box-shadow: none;
            border: none;
        }
        .term-tab-bar scrolledwindow,
        .term-tab-bar undershoot,
        tabbar,
        tabbox,
        .term-tab-bar > revealer > box,
        .term-tab-bar tabbox {
            background: transparent;
            box-shadow: none;
            border: none;
        }
        .terminal-launch-popover {
            min-width: 20rem;
            border-radius: 0.625rem;
            padding: 0.5rem;
            background: #2f2f33;
        }
        .terminal-launch-popover contents,
        .terminal-launch-popover box,
        .terminal-launch-popover scrolledwindow,
        .terminal-launch-popover viewport {
            background: #2f2f33;
        }
        .terminal-launch-popover list,
        .terminal-launch-popover row,
        .terminal-launch-popover searchentry,
        .terminal-launch-popover entry {
            background: #2f2f33;
        }
        .terminal-launch-list {
            padding: 0;
            border-radius: 0.5rem;
            background: #2f2f33;
        }
        .terminal-launch-popover .terminal-launch-section-row,
        .terminal-launch-popover .terminal-launch-section-row:hover,
        .terminal-launch-popover .terminal-launch-section-row:selected,
        .terminal-launch-section-row,
        .terminal-launch-section-row:hover,
        .terminal-launch-section-row:selected {
            background: #2f2f33;
        }
        .terminal-launch-section {
            color: #a7a7ad;
            font-weight: 700;
            padding: 0.5rem 0.625rem 0.375rem;
        }
        .terminal-launch-row {
            min-height: 2.125rem;
            padding: 0;
            margin: 0.0625rem 0;
            border-radius: 0.5rem;
            background: #2f2f33;
        }
        .terminal-launch-row:hover,
        .terminal-launch-row:selected,
        .terminal-launch-row:hover .terminal-launch-row-content,
        .terminal-launch-row:selected .terminal-launch-row-content {
            background: #3a3a3e;
        }
        .terminal-launch-row-content {
            padding: 0.375rem 1rem;
            border-radius: 0.5rem;
            background: #2f2f33;
        }
        .terminal-launch-row label {
            font-weight: 400;
        }
        .terminal-rename-entry {
            min-width: 13rem;
            margin: 0.5rem;
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

fn trace_geometry(path: &std::path::Path, widget: &gtk::DrawingArea, metrics: TerminalMetrics) {
    use std::io::Write;
    let Ok(mut file) = std::fs::File::create(path) else {
        return;
    };
    let _ = writeln!(
        file,
        "canvas_x={}\ncanvas_y={}\ncanvas_width={}\ncanvas_height={}\ncell_width={:.6}\nline_height={:.6}\ncols={}\nrows={}",
        widget.allocation().x(),
        widget.allocation().y(),
        widget.allocated_width(),
        widget.allocated_height(),
        metrics.cell.width,
        metrics.cell.height,
        metrics.size.cols,
        metrics.size.rows,
    );
}

fn apply_interaction_effects(
    effects: Vec<InteractionEffect>,
    workspace: &std::rc::Rc<std::cell::RefCell<TerminalWorkspace>>,
    selection: &std::rc::Rc<std::cell::Cell<Option<SelectionRange>>>,
    selection_text: &std::rc::Rc<std::cell::RefCell<Option<String>>>,
    selection_dirty: &std::rc::Rc<std::cell::Cell<bool>>,
    content: &std::rc::Rc<std::cell::RefCell<Option<RenderableContentOwned>>>,
) {
    for effect in effects {
        match effect {
            InteractionEffect::Write(bytes) => {
                let _ = workspace.borrow_mut().write_active(&bytes);
            }
            InteractionEffect::SelectionChanged(range) => {
                crate::logging::debug_log(&format!("selection changed {range:?}"));
                if let Some(range) = range {
                    if let Some(content) = content.borrow().as_ref() {
                        selection.set(Some(anchor_range_to_display(range, content.display_offset)));
                        *selection_text.borrow_mut() = text_for_viewport_selection(content, range);
                        selection_dirty.set(true);
                    }
                } else {
                    clear_selection(selection, selection_text, selection_dirty);
                }
            }
            InteractionEffect::MoveCursorTo(position) => {
                if let Some(content) = content.borrow().as_ref()
                    && let Some(bytes) = cursor_movement_bytes_for_content(content, position)
                {
                    let _ = workspace.borrow_mut().write_active(&bytes);
                }
            }
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
    width: f64,
    height: f64,
}

fn terminal_metrics_for_widget(widget: &gtk::DrawingArea) -> Option<TerminalMetrics> {
    let width = widget.allocated_width();
    let height = widget.allocated_height();
    if width <= 0 || height <= 0 {
        return None;
    }
    let font_metrics = metrics_for_widget(widget)?;
    let cols =
        ((width as f64 / font_metrics.cell_width).floor() as i32).clamp(1, u16::MAX as i32) as u16;
    let rows = ((height as f64 / font_metrics.line_height).floor() as i32).clamp(1, u16::MAX as i32)
        as u16;
    Some(TerminalMetrics {
        size: ScreenSize::new(cols, rows).ok()?,
        cell: CellMetrics {
            width: font_metrics.cell_width,
            height: font_metrics.line_height,
        },
    })
}

fn pointer_grid_position(
    metrics: Option<CellMetrics>,
    x: f64,
    y: f64,
) -> Option<MouseGridPosition> {
    let metrics = metrics?;
    if x < 0.0 || y < 0.0 || metrics.width <= 0.0 || metrics.height <= 0.0 {
        return None;
    }
    Some(MouseGridPosition {
        column: ((x / metrics.width).floor() as i32).clamp(0, u16::MAX as i32) as u16,
        row: ((y / metrics.height).floor() as i32).clamp(0, u16::MAX as i32) as u16,
    })
}

fn pointer_cursor_position(
    metrics: Option<CellMetrics>,
    x: f64,
    y: f64,
) -> Option<MouseGridPosition> {
    let metrics = metrics?;
    if x < 0.0 || y < 0.0 || metrics.width <= 0.0 || metrics.height <= 0.0 {
        return None;
    }
    Some(MouseGridPosition {
        column: ((x / metrics.width).round() as i32).clamp(0, u16::MAX as i32) as u16,
        row: ((y / metrics.height).floor() as i32).clamp(0, u16::MAX as i32) as u16,
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
