use crate::backend::{MouseMode, RenderableContentOwned, ScreenSize};
use crate::canvas::TerminalCanvas;
use crate::cell_text::lines_to_text;
use crate::command_blocks::{
    command_block_output_range, command_block_output_range_near_cursor, command_blocks,
};
use crate::containers::{LaunchTarget, available_launch_targets};
use crate::input::{
    CursorDirection, CursorUnit, KeyAction, cursor_move_terminal_bytes, key_to_action,
};
use crate::input_selection::{
    DirectedSelectionRange, active_cursor_point, active_input_line_range,
    cursor_movement_bytes_between_points, directed_selection_for_target,
    input_buffer_offset_for_position, keyboard_cursor_bytes, keyboard_cursor_target,
    keyboard_selection_collapse_target, selection_within_active_input,
};
use crate::interaction::{
    InteractionEffect, PointerInteraction, cursor_movement_bytes_between_editable_input_points,
    cursor_movement_bytes_for_content, cursor_movement_bytes_for_editable_input,
    input_position_in_editable_input,
};
use crate::mouse::{MouseButton, MouseGridPosition};
use crate::render::{RenderFrame, RenderPreedit, Renderer};
use crate::selection::{
    GridPoint, SelectionRange, anchor_range_to_display, find_text_range, line_range,
    selected_text_with_metadata, viewport_range_for_display, word_range_at,
};
use crate::snapshot::{
    write_render_frame_snapshot, write_snapshot_with_selection, write_workspace_render_snapshot,
};
use crate::terminal_font::metrics_for_widget;
use crate::terminal_grid::TerminalSemanticPrompt;
use crate::text_width::{display_columns, display_columns_until_char};
use crate::workspace::{PaneId, TabId, TerminalWorkspace};
use crate::workspace_render::{WorkspaceRenderFrame, WorkspaceRenderLayout};
use adw::Application;
use adw::prelude::*;
use gtk::{gio, glib};

const APP_ID: &str = "com.chelokot.Chelotype";
const APP_NAME: &str = "Chelotype";
const GITHUB_REPO_URL: &str = "https://github.com/chelokot/Chelotype";

pub fn run_app() -> glib::ExitCode {
    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    install_app_accelerators(&app);
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
        .focus_on_click(false)
        .build();
    let launch_menu_icon = gtk::Image::from_icon_name("pan-down-symbolic");
    let launch_menu_button = gtk::MenuButton::builder()
        .tooltip_text("Show profiles and containers")
        .child(&launch_menu_icon)
        .focus_on_click(false)
        .build();
    let launcher = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .css_name("splitbutton")
        .css_classes(["image-button"])
        .valign(gtk::Align::Center)
        .build();
    launcher.append(&new_tab_button);
    launcher.append(
        &gtk::Separator::builder()
            .orientation(gtk::Orientation::Vertical)
            .build(),
    );
    launcher.append(&launch_menu_button);
    let app_menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text("Main menu")
        .menu_model(&main_menu_model())
        .build();
    let header = adw::HeaderBar::new();
    header.add_css_class("terminal-header");
    header.pack_start(&launcher);
    header.set_title_widget(Some(&tab_bar));
    header.pack_end(&app_menu_button);

    let canvas = TerminalCanvas::new();
    let style_provider = gtk::CssProvider::new();
    let pending_style_refresh = std::rc::Rc::new(std::cell::Cell::new(false));

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("term-root");
    content.append(&header);
    content.append(canvas.widget());
    content.append(&tab_view);

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Chelotype Terminal")
        .default_width(1760)
        .default_height(990)
        .content(&content)
        .build();
    let last_size = std::rc::Rc::new(std::cell::Cell::new(None::<ScreenSize>));
    install_window_actions(
        &window,
        app,
        canvas.clone(),
        workspace_rc.clone(),
        force_snapshot.clone(),
        pending_style_refresh.clone(),
        last_size.clone(),
    );
    install_style(canvas.widget(), &style_provider);
    let snapshot_enabled = std::env::var("CHELOTYPE_SNAPSHOT").ok().as_deref() == Some("1");
    let render_snapshot_enabled =
        std::env::var("CHELOTYPE_RENDER_SNAPSHOT").ok().as_deref() == Some("1");
    let last_snapshot = std::rc::Rc::new(std::cell::RefCell::new(std::time::Instant::now()));
    let ui_e2e = UiE2eScenario::from_env();
    let ui_e2e_deadline = ui_e2e
        .as_ref()
        .map(|scenario| std::time::Instant::now() + scenario.timeout);
    let cell_metrics = std::rc::Rc::new(std::cell::Cell::new(None::<CellMetrics>));
    let active_pane_origin_col = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let pane_hits = std::rc::Rc::new(std::cell::RefCell::new(Vec::<PaneHit>::new()));
    let geometry_trace = std::env::var("CHELOTYPE_GEOMETRY_TRACE")
        .ok()
        .map(std::path::PathBuf::from);
    let cached_terminal_metrics =
        std::rc::Rc::new(std::cell::Cell::new(None::<CachedTerminalMetrics>));
    let scroll_trace = std::env::var("CHELOTYPE_SCROLL_TRACE")
        .ok()
        .map(std::path::PathBuf::from);
    let tab_trace = std::env::var("CHELOTYPE_TAB_TRACE")
        .ok()
        .map(std::path::PathBuf::from);
    let suppress_motion_button_mask = std::env::var("CHELOTYPE_TEST_SUPPRESS_MOTION_BUTTON_MASK")
        .ok()
        .as_deref()
        == Some("1");
    let synthetic_click_cancel_on_drag_begin =
        std::env::var("CHELOTYPE_TEST_CLICK_CANCEL_ON_DRAG_BEGIN")
            .ok()
            .as_deref()
            == Some("1");
    let mouse_mode = std::rc::Rc::new(std::cell::Cell::new(crate::backend::MouseMode::default()));
    let pointer_interaction =
        std::rc::Rc::new(std::cell::RefCell::new(PointerInteraction::default()));
    let pointer_pane_capture = std::rc::Rc::new(std::cell::Cell::new(None::<PaneHit>));
    let split_resize_drag = std::rc::Rc::new(std::cell::Cell::new(None::<SplitResizeDrag>));
    let drag_gesture_moved = std::rc::Rc::new(std::cell::Cell::new(false));
    let drag_gesture_active = std::rc::Rc::new(std::cell::Cell::new(false));
    let left_pointer_down = std::rc::Rc::new(std::cell::Cell::new(false));
    let input_selection_drag = std::rc::Rc::new(std::cell::Cell::new(None::<InputSelectionDrag>));
    let smooth_scroll = std::rc::Rc::new(std::cell::RefCell::new(
        crate::smooth_scroll::SmoothScroll::default(),
    ));
    let selection = std::rc::Rc::new(std::cell::Cell::new(None::<SelectionRange>));
    let selection_text = std::rc::Rc::new(std::cell::RefCell::new(None::<String>));
    let selection_dirty = std::rc::Rc::new(std::cell::Cell::new(false));
    let keyboard_selection = std::rc::Rc::new(std::cell::Cell::new(None::<DirectedSelectionRange>));
    let preedit = std::rc::Rc::new(std::cell::RefCell::new(None::<PendingPreedit>));
    let last_content = std::rc::Rc::new(std::cell::RefCell::new(None::<RenderableContentOwned>));
    let pending_input_latency =
        std::rc::Rc::new(std::cell::RefCell::new(std::collections::VecDeque::<
            PendingInputSample,
        >::new()));
    add_existing_workspace_pages(&tab_view, &tab_pages, &workspace_rc.borrow());
    let tab_context = TabContext {
        workspace: workspace_rc.clone(),
        tab_view: tab_view.clone(),
        tab_pages: tab_pages.clone(),
        force_snapshot: force_snapshot.clone(),
        selection: selection.clone(),
        selection_text: selection_text.clone(),
        selection_dirty: selection_dirty.clone(),
        keyboard_selection: keyboard_selection.clone(),
    };

    configure_launch_menu(&launch_menu_button, tab_context.clone(), last_size.clone());
    install_tab_actions(&window, &tab_bar, tab_context.clone(), last_size.clone());
    connect_native_tabs(&tab_view, tab_context.clone(), last_size.clone());
    {
        let tabs = tab_context.clone();
        let last_size = last_size.clone();
        new_tab_button.connect_clicked(move |_| {
            add_launch_target_tab(
                LaunchTarget::Host,
                &LaunchMenuContext {
                    tabs: tabs.clone(),
                    last_size: last_size.clone(),
                    popover: gtk::Popover::new(),
                },
            );
        });
    }
    {
        let tabs = tab_context.clone();
        window.connect_close_request(move |_| {
            remember_single_tab_launch_target(&tabs);
            glib::Propagation::Proceed
        });
    }

    let key_controller = gtk::EventControllerKey::new();
    key_controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    let im_context = gtk::IMMulticontext::new();
    im_context.set_client_widget(Some(canvas.widget()));
    im_context.set_use_preedit(true);
    key_controller.set_im_context(Some(&im_context));
    {
        let workspace = workspace_rc.clone();
        let content = last_content.clone();
        let selection = selection.clone();
        let selection_text = selection_text.clone();
        let selection_dirty = selection_dirty.clone();
        let keyboard_selection = keyboard_selection.clone();
        let preedit = preedit.clone();
        let force_snapshot = force_snapshot.clone();
        let pending_input_latency = pending_input_latency.clone();
        im_context.connect_commit(move |_context, committed| {
            if committed.is_empty() {
                return;
            }
            if preedit.borrow_mut().take().is_some() {
                force_snapshot.set(true);
            }
            mark_pending_input_latency(&pending_input_latency);
            write_key_with_selection(
                &workspace,
                &content,
                &selection,
                &selection_text,
                &selection_dirty,
                &keyboard_selection,
                committed.as_bytes().to_vec(),
            );
        });
    }
    {
        let preedit = preedit.clone();
        let force_snapshot = force_snapshot.clone();
        im_context.connect_preedit_changed(move |context| {
            let (text, _attrs, cursor) = context.preedit_string();
            let text = text.to_string();
            *preedit.borrow_mut() = if text.is_empty() {
                None
            } else {
                Some(PendingPreedit {
                    text,
                    cursor: cursor.max(0) as usize,
                })
            };
            force_snapshot.set(true);
        });
    }
    let focus_controller = gtk::EventControllerFocus::new();
    {
        let im_context = im_context.clone();
        focus_controller.connect_enter(move |_| {
            im_context.focus_in();
        });
    }
    {
        let im_context = im_context.clone();
        focus_controller.connect_leave(move |_| {
            im_context.focus_out();
        });
    }
    canvas.widget().add_controller(focus_controller);
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
        let tabs = tab_context.clone();
        let last_size = last_size.clone();
        let keyboard_selection = keyboard_selection.clone();
        let pending_input_latency = pending_input_latency.clone();
        let window = window.clone();
        let canvas = canvas.clone();
        let pending_style_refresh = pending_style_refresh.clone();
        key_controller.connect_key_pressed(move |_ctrl, key, keycode, state| {
            if let Some(action) = key_to_action(key, keycode, state) {
                match action {
                    KeyAction::Write(data) => {
                        if data.as_slice() == [0x03] && selection_text.borrow().is_some() {
                            copy_selection_to_clipboard(&canvas_widget, &selection_text);
                        } else {
                            mark_pending_input_latency(&pending_input_latency);
                            write_key_with_selection(
                                &workspace,
                                &content,
                                &selection,
                                &selection_text,
                                &selection_dirty,
                                &keyboard_selection,
                                data,
                            );
                        }
                    }
                    KeyAction::CursorMove {
                        direction,
                        unit,
                        selecting,
                    } => {
                        mark_pending_input_latency(&pending_input_latency);
                        move_cursor_from_keyboard(
                            &workspace,
                            &content,
                            &selection,
                            &selection_text,
                            &selection_dirty,
                            &keyboard_selection,
                            KeyboardMove {
                                direction,
                                unit,
                                selecting,
                            },
                        );
                    }
                    KeyAction::SelectInput => {
                        mark_pending_input_latency(&pending_input_latency);
                        select_active_input(
                            &content,
                            &selection,
                            &selection_text,
                            &selection_dirty,
                            &keyboard_selection,
                        );
                    }
                    KeyAction::SelectCommandBlockOutput(direction) => {
                        mark_pending_input_latency(&pending_input_latency);
                        select_command_block_output(
                            &content,
                            direction,
                            &selection,
                            &selection_text,
                            &selection_dirty,
                            &keyboard_selection,
                        );
                    }
                    KeyAction::ScrollDisplay(lines) => {
                        mark_pending_input_latency(&pending_input_latency);
                        let _ = workspace.borrow_mut().scroll_active(lines);
                    }
                    KeyAction::CopySelection => {
                        if !copy_selection_to_clipboard(&canvas_widget, &selection_text) {
                            mark_pending_input_latency(&pending_input_latency);
                            write_key_with_selection(
                                &workspace,
                                &content,
                                &selection,
                                &selection_text,
                                &selection_dirty,
                                &keyboard_selection,
                                vec![0x03],
                            );
                        }
                    }
                    KeyAction::CutSelection => {
                        if copy_selection_to_clipboard(&canvas_widget, &selection_text) {
                            mark_pending_input_latency(&pending_input_latency);
                            write_key_with_selection(
                                &workspace,
                                &content,
                                &selection,
                                &selection_text,
                                &selection_dirty,
                                &keyboard_selection,
                                b"\x1b[3~".to_vec(),
                            );
                        } else {
                            mark_pending_input_latency(&pending_input_latency);
                            write_key_with_selection(
                                &workspace,
                                &content,
                                &selection,
                                &selection_text,
                                &selection_dirty,
                                &keyboard_selection,
                                vec![0x18],
                            );
                        }
                    }
                    KeyAction::PasteClipboard => {
                        paste_clipboard_text(
                            &canvas_widget,
                            PasteClipboardContext {
                                workspace: workspace.clone(),
                                content: content.clone(),
                                selection: selection.clone(),
                                selection_text: selection_text.clone(),
                                selection_dirty: selection_dirty.clone(),
                                keyboard_selection: keyboard_selection.clone(),
                                pending_input_latency: pending_input_latency.clone(),
                            },
                        );
                    }
                    KeyAction::UndoInput => {
                        mark_pending_input_latency(&pending_input_latency);
                        write_input_undo(&workspace, &content);
                    }
                    KeyAction::RedoInput => {
                        mark_pending_input_latency(&pending_input_latency);
                        write_input_redo(&workspace, &content);
                    }
                    KeyAction::ZoomIn => {
                        mark_pending_input_latency(&pending_input_latency);
                        crate::terminal_font::zoom_in();
                        last_size.set(None);
                        force_snapshot.set(true);
                        canvas_widget.queue_draw();
                    }
                    KeyAction::ZoomOut => {
                        mark_pending_input_latency(&pending_input_latency);
                        crate::terminal_font::zoom_out();
                        last_size.set(None);
                        force_snapshot.set(true);
                        canvas_widget.queue_draw();
                    }
                    KeyAction::ZoomReset => {
                        mark_pending_input_latency(&pending_input_latency);
                        crate::terminal_font::zoom_reset();
                        last_size.set(None);
                        force_snapshot.set(true);
                        canvas_widget.queue_draw();
                    }
                    KeyAction::NewTab => {
                        add_launch_target_tab(
                            LaunchTarget::Host,
                            &LaunchMenuContext {
                                tabs: tabs.clone(),
                                last_size: last_size.clone(),
                                popover: gtk::Popover::new(),
                            },
                        );
                    }
                    KeyAction::NewWindow => {
                        if let Some(app) = window.application().and_downcast::<Application>() {
                            build_ui(&app);
                        }
                    }
                    KeyAction::CloseTab => {
                        if let Some(id) = active_tab_id(&tabs) {
                            close_tab(&tabs, id);
                        }
                    }
                    KeyAction::SplitPane => {
                        if workspace
                            .borrow_mut()
                            .split_shell_active_with_size(last_size.get())
                            .is_ok()
                        {
                            *content.borrow_mut() = None;
                            canvas_widget.grab_focus();
                            force_active_workspace_snapshot(
                                &force_snapshot,
                                &selection,
                                &selection_text,
                                &selection_dirty,
                                &keyboard_selection,
                                last_size.get(),
                                &workspace,
                            );
                        }
                    }
                    KeyAction::NextTab => {
                        workspace.borrow_mut().activate_next();
                        let active = workspace.borrow().active_tab_id();
                        select_page_for_tab(&tab_view, &tab_pages, active);
                        force_active_workspace_snapshot(
                            &force_snapshot,
                            &selection,
                            &selection_text,
                            &selection_dirty,
                            &keyboard_selection,
                            last_size.get(),
                            &workspace,
                        );
                    }
                    KeyAction::PreviousTab => {
                        workspace.borrow_mut().activate_previous();
                        let active = workspace.borrow().active_tab_id();
                        select_page_for_tab(&tab_view, &tab_pages, active);
                        force_active_workspace_snapshot(
                            &force_snapshot,
                            &selection,
                            &selection_text,
                            &selection_dirty,
                            &keyboard_selection,
                            last_size.get(),
                            &workspace,
                        );
                    }
                    KeyAction::OpenSettings => {
                        show_preferences_dialog(
                            &window,
                            &canvas,
                            force_snapshot.clone(),
                            pending_style_refresh.clone(),
                            last_size.clone(),
                        );
                    }
                    KeyAction::OpenAbout => {
                        show_about_dialog(&window, workspace.borrow().tabs().len());
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
        let pane_hits = pane_hits.clone();
        let active_origin = active_pane_origin_col.clone();
        let mode = mouse_mode.clone();
        let selection = selection.clone();
        let selection_text = selection_text.clone();
        let selection_dirty = selection_dirty.clone();
        let keyboard_selection = keyboard_selection.clone();
        let content = last_content.clone();
        let pointer_interaction = pointer_interaction.clone();
        let pointer_pane_capture = pointer_pane_capture.clone();
        let drag_gesture_moved = drag_gesture_moved.clone();
        let drag_gesture_active = drag_gesture_active.clone();
        let left_pointer_down = left_pointer_down.clone();
        let input_selection_drag = input_selection_drag.clone();
        let canvas = canvas.clone();
        let canvas_widget = canvas.widget().clone();
        let pending_input_latency = pending_input_latency.clone();
        let context_menu = CanvasContextMenuContext {
            workspace: workspace.clone(),
            content: content.clone(),
            selection: selection.clone(),
            selection_text: selection_text.clone(),
            selection_dirty: selection_dirty.clone(),
            keyboard_selection: keyboard_selection.clone(),
            pending_input_latency: pending_input_latency.clone(),
            command_block_output: None,
        };
        click_controller.connect_pressed(move |gesture, press_count, x, y| {
            canvas_widget.grab_focus();
            left_pointer_down.set(false);
            drag_gesture_active.set(false);
            drag_gesture_moved.set(false);
            input_selection_drag.set(None);
            canvas.set_cursor_motion_suppressed(false);
            crate::logging::debug_log(&format!("mouse press x={x:.1} y={y:.1}"));
            if split_resize_boundary_at(metrics.get(), &pane_hits.borrow(), x).is_some() {
                pointer_pane_capture.set(None);
                let _ = pointer_interaction.borrow_mut().cancel();
                return;
            }
            if let Some(target) =
                pointer_grid_position_for_panes(metrics.get(), &pane_hits.borrow(), x, y)
            {
                pointer_pane_capture.set(target.pane);
                activate_pointer_pane(
                    target,
                    PointerPaneActivation {
                        workspace: &workspace,
                        content: &content,
                        mouse_mode: &mode,
                        active_origin: &active_origin,
                        selection: &selection,
                        selection_text: &selection_text,
                        selection_dirty: &selection_dirty,
                        keyboard_selection: &keyboard_selection,
                    },
                );
                crate::logging::debug_log(&format!(
                    "mouse press grid col={} row={}",
                    target.position.column, target.position.row
                ));
                let current_mode = current_mouse_mode(&content, mode.get());
                crate::logging::debug_log(&format!("mouse press mode={current_mode:?}"));
                let button = mouse_button_from_gesture(gesture).unwrap_or(MouseButton::Left);
                left_pointer_down.set(button == MouseButton::Left);
                if button == MouseButton::Right && !current_mode.sends_press_release() {
                    left_pointer_down.set(false);
                    let command_block_output = content.borrow().as_ref().and_then(|content| {
                        command_block_output_text_at_rail(content, metrics.get(), target, x)
                    });
                    show_canvas_context_menu(
                        &canvas_widget,
                        x,
                        y,
                        CanvasContextMenuContext {
                            command_block_output,
                            ..context_menu.clone()
                        },
                    );
                    let _ = pointer_interaction.borrow_mut().cancel();
                    return;
                }
                if button == MouseButton::Left
                    && !current_mode.sends_press_release()
                    && let Some(content) = content.borrow().clone()
                    && let Some(range) =
                        command_block_output_selection_at_rail(&content, metrics.get(), target, x)
                {
                    set_viewport_selection(
                        &content,
                        range,
                        &selection,
                        &selection_text,
                        &selection_dirty,
                        &keyboard_selection,
                    );
                    drag_gesture_moved.set(true);
                    let _ = pointer_interaction.borrow_mut().cancel();
                    return;
                }
                if !current_mode.sends_press_release()
                    && select_mouse_click_range(
                        &content,
                        &selection,
                        &selection_text,
                        &selection_dirty,
                        &keyboard_selection,
                        target.position,
                        press_count,
                    )
                {
                    drag_gesture_moved.set(true);
                    let _ = pointer_interaction.borrow_mut().cancel();
                    return;
                }
                let effects =
                    pointer_interaction
                        .borrow_mut()
                        .press(current_mode, button, target.position);
                if button == MouseButton::Left && !current_mode.sends_press_release() {
                    input_selection_drag.set(content.borrow().as_ref().and_then(|content| {
                        input_selection_drag_for_content(content, target.position)
                    }));
                }
                crate::logging::debug_log(&format!("mouse press effects={effects:?}"));
                apply_interaction_effects(
                    effects,
                    &workspace,
                    &selection,
                    &selection_text,
                    &selection_dirty,
                    &keyboard_selection,
                    &content,
                );
            }
        });
    }
    {
        let workspace = workspace_rc.clone();
        let metrics = cell_metrics.clone();
        let pane_hits = pane_hits.clone();
        let active_origin = active_pane_origin_col.clone();
        let mode = mouse_mode.clone();
        let selection = selection.clone();
        let selection_text = selection_text.clone();
        let selection_dirty = selection_dirty.clone();
        let keyboard_selection = keyboard_selection.clone();
        let content = last_content.clone();
        let pointer_interaction = pointer_interaction.clone();
        let pointer_pane_capture = pointer_pane_capture.clone();
        let drag_gesture_moved = drag_gesture_moved.clone();
        let drag_gesture_active = drag_gesture_active.clone();
        let left_pointer_down = left_pointer_down.clone();
        let input_selection_drag = input_selection_drag.clone();
        let canvas = canvas.clone();
        click_controller.connect_released(move |_gesture, _press_count, x, y| {
            crate::logging::debug_log(&format!("mouse release x={x:.1} y={y:.1}"));
            left_pointer_down.set(false);
            drag_gesture_active.set(false);
            input_selection_drag.set(None);
            canvas.set_cursor_motion_suppressed(false);
            let current_mode = current_mouse_mode(&content, mode.get());
            if drag_gesture_moved.get() && !current_mode.sends_press_release() {
                pointer_pane_capture.set(None);
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
                    &keyboard_selection,
                    &content,
                );
                return;
            }
            if let Some(target) = pointer_grid_position_for_capture_or_panes(
                metrics.get(),
                pointer_pane_capture.get(),
                &pane_hits.borrow(),
                x,
                y,
            ) {
                activate_pointer_pane(
                    target,
                    PointerPaneActivation {
                        workspace: &workspace,
                        content: &content,
                        mouse_mode: &mode,
                        active_origin: &active_origin,
                        selection: &selection,
                        selection_text: &selection_text,
                        selection_dirty: &selection_dirty,
                        keyboard_selection: &keyboard_selection,
                    },
                );
                let position = if current_mode.sends_press_release() {
                    target.position
                } else {
                    pointer_cursor_position_for_target(metrics.get(), target, x, y)
                        .unwrap_or(target.position)
                };
                crate::logging::debug_log(&format!(
                    "mouse release grid col={} row={}",
                    position.column, position.row
                ));
                let effects = pointer_interaction
                    .borrow_mut()
                    .release(current_mode, position);
                crate::logging::debug_log(&format!("mouse release effects={effects:?}"));
                apply_interaction_effects(
                    effects,
                    &workspace,
                    &selection,
                    &selection_text,
                    &selection_dirty,
                    &keyboard_selection,
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
                    &keyboard_selection,
                    &content,
                );
            }
            pointer_pane_capture.set(None);
        });
    }
    {
        let workspace = workspace_rc.clone();
        let selection = selection.clone();
        let selection_text = selection_text.clone();
        let selection_dirty = selection_dirty.clone();
        let keyboard_selection = keyboard_selection.clone();
        let content = last_content.clone();
        let pointer_interaction = pointer_interaction.clone();
        let pointer_pane_capture = pointer_pane_capture.clone();
        let drag_gesture_active = drag_gesture_active.clone();
        let left_pointer_down = left_pointer_down.clone();
        let input_selection_drag = input_selection_drag.clone();
        let canvas = canvas.clone();
        gtk::prelude::GestureExt::connect_cancel(&click_controller, move |_gesture, _sequence| {
            input_selection_drag.set(None);
            canvas.set_cursor_motion_suppressed(false);
            let effects = cancel_click_gesture(
                &drag_gesture_active,
                &left_pointer_down,
                &pointer_pane_capture,
                &pointer_interaction,
            );
            crate::logging::debug_log(&format!("mouse gesture cancelled effects={effects:?}"));
            apply_interaction_effects(
                effects,
                &workspace,
                &selection,
                &selection_text,
                &selection_dirty,
                &keyboard_selection,
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
        let pane_hits = pane_hits.clone();
        let last_size = last_size.clone();
        let force_snapshot = force_snapshot.clone();
        let active_origin = active_pane_origin_col.clone();
        let mode = mouse_mode.clone();
        let selection = selection.clone();
        let selection_text = selection_text.clone();
        let selection_dirty = selection_dirty.clone();
        let keyboard_selection = keyboard_selection.clone();
        let content = last_content.clone();
        let pointer_interaction = pointer_interaction.clone();
        let pointer_pane_capture = pointer_pane_capture.clone();
        let split_resize_drag = split_resize_drag.clone();
        let drag_gesture_moved = drag_gesture_moved.clone();
        let drag_gesture_active = drag_gesture_active.clone();
        let left_pointer_down = left_pointer_down.clone();
        let input_selection_drag = input_selection_drag.clone();
        let canvas_widget = canvas.widget().clone();
        drag_controller.connect_drag_begin(move |_gesture, x, y| {
            if current_mouse_mode(&content, mode.get()).sends_press_release() {
                return;
            }
            drag_gesture_active.set(true);
            left_pointer_down.set(true);
            canvas_widget.grab_focus();
            drag_gesture_moved.set(false);
            if let Some(boundary_index) =
                split_resize_boundary_at(metrics.get(), &pane_hits.borrow(), x)
            {
                crate::logging::debug_log(&format!(
                    "split resize begin boundary={boundary_index} x={x:.1}"
                ));
                split_resize_drag.set(Some(SplitResizeDrag {
                    boundary_index,
                    start_x: x,
                    applied_delta_cols: 0,
                }));
                drag_gesture_moved.set(true);
                pointer_pane_capture.set(None);
                let _ = pointer_interaction.borrow_mut().cancel();
                if let Some(size) = last_size.get() {
                    let _ = workspace.borrow_mut().resize_active_tab(size);
                }
                force_snapshot.set(true);
                canvas_widget.queue_draw();
                return;
            }
            if let Some(target) =
                pointer_grid_position_for_panes(metrics.get(), &pane_hits.borrow(), x, y)
            {
                pointer_pane_capture.set(target.pane);
                activate_pointer_pane(
                    target,
                    PointerPaneActivation {
                        workspace: &workspace,
                        content: &content,
                        mouse_mode: &mode,
                        active_origin: &active_origin,
                        selection: &selection,
                        selection_text: &selection_text,
                        selection_dirty: &selection_dirty,
                        keyboard_selection: &keyboard_selection,
                    },
                );
                let effects = pointer_interaction.borrow_mut().press(
                    MouseMode::default(),
                    MouseButton::Left,
                    target.position,
                );
                input_selection_drag.set(content.borrow().as_ref().and_then(|content| {
                    input_selection_drag_for_content(content, target.position)
                }));
                crate::logging::debug_log(&format!("drag begin effects={effects:?}"));
                apply_interaction_effects(
                    effects,
                    &workspace,
                    &selection,
                    &selection_text,
                    &selection_dirty,
                    &keyboard_selection,
                    &content,
                );
                if synthetic_click_cancel_on_drag_begin {
                    let effects = cancel_click_gesture(
                        &drag_gesture_active,
                        &left_pointer_down,
                        &pointer_pane_capture,
                        &pointer_interaction,
                    );
                    crate::logging::debug_log(&format!(
                        "synthetic click cancel during drag begin effects={effects:?}"
                    ));
                    apply_interaction_effects(
                        effects,
                        &workspace,
                        &selection,
                        &selection_text,
                        &selection_dirty,
                        &keyboard_selection,
                        &content,
                    );
                }
            }
        });
    }
    {
        let workspace = workspace_rc.clone();
        let metrics = cell_metrics.clone();
        let pane_hits = pane_hits.clone();
        let last_size = last_size.clone();
        let force_snapshot = force_snapshot.clone();
        let mode = mouse_mode.clone();
        let selection = selection.clone();
        let selection_text = selection_text.clone();
        let selection_dirty = selection_dirty.clone();
        let keyboard_selection = keyboard_selection.clone();
        let content = last_content.clone();
        let pointer_interaction = pointer_interaction.clone();
        let pointer_pane_capture = pointer_pane_capture.clone();
        let split_resize_drag = split_resize_drag.clone();
        let drag_gesture_moved = drag_gesture_moved.clone();
        let input_selection_drag = input_selection_drag.clone();
        let canvas = canvas.clone();
        let canvas_widget = canvas.widget().clone();
        drag_controller.connect_drag_update(move |gesture, offset_x, offset_y| {
            if current_mouse_mode(&content, mode.get()).sends_press_release() {
                return;
            }
            let Some((start_x, start_y)) = gesture.start_point() else {
                return;
            };
            if let Some(mut resize) = split_resize_drag.get() {
                if let (Some(metrics), Some(size)) = (metrics.get(), last_size.get()) {
                    let current_delta_cols =
                        ((start_x + offset_x - resize.start_x) / metrics.width).round() as i16;
                    let incremental_delta = current_delta_cols - resize.applied_delta_cols;
                    if incremental_delta != 0 {
                        let applied_delta = {
                            let mut workspace = workspace.borrow_mut();
                            let before = workspace
                                .active_tab_pane_columns(size.cols)
                                .get(resize.boundary_index)
                                .copied()
                                .unwrap_or_default()
                                as i16;
                            if workspace
                                .resize_active_tab_split(
                                    resize.boundary_index,
                                    incremental_delta,
                                    size,
                                )
                                .unwrap_or(false)
                            {
                                workspace
                                    .active_tab_pane_columns(size.cols)
                                    .get(resize.boundary_index)
                                    .copied()
                                    .unwrap_or_default() as i16
                                    - before
                            } else {
                                0
                            }
                        };
                        if applied_delta == 0 {
                            crate::logging::debug_log(&format!(
                                "split resize clamped boundary={} requested_delta={incremental_delta}",
                                resize.boundary_index
                            ));
                            drag_gesture_moved.set(true);
                            return;
                        }
                        crate::logging::debug_log(&format!(
                            "split resize update boundary={} requested_delta={incremental_delta} applied_delta={applied_delta}",
                            resize.boundary_index
                        ));
                        resize.applied_delta_cols += applied_delta;
                        split_resize_drag.set(Some(resize));
                        force_snapshot.set(true);
                        canvas_widget.queue_draw();
                    }
                }
                drag_gesture_moved.set(true);
                return;
            }
            if let Some(target) = pointer_grid_position_for_capture_or_panes(
                metrics.get(),
                pointer_pane_capture.get(),
                &pane_hits.borrow(),
                start_x + offset_x,
                start_y + offset_y,
            ) {
                let effects = pointer_interaction
                    .borrow_mut()
                    .motion(MouseMode::default(), target.position);
                let selection_changed = effects
                    .iter()
                    .any(|effect| matches!(effect, InteractionEffect::SelectionChanged(Some(_))));
                if selection_changed {
                    drag_gesture_moved.set(true);
                }
                crate::logging::debug_log(&format!("drag update effects={effects:?}"));
                apply_interaction_effects(
                    effects,
                    &workspace,
                    &selection,
                    &selection_text,
                    &selection_dirty,
                    &keyboard_selection,
                    &content,
                );
                if selection_changed
                    && follow_input_selection_cursor(
                        &input_selection_drag,
                        &workspace,
                        &content,
                        &canvas,
                        target.position,
                    )
                {
                    force_snapshot.set(true);
                    canvas_widget.queue_draw();
                }
            }
        });
    }
    {
        let workspace = workspace_rc.clone();
        let metrics = cell_metrics.clone();
        let pane_hits = pane_hits.clone();
        let mode = mouse_mode.clone();
        let selection = selection.clone();
        let selection_text = selection_text.clone();
        let selection_dirty = selection_dirty.clone();
        let keyboard_selection = keyboard_selection.clone();
        let content = last_content.clone();
        let pointer_interaction = pointer_interaction.clone();
        let pointer_pane_capture = pointer_pane_capture.clone();
        let split_resize_drag = split_resize_drag.clone();
        let drag_gesture_moved = drag_gesture_moved.clone();
        let drag_gesture_active = drag_gesture_active.clone();
        let left_pointer_down = left_pointer_down.clone();
        let input_selection_drag = input_selection_drag.clone();
        let canvas = canvas.clone();
        drag_controller.connect_drag_end(move |gesture, offset_x, offset_y| {
            left_pointer_down.set(false);
            drag_gesture_active.set(false);
            input_selection_drag.set(None);
            canvas.set_cursor_motion_suppressed(false);
            if current_mouse_mode(&content, mode.get()).sends_press_release() {
                return;
            }
            if split_resize_drag.replace(None).is_some() {
                drag_gesture_moved.set(false);
                return;
            }
            let Some((start_x, start_y)) = gesture.start_point() else {
                return;
            };
            let effects = if let Some(target) = pointer_grid_position_for_capture_or_panes(
                metrics.get(),
                pointer_pane_capture.get(),
                &pane_hits.borrow(),
                start_x + offset_x,
                start_y + offset_y,
            ) {
                pointer_interaction
                    .borrow_mut()
                    .release(MouseMode::default(), target.position)
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
                &keyboard_selection,
                &content,
            );
            pointer_pane_capture.set(None);
        });
    }
    {
        let workspace = workspace_rc.clone();
        let selection = selection.clone();
        let selection_text = selection_text.clone();
        let selection_dirty = selection_dirty.clone();
        let keyboard_selection = keyboard_selection.clone();
        let content = last_content.clone();
        let pointer_interaction = pointer_interaction.clone();
        let pointer_pane_capture = pointer_pane_capture.clone();
        let split_resize_drag = split_resize_drag.clone();
        let drag_gesture_moved = drag_gesture_moved.clone();
        let drag_gesture_active = drag_gesture_active.clone();
        let left_pointer_down = left_pointer_down.clone();
        let input_selection_drag = input_selection_drag.clone();
        let canvas = canvas.clone();
        gtk::prelude::GestureExt::connect_cancel(&drag_controller, move |_gesture, _sequence| {
            left_pointer_down.set(false);
            drag_gesture_active.set(false);
            drag_gesture_moved.set(false);
            input_selection_drag.set(None);
            canvas.set_cursor_motion_suppressed(false);
            pointer_pane_capture.set(None);
            split_resize_drag.set(None);
            let effects = pointer_interaction.borrow_mut().cancel();
            crate::logging::debug_log(&format!("drag gesture cancelled effects={effects:?}"));
            apply_interaction_effects(
                effects,
                &workspace,
                &selection,
                &selection_text,
                &selection_dirty,
                &keyboard_selection,
                &content,
            );
        });
    }
    canvas.widget().add_controller(drag_controller);

    let motion_controller = gtk::EventControllerMotion::new();
    {
        let workspace = workspace_rc.clone();
        let metrics = cell_metrics.clone();
        let pane_hits = pane_hits.clone();
        let force_snapshot = force_snapshot.clone();
        let mode = mouse_mode.clone();
        let selection = selection.clone();
        let selection_text = selection_text.clone();
        let selection_dirty = selection_dirty.clone();
        let keyboard_selection = keyboard_selection.clone();
        let content = last_content.clone();
        let pointer_interaction = pointer_interaction.clone();
        let pointer_pane_capture = pointer_pane_capture.clone();
        let split_resize_drag = split_resize_drag.clone();
        let drag_gesture_moved = drag_gesture_moved.clone();
        let left_pointer_down = left_pointer_down.clone();
        let input_selection_drag = input_selection_drag.clone();
        let canvas = canvas.clone();
        let canvas_widget = canvas.widget().clone();
        motion_controller.connect_motion(move |controller, x, y| {
            crate::logging::debug_log(&format!("mouse motion x={x:.1} y={y:.1}"));
            if split_resize_drag.get().is_some()
                || split_resize_boundary_at(metrics.get(), &pane_hits.borrow(), x).is_some()
            {
                if let Some(widget) = controller.widget() {
                    widget.set_cursor_from_name(Some("col-resize"));
                }
            } else if let Some(widget) = controller.widget() {
                widget.set_cursor_from_name(Some("text"));
            }
            let current_mode = current_mouse_mode(&content, mode.get());
            let motion_button_mask = !suppress_motion_button_mask
                && controller
                    .current_event_state()
                    .contains(gtk::gdk::ModifierType::BUTTON1_MASK);
            if !current_mode.sends_press_release()
                && (left_pointer_down.get() || motion_button_mask)
            {
                if let Some(target) = pointer_grid_position_for_capture_or_panes(
                    metrics.get(),
                    pointer_pane_capture.get(),
                    &pane_hits.borrow(),
                    x,
                    y,
                ) {
                    let effects = pointer_interaction
                        .borrow_mut()
                        .motion(MouseMode::default(), target.position);
                    let selection_changed = effects.iter().any(|effect| {
                        matches!(effect, InteractionEffect::SelectionChanged(Some(_)))
                    });
                    if selection_changed {
                        drag_gesture_moved.set(true);
                    }
                    crate::logging::debug_log(&format!(
                        "mouse selection motion effects={effects:?}"
                    ));
                    apply_interaction_effects(
                        effects,
                        &workspace,
                        &selection,
                        &selection_text,
                        &selection_dirty,
                        &keyboard_selection,
                        &content,
                    );
                    if selection_changed
                        && follow_input_selection_cursor(
                            &input_selection_drag,
                            &workspace,
                            &content,
                            &canvas,
                            target.position,
                        )
                    {
                        force_snapshot.set(true);
                        canvas_widget.queue_draw();
                    }
                }
                return;
            }
            if !current_mode.sends_drag() {
                return;
            }
            if let Some(target) = pointer_grid_position_for_capture_or_panes(
                metrics.get(),
                pointer_pane_capture.get(),
                &pane_hits.borrow(),
                x,
                y,
            ) {
                crate::logging::debug_log(&format!(
                    "mouse motion grid col={} row={}",
                    target.position.column, target.position.row
                ));
                let effects = pointer_interaction
                    .borrow_mut()
                    .motion(current_mode, target.position);
                crate::logging::debug_log(&format!("mouse motion effects={effects:?}"));
                apply_interaction_effects(
                    effects,
                    &workspace,
                    &selection,
                    &selection_text,
                    &selection_dirty,
                    &keyboard_selection,
                    &content,
                );
            }
        });
    }
    canvas.widget().add_controller(motion_controller);

    let scroll_controller = gtk::EventControllerScroll::new(
        gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::KINETIC,
    );
    {
        let workspace = workspace_rc.clone();
        let force_snapshot = force_snapshot.clone();
        let last_size = last_size.clone();
        let cell_metrics = cell_metrics.clone();
        let canvas = canvas.clone();
        let canvas_widget = canvas.widget().clone();
        let smooth_scroll = smooth_scroll.clone();
        let scroll_trace = scroll_trace.clone();
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
                if !crate::config::smooth_scrolling_enabled() {
                    smooth_scroll.borrow_mut().cancel();
                    canvas.set_scroll_visual_offset_px(0.0);
                    if workspace
                        .borrow_mut()
                        .scroll_active_changed(lines)
                        .unwrap_or(false)
                    {
                        force_snapshot.set(true);
                        canvas.widget().queue_draw();
                    } else {
                        trace_smooth_scroll(scroll_trace.as_deref(), "limit", lines, 0.0);
                    }
                    return glib::Propagation::Stop;
                }
                let Some(metrics) = cell_metrics.get() else {
                    if workspace
                        .borrow_mut()
                        .scroll_active_changed(lines)
                        .unwrap_or(false)
                    {
                        force_snapshot.set(true);
                    }
                    canvas_widget.queue_draw();
                    return glib::Propagation::Stop;
                };
                if let Some(delta_px) = crate::interaction::scroll_delta_pixels(dy, metrics.height)
                {
                    let available_lines = workspace
                        .borrow()
                        .available_active_scroll_lines(lines)
                        .unwrap_or(0);
                    if available_lines == 0 {
                        smooth_scroll.borrow_mut().cancel();
                        canvas.set_scroll_visual_offset_px(0.0);
                        trace_smooth_scroll(scroll_trace.as_deref(), "limit", lines, 0.0);
                        return glib::Propagation::Stop;
                    }
                    let mut scroll = smooth_scroll.borrow_mut();
                    let enqueued_px =
                        scroll.enqueue_pixels_clamped(delta_px, metrics.height, available_lines);
                    if enqueued_px.abs() < 0.5 {
                        trace_smooth_scroll(scroll_trace.as_deref(), "limit", lines, 0.0);
                        return glib::Propagation::Stop;
                    }
                    trace_smooth_scroll(
                        scroll_trace.as_deref(),
                        "enqueue",
                        lines,
                        scroll.remaining_px(),
                    );
                }
                canvas.widget().queue_draw();
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
    configure_profile_scroll_burst(
        &workspace_rc,
        &canvas,
        &cell_metrics,
        &smooth_scroll,
        scroll_trace.as_deref(),
    );

    let app_for_tick = app.clone();
    let last_frame_tick = std::rc::Rc::new(std::cell::Cell::new(None::<i64>));
    let last_wall_tick = std::rc::Rc::new(std::cell::Cell::new(None::<std::time::Instant>));
    let last_completed_frame_timing = std::rc::Rc::new(std::cell::Cell::new(None::<i64>));
    let last_presentation_time = std::rc::Rc::new(std::cell::Cell::new(None::<i64>));
    let profile_frame_baseline = std::env::var("CHELOTYPE_PROFILE_FRAME_BASELINE")
        .ok()
        .as_deref()
        == Some("1");
    let profile_timer_baseline = std::env::var("CHELOTYPE_PROFILE_TIMER_BASELINE")
        .ok()
        .as_deref()
        == Some("1");
    let profile_updating_baseline = std::env::var("CHELOTYPE_PROFILE_UPDATING_BASELINE")
        .ok()
        .as_deref()
        == Some("1");
    let started_frame_updating = std::rc::Rc::new(std::cell::Cell::new(false));
    if profile_timer_baseline {
        let timer_canvas = canvas.clone();
        let last_timer_tick = std::rc::Rc::new(std::cell::Cell::new(None::<std::time::Instant>));
        glib::timeout_add_local(crate::frame_timing::TARGET_FRAME_DURATION, move || {
            let now = std::time::Instant::now();
            if let Some(previous) = last_timer_tick.replace(Some(now)) {
                crate::perf_trace::record_duration(
                    "glib_timeout_interval",
                    now.duration_since(previous),
                );
            }
            timer_canvas.widget().queue_draw();
            glib::ControlFlow::Continue
        });
    }
    let media_preferences_force_snapshot = force_snapshot.clone();
    let tick_canvas = canvas.clone();
    let tick_style_provider = style_provider.clone();
    let tick_pending_style_refresh = pending_style_refresh.clone();
    let trace_window = window.clone();
    let tick_last_completed_frame_timing = last_completed_frame_timing.clone();
    let tick_last_presentation_time = last_presentation_time.clone();
    let tick_last_size = last_size.clone();
    canvas.widget().add_tick_callback(move |_, frame_clock| {
        let tick_wall_started = std::time::Instant::now();
        if profile_updating_baseline && !started_frame_updating.replace(true) {
            frame_clock.begin_updating();
            crate::perf_trace::record_counter("gdk_frame_clock_begin_updating", 1);
        }
        if profile_updating_baseline {
            crate::perf_trace::record_counter("gdk_frame_clock_updating_active", 1);
        }
        if let Some(previous) = last_wall_tick.replace(Some(tick_wall_started)) {
            crate::perf_trace::record_duration(
                "gtk_tick_wall_interval",
                tick_wall_started.duration_since(previous),
            );
        }
        let tick_started = frame_clock.frame_time();
        let elapsed_frame = last_frame_tick
            .replace(Some(tick_started))
            .and_then(|previous| {
                (tick_started >= previous)
                    .then(|| std::time::Duration::from_micros((tick_started - previous) as u64))
            });
        if let Some(elapsed_frame) = elapsed_frame {
            crate::perf_trace::record_duration("gtk_frame_interval", elapsed_frame);
        }
        record_frame_clock_diagnostics(
            tick_canvas.widget(),
            frame_clock,
            tick_started,
            &tick_last_completed_frame_timing,
            &tick_last_presentation_time,
        );
        if profile_frame_baseline || profile_timer_baseline || profile_updating_baseline {
            tick_canvas.widget().queue_draw();
            if profile_frame_baseline {
                frame_clock.request_phase(gtk::gdk::FrameClockPhase::UPDATE);
                frame_clock.request_phase(gtk::gdk::FrameClockPhase::PAINT);
            }
            record_tick_work(tick_wall_started);
            return glib::ControlFlow::Continue;
        }
        let measured_metrics = terminal_metrics_for_widget_cached(
            tick_canvas.widget(),
            &cached_terminal_metrics,
            tick_last_size.get().is_none(),
        );
        cell_metrics.set(measured_metrics.map(|metrics| metrics.cell));
        if let (Some(path), Some(metrics)) = (&geometry_trace, measured_metrics) {
            trace_geometry(path, tick_canvas.widget(), metrics);
        }
        if let Some(path) = &tab_trace {
            trace_tabs(
                path,
                &trace_window,
                &tab_bar,
                &launch_menu_button,
                &workspace_rc.borrow(),
            );
        }
        if let Some(size) = measured_metrics.map(|metrics| metrics.size)
            && tick_last_size.get() != Some(size)
        {
            let resize_result = if tick_last_size.get().is_none() && crate::host::is_flatpak() {
                workspace_rc.borrow_mut().respawn_active_tab_with_size(size)
            } else {
                workspace_rc.borrow_mut().resize_active_tab(size)
            };
            if resize_result.is_ok() {
                tick_last_size.set(Some(size));
            }
        }
        let cursor_tick_started = std::time::Instant::now();
        tick_canvas.tick_cursor_visual();
        crate::perf_trace::record_duration("gtk_cursor_tick", cursor_tick_started.elapsed());
        let scroll_tick_started = std::time::Instant::now();
        let scroll_changed = measured_metrics.is_some_and(|metrics| {
            advance_smooth_scroll_frame(
                &workspace_rc,
                &tick_canvas,
                metrics.cell,
                &smooth_scroll,
                crate::frame_timing::animation_frame_duration(elapsed_frame),
                scroll_trace.as_deref(),
            )
        });
        crate::perf_trace::record_duration("gtk_smooth_scroll_tick", scroll_tick_started.elapsed());
        if smooth_scroll.borrow().is_active() {
            frame_clock.request_phase(gtk::gdk::FrameClockPhase::UPDATE);
            frame_clock.request_phase(gtk::gdk::FrameClockPhase::PAINT);
        }
        let force = force_snapshot.replace(false) || scroll_changed;
        let selection_changed = selection_dirty.replace(false);
        let preedit_state = preedit.borrow().clone();
        let pane_count = workspace_rc.borrow().active_tab_pane_count();

        if pane_count > 1 {
            let terminal_changed =
                force || workspace_rc.borrow_mut().active_tab_has_dirty_renderable();
            if !terminal_changed
                && !selection_changed
                && preedit_state.is_none()
                && !scroll_changed
                && ui_e2e.is_none()
            {
                record_tick_work(tick_wall_started);
                return glib::ControlFlow::Continue;
            }
            let panes = workspace_rc.borrow_mut().snapshot_active_tab_renderables();
            let Some(active_content) = panes
                .iter()
                .find(|pane| pane.active)
                .map(|pane| pane.content.clone())
            else {
                record_tick_work(tick_wall_started);
                return glib::ControlFlow::Continue;
            };
            mouse_mode.set(active_content.mouse);
            *last_content.borrow_mut() = Some(active_content.clone());
            let expected_selection_text = selection_text.borrow().clone();
            let visible_selection = visible_or_reanchored_selection(
                &active_content,
                &selection,
                expected_selection_text.as_deref(),
            );
            if selection_text.borrow().is_none()
                && let Some(visible_selection) = visible_selection
            {
                *selection_text.borrow_mut() =
                    text_for_viewport_selection(&active_content, visible_selection);
            }
            let allocations_before = crate::allocation_trace::snapshot();
            let render_started = std::time::Instant::now();
            let layout = measured_metrics.map(|metrics| WorkspaceRenderLayout {
                cols: usize::from(metrics.size.cols),
                rows: usize::from(metrics.size.rows),
                pane_cols: workspace_rc
                    .borrow()
                    .active_tab_pane_columns(metrics.size.cols)
                    .into_iter()
                    .map(usize::from)
                    .collect(),
            });
            let snapshot_due = snapshot_enabled
                && (last_snapshot.borrow().elapsed() >= std::time::Duration::from_secs(1)
                    || selection_changed
                    || force);
            let rendered = if snapshot_due || render_snapshot_enabled {
                WorkspaceRenderFrame::from_active_tab_panes_with_layout(
                    panes,
                    visible_selection,
                    layout,
                )
            } else {
                WorkspaceRenderFrame::from_active_tab_panes_for_paint_with_layout(
                    panes,
                    visible_selection,
                    layout,
                )
            };
            let mut rendered = rendered;
            apply_preedit_to_workspace_render(&mut rendered, preedit_state.as_ref());
            crate::perf_trace::record_duration("gtk_render", render_started.elapsed());
            active_pane_origin_col.set(
                rendered
                    .panes
                    .iter()
                    .find(|pane| pane.active)
                    .map(|pane| pane.origin_col)
                    .unwrap_or(0),
            );
            *pane_hits.borrow_mut() = rendered
                .panes
                .iter()
                .map(|pane| PaneHit {
                    id: PaneId::from_raw(pane.pane_id),
                    origin_col: pane.origin_col,
                    cols: pane.cols,
                })
                .collect();
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
            if crate::perf_trace::enabled()
                && let Some(rss_kib) = crate::process_metrics::resident_set_kib()
            {
                crate::perf_trace::record_counter("process_rss_kib", rss_kib);
            }
            let e2e_text = ui_e2e.as_ref().map(|_| {
                rendered
                    .panes
                    .iter()
                    .flat_map(|pane| pane.frame.lines.iter())
                    .map(|line| line.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            });
            let rendered_snapshot = if snapshot_due || e2e_text.is_some() {
                Some(rendered.clone())
            } else {
                None
            };
            tick_canvas.set_workspace_render(rendered);
            if tick_pending_style_refresh.replace(false) {
                refresh_style(&tick_style_provider);
            }
            record_pending_input_latency(&pending_input_latency);
            if selection_changed {
                copy_selection_to_primary(tick_canvas.widget(), &selection_text);
            }
            if let (Some(scenario), Some(text)) = (&ui_e2e, e2e_text.as_deref()) {
                if scenario
                    .expected
                    .iter()
                    .all(|expected| text.contains(expected))
                {
                    gtk::test_widget_wait_for_draw(tick_canvas.widget());
                    let _ =
                        write_snapshot_with_selection(active_content, "gtk_e2e", visible_selection);
                    if let Some(rendered) = &rendered_snapshot {
                        let _ = write_workspace_render_snapshot(rendered, "gtk_e2e_workspace");
                    }
                    app_for_tick.quit();
                    record_tick_work(tick_wall_started);
                    return glib::ControlFlow::Break;
                }
                if ui_e2e_deadline.is_some_and(|deadline| std::time::Instant::now() > deadline) {
                    eprintln!("gtk e2e expected content did not appear: {text}");
                    std::process::exit(1);
                }
            }
            if snapshot_due {
                *last_snapshot.borrow_mut() = std::time::Instant::now();
                if let Some(rendered) = &rendered_snapshot {
                    let _ = write_workspace_render_snapshot(rendered, "workspace_frame");
                }
                if selection_text.borrow().is_none()
                    && let Some(visible_selection) = visible_selection
                {
                    *selection_text.borrow_mut() =
                        text_for_viewport_selection(&active_content, visible_selection);
                }
                let _ = write_snapshot_with_selection(active_content, "frame", visible_selection);
                if selection.get().is_some() {
                    copy_selection_to_primary(tick_canvas.widget(), &selection_text);
                }
            }
            record_tick_work(tick_wall_started);
            return glib::ControlFlow::Continue;
        }

        active_pane_origin_col.set(0);
        pane_hits.borrow_mut().clear();
        let terminal_content = if force {
            let snapshot_started = std::time::Instant::now();
            let content = workspace_rc.borrow_mut().snapshot_active_renderable();
            crate::perf_trace::record_duration("gtk_snapshot_forced", snapshot_started.elapsed());
            content
        } else {
            let snapshot_started = std::time::Instant::now();
            let content = workspace_rc
                .borrow_mut()
                .snapshot_active_renderable_if_dirty();
            crate::perf_trace::record_duration("gtk_snapshot_dirty", snapshot_started.elapsed());
            content
        };
        let terminal_changed = terminal_content.is_some();
        if let Some(content) = terminal_content {
            mouse_mode.set(content.mouse);
            *last_content.borrow_mut() = Some(content);
        }
        let render_content = if terminal_changed || selection_changed {
            last_content.borrow().clone()
        } else {
            None
        };
        if let Some(content) = render_content {
            let expected_selection_text = selection_text.borrow().clone();
            let visible_selection = visible_or_reanchored_selection(
                &content,
                &selection,
                expected_selection_text.as_deref(),
            );
            if selection_text.borrow().is_none()
                && let Some(visible_selection) = visible_selection
            {
                *selection_text.borrow_mut() =
                    text_for_viewport_selection(&content, visible_selection);
            }
            let allocations_before = crate::allocation_trace::snapshot();
            let render_started = std::time::Instant::now();
            let rendered = if render_snapshot_enabled {
                Renderer::render_frame_with_selection(&content, visible_selection)
            } else {
                Renderer::render_frame_for_paint(&content, visible_selection)
            };
            let mut rendered = rendered;
            apply_preedit_to_render_frame(&mut rendered, preedit_state.as_ref());
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
            if crate::perf_trace::enabled()
                && let Some(rss_kib) = crate::process_metrics::resident_set_kib()
            {
                crate::perf_trace::record_counter("process_rss_kib", rss_kib);
            }
            let e2e_render_snapshot = if ui_e2e.is_some() && render_snapshot_enabled {
                Some(rendered.clone())
            } else {
                None
            };
            tick_canvas.set_render(rendered);
            if tick_pending_style_refresh.replace(false) {
                refresh_style(&tick_style_provider);
            }
            record_pending_input_latency(&pending_input_latency);
            if selection_changed {
                copy_selection_to_primary(tick_canvas.widget(), &selection_text);
            }
            if let Some(scenario) = &ui_e2e {
                let text = lines_to_text(&content.lines);
                if scenario
                    .expected
                    .iter()
                    .all(|expected| text.contains(expected))
                {
                    gtk::test_widget_wait_for_draw(tick_canvas.widget());
                    if let Some(rendered) = &e2e_render_snapshot {
                        let _ = write_render_frame_snapshot(rendered, "gtk_e2e_render");
                    }
                    let _ = write_snapshot_with_selection(content, "gtk_e2e", visible_selection);
                    app_for_tick.quit();
                    record_tick_work(tick_wall_started);
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
                || selection_changed
                || force)
            && let Some(content) = last_content.borrow().clone()
        {
            *last_snapshot.borrow_mut() = std::time::Instant::now();
            crate::logging::debug_log(&format!("snapshot selection {:?}", selection.get()));
            let expected_selection_text = selection_text.borrow().clone();
            let visible_selection = visible_or_reanchored_selection(
                &content,
                &selection,
                expected_selection_text.as_deref(),
            );
            if selection_text.borrow().is_none()
                && let Some(visible_selection) = visible_selection
            {
                *selection_text.borrow_mut() =
                    text_for_viewport_selection(&content, visible_selection);
            }
            let rendered = Renderer::render_frame_with_selection(&content, visible_selection);
            let mut rendered = rendered;
            apply_preedit_to_render_frame(&mut rendered, preedit.borrow().as_ref());
            if render_snapshot_enabled {
                let _ = write_render_frame_snapshot(&rendered, "frame_render");
            }
            let _ = write_snapshot_with_selection(content, "frame", visible_selection);
            if selection.get().is_some() {
                copy_selection_to_primary(tick_canvas.widget(), &selection_text);
            }
        }
        record_tick_work(tick_wall_started);
        glib::ControlFlow::Continue
    });

    let open_startup_preferences = crate::config::take_first_launch_preferences()
        || std::env::var("CHELOTYPE_MEDIA_OPEN_PREFERENCES")
            .ok()
            .as_deref()
            == Some("1");

    window.present();

    if open_startup_preferences {
        let window = window.clone();
        let canvas = canvas.clone();
        let force_snapshot = media_preferences_force_snapshot.clone();
        let pending_style_refresh = pending_style_refresh.clone();
        let last_size = last_size.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(450), move || {
            show_preferences_dialog(
                &window,
                &canvas,
                force_snapshot,
                pending_style_refresh,
                last_size,
            );
        });
    }
}

fn configure_launch_menu(
    menu_button: &gtk::MenuButton,
    tabs: TabContext,
    last_size: std::rc::Rc<std::cell::Cell<Option<ScreenSize>>>,
) {
    let popover = gtk::Popover::new();
    popover.add_css_class("tab-menu");
    let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Filter...")
        .build();
    panel.append(&search);
    panel.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let entries = std::rc::Rc::new(std::cell::RefCell::new(Vec::<LaunchMenuEntry>::new()));
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.add_css_class("launch-targets");
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .child(&list)
        .build();
    panel.append(&scroller);
    popover.set_child(Some(&panel));
    menu_button.set_popover(Some(&popover));

    let targets = std::rc::Rc::new(available_launch_targets());
    rebuild_launch_list(
        &list,
        &entries,
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
        let entries = entries.clone();
        let targets = targets.clone();
        let context = LaunchMenuContext {
            tabs: tabs.clone(),
            last_size: last_size.clone(),
            popover: popover.clone(),
        };
        search.connect_search_changed(move |entry| {
            rebuild_launch_list(
                &list,
                &entries,
                entry.text().as_str(),
                targets.as_ref(),
                context.clone(),
            );
        });
    }
    {
        let context = LaunchMenuContext {
            tabs,
            last_size,
            popover,
        };
        let entries = entries.clone();
        search.connect_activate(move |_| {
            let Some(target) = entries.borrow().first().map(|entry| entry.target.clone()) else {
                return;
            };
            add_launch_target_tab(target, &context);
            context.popover.popdown();
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
    keyboard_selection: std::rc::Rc<std::cell::Cell<Option<DirectedSelectionRange>>>,
}

#[derive(Clone)]
struct LaunchMenuContext {
    tabs: TabContext,
    last_size: std::rc::Rc<std::cell::Cell<Option<ScreenSize>>>,
    popover: gtk::Popover,
}

#[derive(Clone)]
struct LaunchMenuEntry {
    title: String,
    target: LaunchTarget,
}

impl LaunchMenuEntry {
    fn is_container(&self) -> bool {
        !matches!(self.target, LaunchTarget::Host)
    }
}

#[derive(Default)]
struct TabPages {
    entries: Vec<TabPageEntry>,
}

struct TabPageEntry {
    id: TabId,
    page: adw::TabPage,
}

impl TabPages {
    fn push(&mut self, id: TabId, page: adw::TabPage) {
        self.entries.push(TabPageEntry { id, page });
    }

    fn id_for_page(&self, page: &adw::TabPage) -> Option<TabId> {
        self.entries
            .iter()
            .find(|entry| entry.page == *page)
            .map(|entry| entry.id)
    }

    fn page_for_id(&self, id: TabId) -> Option<adw::TabPage> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.page.clone())
    }

    fn remove_page(&mut self, page: &adw::TabPage) {
        self.entries.retain(|entry| entry.page != *page);
    }

    fn reorder_page(&mut self, page: &adw::TabPage, position: usize) {
        let Some(index) = self.entries.iter().position(|entry| entry.page == *page) else {
            return;
        };
        let position = position.min(self.entries.len().saturating_sub(1));
        let entry = self.entries.remove(index);
        self.entries.insert(position, entry);
    }

    fn id_at_position(&self, position: usize) -> Option<TabId> {
        self.entries.get(position).map(|entry| entry.id)
    }

    fn sync_title(&self, workspace: &TerminalWorkspace, id: TabId) {
        let Some(page) = self.page_for_id(id) else {
            return;
        };
        if let Some(title) = workspace.tab_title(id) {
            page.set_title(&title);
            page.set_tooltip(&title);
        }
    }
}

fn rebuild_launch_list(
    list: &gtk::Box,
    entries: &std::rc::Rc<std::cell::RefCell<Vec<LaunchMenuEntry>>>,
    filter: &str,
    targets: &[LaunchTarget],
    context: LaunchMenuContext,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let filter = filter.trim().to_lowercase();
    let mut next = Vec::new();
    for target in targets.iter().filter(|target| {
        filter.is_empty() || target.title().to_lowercase().contains(filter.as_str())
    }) {
        next.push(LaunchMenuEntry {
            title: target.title(),
            target: target.clone(),
        });
    }
    let mut previous_was_container = false;
    for entry in &next {
        if entry.is_container() && !previous_was_container {
            let header = gtk::Label::builder()
                .label("Containers")
                .halign(gtk::Align::Fill)
                .xalign(0.5)
                .build();
            header.add_css_class("title");
            header.add_css_class("dim-label");
            header.add_css_class("launch-targets-header");
            list.append(&header);
        }
        previous_was_container = entry.is_container();
        let button = gtk::Button::builder()
            .css_classes(["flat", "launch-target-row"])
            .focus_on_click(false)
            .build();
        let label = gtk::Label::builder()
            .label(entry.title.as_str())
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        content.append(&label);
        button.set_child(Some(&content));
        let target = entry.target.clone();
        let context = context.clone();
        button.connect_clicked(move |_| {
            add_launch_target_tab(target.clone(), &context);
            context.popover.popdown();
        });
        list.append(&button);
    }
    *entries.borrow_mut() = next;
}

fn add_launch_target_tab(target: LaunchTarget, context: &LaunchMenuContext) {
    let title = target.title();
    let Ok(id) = context
        .tabs
        .workspace
        .borrow_mut()
        .add_launch_target_tab_with_size(target.clone(), context.last_size.get())
    else {
        return;
    };
    let page = append_tab_page(&context.tabs.tab_view, id, &title);
    context.tabs.tab_pages.borrow_mut().push(id, page.clone());
    context.tabs.tab_view.set_selected_page(&page);
    activate_workspace_tab(&context.tabs, id, context.last_size.get());
}

fn remember_single_tab_launch_target(tabs: &TabContext) {
    let workspace = tabs.workspace.borrow();
    if let Some(target) = single_tab_launch_target_to_remember(&workspace) {
        crate::containers::remember_startup_launch_target(&target);
    }
}

fn single_tab_launch_target_to_remember(workspace: &TerminalWorkspace) -> Option<LaunchTarget> {
    if workspace.tab_count() != 1 {
        return None;
    }
    let target = workspace.tab_launch_target(workspace.active_tab_id())?;
    (!matches!(target, LaunchTarget::Host)).then_some(target)
}

fn add_existing_workspace_pages(
    tab_view: &adw::TabView,
    tab_pages: &std::rc::Rc<std::cell::RefCell<TabPages>>,
    workspace: &TerminalWorkspace,
) {
    for tab in workspace.tabs() {
        let title = workspace
            .tab_title(tab.id)
            .unwrap_or_else(|| format!("Terminal {}", tab.index + 1));
        let page = append_tab_page(tab_view, tab.id, &title);
        tab_pages.borrow_mut().push(tab.id, page.clone());
        if tab.active {
            tab_view.set_selected_page(&page);
        }
    }
}

fn append_tab_page(tab_view: &adw::TabView, id: TabId, title: &str) -> adw::TabPage {
    let child = gtk::Box::new(gtk::Orientation::Vertical, 0);
    child.set_widget_name(&format!("chelotype-tab-{:?}", id));
    let page = tab_view.append(&child);
    page.set_title(title);
    page.set_tooltip(title);
    page
}

fn select_page_for_tab(
    tab_view: &adw::TabView,
    tab_pages: &std::rc::Rc<std::cell::RefCell<TabPages>>,
    id: TabId,
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
    let menu_target = std::rc::Rc::new(std::cell::Cell::new(None::<TabId>));
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

    let middle_click = gtk::GestureClick::new();
    middle_click.set_button(2);
    {
        let tabs = tabs.clone();
        middle_click.connect_pressed(move |gesture, _press_count, x, _y| {
            let Some(widget) = gesture.widget() else {
                return;
            };
            let count = tabs.tab_pages.borrow().entries.len();
            if count <= 1 {
                return;
            }
            let width = widget.allocated_width().max(1) as f64;
            let position = ((x / width) * count as f64).floor() as usize;
            let Some(id) = tabs
                .tab_pages
                .borrow()
                .id_at_position(position.min(count - 1))
            else {
                return;
            };
            close_tab(&tabs, id);
        });
    }
    tab_bar.add_controller(middle_click);
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
            activate_workspace_tab(&tabs, id, last_size.get());
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
            tabs.tab_pages
                .borrow_mut()
                .reorder_page(page, position.max(0) as usize);
        });
    }
    {
        let tabs = tabs.clone();
        tab_view.connect_page_detached(move |_view, page, _position| {
            tabs.tab_pages.borrow_mut().remove_page(page);
            let active = tabs.workspace.borrow().active_tab_id();
            select_page_for_tab(&tabs.tab_view, &tabs.tab_pages, active);
        });
    }
    {
        let tabs = tabs.clone();
        tab_view.connect_close_page(move |_view, page| {
            let Some(id) = tabs.tab_pages.borrow().id_for_page(page) else {
                return glib::Propagation::Stop;
            };
            if tabs.workspace.borrow_mut().close(id) {
                remember_single_tab_launch_target(&tabs);
                force_active_workspace_snapshot(
                    &tabs.force_snapshot,
                    &tabs.selection,
                    &tabs.selection_text,
                    &tabs.selection_dirty,
                    &tabs.keyboard_selection,
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

fn active_tab_id(tabs: &TabContext) -> Option<TabId> {
    let page = tabs.tab_view.selected_page()?;
    tabs.tab_pages.borrow().id_for_page(&page)
}

#[derive(Clone, Copy)]
enum TabMoveDirection {
    Left,
    Right,
}

fn move_tab(tabs: &TabContext, id: TabId, direction: TabMoveDirection) {
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

fn close_tab(tabs: &TabContext, id: TabId) {
    let Some(page) = tabs.tab_pages.borrow().page_for_id(id) else {
        return;
    };
    tabs.tab_view.close_page(&page);
}

fn close_other_tabs(tabs: &TabContext, id: TabId, size: Option<ScreenSize>) {
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
    activate_workspace_tab(tabs, id, size);
}

fn open_rename_popover(id: TabId, parent: &gtk::Widget, tabs: &TabContext) {
    let popover = gtk::Popover::new();
    popover.set_parent(parent);
    let entry = gtk::Entry::builder()
        .text(
            tabs.workspace
                .borrow()
                .tab_title(id)
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

fn activate_workspace_tab(tabs: &TabContext, id: TabId, size: Option<ScreenSize>) {
    let activated = tabs.workspace.borrow_mut().activate(id);
    if activated {
        force_active_workspace_snapshot(
            &tabs.force_snapshot,
            &tabs.selection,
            &tabs.selection_text,
            &tabs.selection_dirty,
            &tabs.keyboard_selection,
            size,
            &tabs.workspace,
        );
    }
}

fn force_active_workspace_snapshot(
    force_snapshot: &std::rc::Rc<std::cell::Cell<bool>>,
    selection: &std::rc::Rc<std::cell::Cell<Option<SelectionRange>>>,
    selection_text: &std::rc::Rc<std::cell::RefCell<Option<String>>>,
    selection_dirty: &std::rc::Rc<std::cell::Cell<bool>>,
    keyboard_selection: &std::rc::Rc<std::cell::Cell<Option<DirectedSelectionRange>>>,
    size: Option<ScreenSize>,
    workspace: &std::rc::Rc<std::cell::RefCell<TerminalWorkspace>>,
) {
    if let Some(size) = size {
        let _ = workspace.borrow_mut().resize_active_tab(size);
    }
    clear_selection(
        selection,
        selection_text,
        selection_dirty,
        keyboard_selection,
    );
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
    copy_text_to_clipboard(widget, &text);
    true
}

fn copy_text_to_clipboard(widget: &gtk::DrawingArea, text: &str) {
    widget.clipboard().set_text(text);
    trace_clipboard_export("clipboard", text);
}

#[derive(Clone)]
struct PasteClipboardContext {
    workspace: std::rc::Rc<std::cell::RefCell<TerminalWorkspace>>,
    content: std::rc::Rc<std::cell::RefCell<Option<RenderableContentOwned>>>,
    selection: std::rc::Rc<std::cell::Cell<Option<SelectionRange>>>,
    selection_text: std::rc::Rc<std::cell::RefCell<Option<String>>>,
    selection_dirty: std::rc::Rc<std::cell::Cell<bool>>,
    keyboard_selection: std::rc::Rc<std::cell::Cell<Option<DirectedSelectionRange>>>,
    pending_input_latency: PendingInputLatency,
}

fn paste_clipboard_text(widget: &gtk::DrawingArea, context: PasteClipboardContext) {
    widget
        .clipboard()
        .read_text_async(None::<&gtk::gio::Cancellable>, move |result| {
            let Ok(Some(text)) = result else {
                return;
            };
            mark_pending_input_latency(&context.pending_input_latency);
            let shell_bridge_active = context
                .content
                .borrow()
                .as_ref()
                .is_some_and(shell_input_bridge_active);
            let bracketed_paste =
                shell_bridge_active || context.workspace.borrow_mut().active_bracketed_paste_mode();
            let data = terminal_paste_bytes(bracketed_paste, text.as_bytes());
            write_key_with_selection(
                &context.workspace,
                &context.content,
                &context.selection,
                &context.selection_text,
                &context.selection_dirty,
                &context.keyboard_selection,
                data,
            );
        });
}

fn terminal_paste_bytes(bracketed_paste: bool, text: &[u8]) -> Vec<u8> {
    const START: &[u8] = b"\x1b[200~";
    const END: &[u8] = b"\x1b[201~";

    if bracketed_paste && !text.windows(END.len()).any(|window| window == END) {
        let mut bytes = Vec::with_capacity(START.len() + text.len() + END.len());
        bytes.extend_from_slice(START);
        bytes.extend_from_slice(text);
        bytes.extend_from_slice(END);
        bytes
    } else {
        text.to_vec()
    }
}

#[derive(Clone)]
struct CanvasContextMenuContext {
    workspace: std::rc::Rc<std::cell::RefCell<TerminalWorkspace>>,
    content: std::rc::Rc<std::cell::RefCell<Option<RenderableContentOwned>>>,
    selection: std::rc::Rc<std::cell::Cell<Option<SelectionRange>>>,
    selection_text: std::rc::Rc<std::cell::RefCell<Option<String>>>,
    selection_dirty: std::rc::Rc<std::cell::Cell<bool>>,
    keyboard_selection: std::rc::Rc<std::cell::Cell<Option<DirectedSelectionRange>>>,
    pending_input_latency: PendingInputLatency,
    command_block_output: Option<String>,
}

fn show_canvas_context_menu(
    widget: &gtk::DrawingArea,
    x: f64,
    y: f64,
    context: CanvasContextMenuContext,
) {
    let menu_model = gtk::gio::Menu::new();
    if context.command_block_output.is_some() {
        menu_model.append(
            Some("Copy Block Output"),
            Some("terminal-menu.copy-block-output"),
        );
    }
    menu_model.append(Some("Copy"), Some("terminal-menu.copy"));
    menu_model.append(Some("Cut"), Some("terminal-menu.cut"));
    menu_model.append(Some("Paste"), Some("terminal-menu.paste"));
    menu_model.append(Some("Select Input"), Some("terminal-menu.select-input"));

    let popover = gtk::PopoverMenu::from_model(Some(&menu_model));
    popover.set_parent(widget);
    popover.set_has_arrow(false);
    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
        x.round() as i32,
        y.round() as i32,
        1,
        1,
    )));

    let actions = gtk::gio::SimpleActionGroup::new();

    let copy_block_output = gtk::gio::SimpleAction::new("copy-block-output", None);
    copy_block_output.set_enabled(context.command_block_output.is_some());
    {
        let widget = widget.clone();
        let output = context.command_block_output.clone();
        let popover = popover.clone();
        copy_block_output.connect_activate(move |_, _| {
            if let Some(output) = output.as_ref() {
                copy_text_to_clipboard(&widget, output);
            }
            popover.popdown();
        });
    }
    actions.add_action(&copy_block_output);

    let copy = gtk::gio::SimpleAction::new("copy", None);
    copy.set_enabled(context.selection_text.borrow().is_some());
    {
        let widget = widget.clone();
        let selection_text = context.selection_text.clone();
        let popover = popover.clone();
        copy.connect_activate(move |_, _| {
            copy_selection_to_clipboard(&widget, &selection_text);
            popover.popdown();
        });
    }
    actions.add_action(&copy);

    let cut = gtk::gio::SimpleAction::new("cut", None);
    cut.set_enabled(context.selection_text.borrow().is_some());
    {
        let widget = widget.clone();
        let workspace = context.workspace.clone();
        let content = context.content.clone();
        let selection = context.selection.clone();
        let selection_text = context.selection_text.clone();
        let selection_dirty = context.selection_dirty.clone();
        let keyboard_selection = context.keyboard_selection.clone();
        let pending_input_latency = context.pending_input_latency.clone();
        let popover = popover.clone();
        cut.connect_activate(move |_, _| {
            if copy_selection_to_clipboard(&widget, &selection_text) {
                mark_pending_input_latency(&pending_input_latency);
                write_key_with_selection(
                    &workspace,
                    &content,
                    &selection,
                    &selection_text,
                    &selection_dirty,
                    &keyboard_selection,
                    b"\x1b[3~".to_vec(),
                );
            }
            popover.popdown();
        });
    }
    actions.add_action(&cut);

    let paste = gtk::gio::SimpleAction::new("paste", None);
    {
        let widget = widget.clone();
        let context = PasteClipboardContext {
            workspace: context.workspace.clone(),
            content: context.content.clone(),
            selection: context.selection.clone(),
            selection_text: context.selection_text.clone(),
            selection_dirty: context.selection_dirty.clone(),
            keyboard_selection: context.keyboard_selection.clone(),
            pending_input_latency: context.pending_input_latency.clone(),
        };
        let popover = popover.clone();
        paste.connect_activate(move |_, _| {
            paste_clipboard_text(&widget, context.clone());
            popover.popdown();
        });
    }
    actions.add_action(&paste);

    let select_input = gtk::gio::SimpleAction::new("select-input", None);
    {
        let content = context.content.clone();
        let selection = context.selection.clone();
        let selection_text = context.selection_text.clone();
        let selection_dirty = context.selection_dirty.clone();
        let keyboard_selection = context.keyboard_selection.clone();
        let popover = popover.clone();
        select_input.connect_activate(move |_, _| {
            select_active_input(
                &content,
                &selection,
                &selection_text,
                &selection_dirty,
                &keyboard_selection,
            );
            popover.popdown();
        });
    }
    actions.add_action(&select_input);

    widget.insert_action_group("terminal-menu", Some(&actions));
    popover.popup();
}

fn install_app_accelerators(app: &Application) {
    app.set_accels_for_action("win.new-window", &["<Control><Shift>n"]);
    app.set_accels_for_action("win.preferences", &["<Control>comma"]);
    app.set_accels_for_action("win.about", &["F1"]);
}

fn main_menu_model() -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append(Some("New Window"), Some("win.new-window"));
    menu.append(Some("Preferences"), Some("win.preferences"));
    menu.append(Some("About"), Some("win.about"));
    menu
}

fn install_window_actions(
    window: &adw::ApplicationWindow,
    app: &Application,
    canvas: TerminalCanvas,
    workspace: std::rc::Rc<std::cell::RefCell<TerminalWorkspace>>,
    force_snapshot: std::rc::Rc<std::cell::Cell<bool>>,
    pending_style_refresh: std::rc::Rc<std::cell::Cell<bool>>,
    last_size: std::rc::Rc<std::cell::Cell<Option<ScreenSize>>>,
) {
    let new_window = gio::SimpleAction::new("new-window", None);
    {
        let app = app.clone();
        new_window.connect_activate(move |_, _| build_ui(&app));
    }
    window.add_action(&new_window);

    let preferences = gio::SimpleAction::new("preferences", None);
    {
        let window = window.clone();
        let canvas = canvas.clone();
        let force_snapshot = force_snapshot.clone();
        let pending_style_refresh = pending_style_refresh.clone();
        let last_size = last_size.clone();
        preferences.connect_activate(move |_, _| {
            show_preferences_dialog(
                &window,
                &canvas,
                force_snapshot.clone(),
                pending_style_refresh.clone(),
                last_size.clone(),
            );
        });
    }
    window.add_action(&preferences);

    let about = gio::SimpleAction::new("about", None);
    {
        let window = window.clone();
        about.connect_activate(move |_, _| {
            show_about_dialog(&window, workspace.borrow().tabs().len());
        });
    }
    window.add_action(&about);
}

fn show_about_dialog(parent: &adw::ApplicationWindow, tab_count: usize) {
    let debug_info = debug_info(parent, tab_count);
    let dialog = adw::AboutDialog::builder()
        .application_icon(APP_ID)
        .application_name(APP_NAME)
        .developer_name("chelokot")
        .version(env!("CARGO_PKG_VERSION"))
        .copyright("Copyright 2026 Andrii Vlasenko")
        .developers(["Andrii Vlasenko"])
        .license_type(gtk::License::Custom)
        .license("MIT OR Apache-2.0")
        .website(GITHUB_REPO_URL)
        .issue_url(issue_url(&debug_info))
        .release_notes_version(env!("CARGO_PKG_VERSION"))
        .release_notes(release_notes())
        .build();
    dialog.present(Some(parent));
}

fn issue_url(debug_info: &str) -> String {
    let body = format!(
        "### Describe the issue\n\n\n\n### Steps to reproduce\n\n1. \n\n### Debug information\n\n```text\n{}\n```",
        debug_info
    );
    format!(
        "{GITHUB_REPO_URL}/issues/new?body={}",
        url_query_value(&body)
    )
}

fn release_notes() -> &'static str {
    "<p>Adds Snappy cursor motion, improves multiline input editing, and expands Neovide cursor tuning.</p>
    <p>Earlier releases:</p>
    <ul>
      <li>0.1.14: Cursor preference spacing and preview-card layout geometry tests.</li>
      <li>0.1.13: Wrapped input selection, mouse drag cursor following, and shell autosuggestion cursor handoff.</li>
      <li>0.1.12: Polished preference preview cards, scrolling previews, and palette layout.</li>
      <li>0.1.11: Curated palette picker with alphabetical expanded view and search.</li>
      <li>0.1.10: Polished What's New release history layout.</li>
      <li>0.1.9: AppStream-compatible About dialog changelog markup.</li>
      <li>0.1.8: Full release history in About.</li>
      <li>0.1.7: Main menu, About dialog, issue reporting metadata, and release notes.</li>
      <li>0.1.6: Palette transitions across the terminal, window chrome, and preferences UI.</li>
      <li>0.1.5: Host shell terminal width propagation for Flatpak launches.</li>
      <li>0.1.4: Compact default terminal font size for Flatpak installs.</li>
      <li>0.1.3: Flatpak repository publishing on GitHub Actions user remotes.</li>
      <li>0.1.2: Signed Flatpak repository from the rebased release source.</li>
      <li>0.1.1: Signed self-hosted Flatpak repository with GitHub Pages publishing.</li>
      <li>0.1.0: Initial Flathub-ready release.</li>
    </ul>"
}

fn debug_info(window: &adw::ApplicationWindow, tab_count: usize) -> String {
    let display = gtk::prelude::WidgetExt::display(window);
    let display_type = display.type_().name();
    let renderer_type = window
        .renderer()
        .map(|renderer| renderer.type_().name().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let monitor = window
        .surface()
        .and_then(|surface| display.monitor_at_surface(&surface));
    let monitor_info = monitor
        .map(|monitor| {
            let geometry = monitor.geometry();
            let scale_factor =
                <gtk::gdk::Monitor as gtk::gdk::prelude::MonitorExt>::scale_factor(&monitor);
            format!(
                "window[0].scale = {:.6}\nwindow[0].scale_factor = {}\nwindow[0].monitor.geometry = {},{} {}x{}\nwindow[0].monitor.refresh_rate = {}",
                f64::from(scale_factor),
                scale_factor,
                geometry.x(),
                geometry.y(),
                geometry.width(),
                geometry.height(),
                <gtk::gdk::Monitor as gtk::gdk::prelude::MonitorExt>::refresh_rate(&monitor)
            )
        })
        .unwrap_or_else(|| {
            format!(
                "window[0].scale_factor = {}\nwindow[0].monitor = unknown",
                window.scale_factor()
            )
        });

    format!(
        "{APP_NAME} {version} ({version})\n\nOperating System: {os}\n\n{uname}\n\nAgent: {agent}\n\nDesktop Session: {session}\n\nGTK: {gtk_version}\n\nDisplay: {display_type}\nAccessibility: {accessibility}\n\nGTK Theme: {gtk_theme}\nSystem Font: {system_font}\nFont: {font}\n\nwindow[0].n_tabs = {tab_count}\nwindow[0].renderer = {renderer_type}\n{monitor_info}\n\nApp ID: {APP_ID}\n\nContainers:\n{containers}\n\n{os_release}",
        version = env!("CARGO_PKG_VERSION"),
        os = glib::os_info("PRETTY_NAME").unwrap_or_else(|| "unknown".into()),
        uname = uname_info(),
        agent = agent_info(),
        session = environment_value("XDG_CURRENT_DESKTOP")
            .or_else(|| environment_value("DESKTOP_SESSION"))
            .unwrap_or_else(|| "unknown".to_string()),
        gtk_version = gtk_version(),
        accessibility = "unknown",
        gtk_theme = gtk_setting("gtk-theme-name"),
        system_font = gtk_setting("gtk-font-name"),
        font = terminal_font_info(),
        containers = containers_info(),
        os_release = os_release_info(),
    )
}

fn gtk_version() -> String {
    format!(
        "{}.{}.{}",
        gtk::major_version(),
        gtk::minor_version(),
        gtk::micro_version()
    )
}

fn uname_info() -> String {
    let Ok(output) = std::process::Command::new("uname").arg("-smrv").output() else {
        return "uname: unavailable".to_string();
    };
    if !output.status.success() {
        return "uname: unavailable".to_string();
    }
    let output = String::from_utf8_lossy(&output.stdout);
    format!("uname = {}", output.trim())
}

fn agent_info() -> String {
    if std::env::var_os("FLATPAK_ID").is_some() {
        "running in Flatpak".to_string()
    } else if let Some(container) = environment_value("container") {
        format!("running in {container}")
    } else {
        "running on host".to_string()
    }
}

fn gtk_setting(name: &str) -> String {
    gtk::Settings::default()
        .map(|settings| settings.property::<String>(name))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn terminal_font_info() -> String {
    environment_value("CHELOTYPE_FONT_SIZE")
        .map(|size| format!("system font scaled to {size}px"))
        .unwrap_or_else(|| "-- Using System Font --".to_string())
}

fn containers_info() -> String {
    let targets = available_launch_targets()
        .into_iter()
        .filter(|target| !matches!(target, LaunchTarget::Host))
        .map(|target| format!("  - {}", target.id()))
        .collect::<Vec<_>>();
    if targets.is_empty() {
        "  none".to_string()
    } else {
        targets.join("\n")
    }
}

fn os_release_info() -> String {
    std::fs::read_to_string("/etc/os-release")
        .map(|text| text.trim().to_string())
        .unwrap_or_else(|_| "/etc/os-release unavailable".to_string())
}

fn environment_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn url_query_value(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn show_preferences_dialog(
    parent: &adw::ApplicationWindow,
    canvas: &TerminalCanvas,
    force_snapshot: std::rc::Rc<std::cell::Cell<bool>>,
    pending_style_refresh: std::rc::Rc<std::cell::Cell<bool>>,
    last_size: std::rc::Rc<std::cell::Cell<Option<ScreenSize>>>,
) {
    let preference_geometry_trace = environment_value("CHELOTYPE_PREFERENCES_GEOMETRY_TRACE");
    let default_height = std::env::var("CHELOTYPE_MEDIA_PREFERENCES_HEIGHT")
        .ok()
        .and_then(|height| height.parse::<i32>().ok())
        .unwrap_or(960);
    let window = adw::Window::builder()
        .title("Preferences")
        .default_width(960)
        .default_height(default_height)
        .transient_for(parent)
        .modal(true)
        .build();
    let content = adw::ToolbarView::new();
    let header = adw::HeaderBar::builder().build();
    header.add_css_class("terminal-header");
    let stack = adw::ViewStack::builder()
        .hexpand(true)
        .vexpand(true)
        .build();
    let cursor_page = cursor_preferences_page(canvas);
    let appearance_page = appearance_preferences_page(
        parent,
        canvas,
        force_snapshot,
        pending_style_refresh,
        last_size,
    );
    stack.add_titled_with_icon(
        &cursor_page,
        Some("cursor"),
        "Cursor",
        "input-keyboard-symbolic",
    );
    stack.add_titled_with_icon(
        &appearance_page,
        Some("appearance"),
        "Appearance",
        "applications-graphics-symbolic",
    );
    stack.set_visible_child_name("cursor");
    if preference_geometry_trace.is_some()
        && environment_value("CHELOTYPE_PREFERENCES_GEOMETRY_PAGE").as_deref() != Some("cursor")
    {
        stack.set_visible_child_name("appearance");
    }

    let header_switcher = adw::ViewSwitcher::builder()
        .stack(&stack)
        .policy(adw::ViewSwitcherPolicy::Wide)
        .build();
    let header_title = adw::WindowTitle::builder().title("Preferences").build();
    let header_title_stack = gtk::Stack::new();
    header_title_stack.add_named(&header_switcher, Some("switcher"));
    header_title_stack.add_named(&header_title, Some("title"));
    header_title_stack.set_visible_child_name("switcher");
    header.set_title_widget(Some(&header_title_stack));
    content.add_top_bar(&header);
    content.set_content(Some(&stack));

    let switcher_bar = adw::ViewSwitcherBar::builder()
        .stack(&stack)
        .reveal(false)
        .build();
    content.add_bottom_bar(&switcher_bar);

    let adaptive_root = adw::BreakpointBin::builder()
        .child(&content)
        .width_request(360)
        .height_request(360)
        .build();
    let narrow_navigation = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        700.0,
        adw::LengthUnit::Sp,
    ));
    narrow_navigation.add_setter(
        &header_title_stack,
        "visible-child-name",
        Some(&"title".to_value()),
    );
    narrow_navigation.add_setter(&switcher_bar, "reveal", Some(&true.to_value()));
    adaptive_root.add_breakpoint(narrow_navigation);

    window.set_content(Some(&adaptive_root));
    window.present();
    if let Some(trace_path) = preference_geometry_trace {
        let window = window.clone();
        let application = parent.application();
        glib::timeout_add_local_once(std::time::Duration::from_millis(350), move || {
            write_preferences_geometry_trace(&window, &trace_path);
            if std::env::var("CHELOTYPE_PREFERENCES_GEOMETRY_EXIT")
                .ok()
                .as_deref()
                == Some("1")
                && let Some(application) = application
            {
                application.quit();
            }
        });
    }
}

fn write_preferences_geometry_trace(window: &adw::Window, path: &str) {
    let root = window.upcast_ref::<gtk::Widget>();
    let mut output = String::new();
    for (name, widget_name) in [
        ("card", "chelotype-scrolling-instant-card"),
        ("terminal", "chelotype-scrolling-instant-preview"),
        ("label", "chelotype-scrolling-instant-label"),
        ("animation_speed", "chelotype-animation-speed-row"),
        ("advanced", "chelotype-advanced-animation-settings"),
        ("cursor_blinking", "chelotype-cursor-blinking-row"),
    ] {
        let Some(widget) = find_named_widget(root, widget_name) else {
            output.push_str(&format!("{name}=missing\n"));
            continue;
        };
        let Some(bounds) = widget.compute_bounds(root) else {
            output.push_str(&format!("{name}=unallocated\n"));
            continue;
        };
        output.push_str(&format!(
            "{name}.x_min={:.3}\n{name}.x_max={:.3}\n{name}.y_min={:.3}\n{name}.y_max={:.3}\n{name}.width={:.3}\n{name}.height={:.3}\n",
            bounds.x(),
            bounds.x() + bounds.width(),
            bounds.y(),
            bounds.y() + bounds.height(),
            bounds.width(),
            bounds.height(),
        ));
    }
    let _ = std::fs::write(path, output);
}

fn find_named_widget(root: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    if root.widget_name().as_str() == name {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Some(found) = find_named_widget(&widget, name) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn cursor_preferences_page(canvas: &TerminalCanvas) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder().title("General").build();
    let group = adw::PreferencesGroup::builder()
        .title("Cursor animation")
        .build();
    let cursor_shape_grid = gtk::Grid::builder()
        .column_spacing(10)
        .margin_top(8)
        .margin_bottom(8)
        .build();
    let cursor_animation_grid = gtk::Grid::builder()
        .column_spacing(10)
        .row_spacing(10)
        .margin_top(8)
        .margin_bottom(8)
        .build();
    let animation_settings_group = AnimationSettingsGroup::new();
    populate_cursor_shape_grid(
        &cursor_shape_grid,
        canvas.clone(),
        cursor_animation_grid.clone(),
        animation_settings_group.clone(),
    );

    let shape_group = adw::PreferencesGroup::builder()
        .title("Cursor shape")
        .build();
    shape_group.add(&cursor_shape_grid);
    page.add(&shape_group);

    group.add(&cursor_animation_grid);
    populate_cursor_animation_grid(
        &cursor_animation_grid,
        crate::config::cursor_shape(),
        canvas.clone(),
        animation_settings_group.clone(),
    );
    populate_animation_settings(
        &animation_settings_group,
        crate::config::cursor_style(),
        crate::config::cursor_shape(),
        canvas.clone(),
    );
    let cursor_options = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .build();
    let cursor_blinking_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    cursor_blinking_list.append(&cursor_blinking_row(canvas.clone()));
    cursor_options.append(&animation_settings_group.container);
    cursor_options.append(&cursor_blinking_list);
    group.add(&cursor_options);
    page.add(&group);
    page
}

fn appearance_preferences_page(
    parent: &adw::ApplicationWindow,
    canvas: &TerminalCanvas,
    force_snapshot: std::rc::Rc<std::cell::Cell<bool>>,
    pending_style_refresh: std::rc::Rc<std::cell::Cell<bool>>,
    last_size: std::rc::Rc<std::cell::Cell<Option<ScreenSize>>>,
) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder().title("Appearance").build();
    let font_group = adw::PreferencesGroup::builder().title("Font").build();
    let (system_font_row, custom_font_row) = font_rows(parent, canvas.clone());
    font_group.add(&system_font_row);
    font_group.add(&custom_font_row);
    page.add(&font_group);

    let group = adw::PreferencesGroup::builder().title("Scrolling").build();
    let grid = gtk::Grid::builder()
        .column_spacing(12)
        .margin_top(8)
        .margin_bottom(8)
        .build();
    let off_tile = scrolling_preview_tile(false, canvas.clone());
    let smooth_tile = scrolling_preview_tile(true, canvas.clone());
    smooth_tile.set_group(Some(&off_tile));
    off_tile.set_active(!crate::config::smooth_scrolling_enabled());
    smooth_tile.set_active(crate::config::smooth_scrolling_enabled());
    grid.attach(&off_tile, 0, 0, 1, 1);
    grid.attach(&smooth_tile, 1, 0, 1, 1);
    group.add(&grid);
    page.add(&group);

    let palette_group = palette_preferences_group(
        canvas.clone(),
        force_snapshot.clone(),
        pending_style_refresh,
    );
    page.add(&palette_group);
    page.add(&spacing_preferences_group(
        canvas.clone(),
        force_snapshot,
        last_size,
    ));
    page
}

fn palette_preferences_group(
    canvas: TerminalCanvas,
    force_snapshot: std::rc::Rc<std::cell::Cell<bool>>,
    pending_style_refresh: std::rc::Rc<std::cell::Cell<bool>>,
) -> adw::PreferencesGroup {
    let show_all = std::rc::Rc::new(std::cell::Cell::new(false));
    let selected_palette = std::rc::Rc::new(std::cell::RefCell::new(
        crate::terminal_palette::default_terminal_palette()
            .id
            .to_string(),
    ));
    let previews = std::rc::Rc::new(std::cell::RefCell::new(Vec::<gtk::DrawingArea>::new()));
    let cards = std::rc::Rc::new(std::cell::RefCell::new(Vec::<PaletteCard>::new()));
    let toggle = gtk::Button::new();
    set_pointer_cursor(&toggle);
    set_palette_visibility_toggle(&toggle, show_all.get());
    let group = adw::PreferencesGroup::builder().build();
    let container = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .build();
    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .build();
    let title = gtk::Label::builder()
        .label("Palette")
        .halign(gtk::Align::Start)
        .hexpand(true)
        .css_classes(["heading"])
        .build();
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search palettes")
        .hexpand(true)
        .visible(false)
        .build();
    header.append(&title);
    header.append(&search);
    header.append(&toggle);
    container.append(&header);
    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .min_children_per_line(3)
        .max_children_per_line(3)
        .row_spacing(12)
        .column_spacing(12)
        .margin_top(8)
        .margin_bottom(8)
        .build();
    let sort_all_palettes = std::rc::Rc::new(std::cell::Cell::new(false));
    for palette in crate::terminal_palette::terminal_palette_display_order(true) {
        let card = palette_preview_card(
            palette,
            selected_palette.clone(),
            previews.clone(),
            canvas.clone(),
            force_snapshot.clone(),
            pending_style_refresh.clone(),
        );
        let flow_child = gtk::FlowBoxChild::builder()
            .child(&card)
            .visible(palette.primary)
            .css_classes(["palette-flow-child"])
            .build();
        flow.append(&flow_child);
        cards.borrow_mut().push(PaletteCard {
            child: flow_child,
            palette,
        });
    }
    {
        let sort_all_palettes = sort_all_palettes.clone();
        let cards = cards.clone();
        flow.set_sort_func(move |first, second| {
            let cards = cards.borrow();
            let first_palette = palette_for_flow_child(&cards, first);
            let second_palette = palette_for_flow_child(&cards, second);
            palette_card_order(first_palette, second_palette, sort_all_palettes.get())
        });
    }
    flow.invalidate_sort();
    {
        let show_all = show_all.clone();
        let cards = cards.clone();
        let flow = flow.clone();
        let sort_all_palettes = sort_all_palettes.clone();
        let search = search.clone();
        let title = title.clone();
        toggle.connect_clicked(move |button| {
            let expanded = !show_all.get();
            show_all.set(expanded);
            set_palette_visibility_toggle(button, expanded);
            sort_all_palettes.set(expanded);
            flow.invalidate_sort();
            title.set_visible(!expanded);
            search.set_visible(expanded);
            if expanded {
                search.grab_focus();
            } else {
                search.set_text("");
            }
            update_palette_card_visibility(&cards.borrow(), expanded, search.text().as_str());
        });
    }
    {
        let show_all = show_all.clone();
        let cards = cards.clone();
        search.connect_search_changed(move |entry| {
            update_palette_card_visibility(&cards.borrow(), show_all.get(), entry.text().as_str());
        });
    }
    container.append(&flow);
    group.add(&container);
    group
}

fn spacing_preferences_group(
    canvas: TerminalCanvas,
    force_snapshot: std::rc::Rc<std::cell::Cell<bool>>,
    last_size: std::rc::Rc<std::cell::Cell<Option<ScreenSize>>>,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().build();
    group.add(&terminal_spacing_row(
        "Line Spacing",
        crate::config::line_spacing(),
        canvas.clone(),
        force_snapshot.clone(),
        last_size.clone(),
        crate::config::set_line_spacing,
    ));
    group.add(&terminal_spacing_row(
        "Column Spacing",
        crate::config::column_spacing(),
        canvas,
        force_snapshot,
        last_size,
        crate::config::set_column_spacing,
    ));
    group
}

fn terminal_spacing_row(
    title: &str,
    value: f64,
    canvas: TerminalCanvas,
    force_snapshot: std::rc::Rc<std::cell::Cell<bool>>,
    last_size: std::rc::Rc<std::cell::Cell<Option<ScreenSize>>>,
    write_value: fn(f64),
) -> adw::SpinRow {
    let adjustment = gtk::Adjustment::new(value, 0.5, 2.0, 0.1, 0.1, 0.0);
    let row = adw::SpinRow::builder()
        .title(title)
        .adjustment(&adjustment)
        .digits(1)
        .numeric(true)
        .snap_to_ticks(true)
        .build();
    row.connect_value_notify(move |row| {
        write_value(row.value());
        last_size.set(None);
        force_snapshot.set(true);
        canvas.widget().queue_draw();
    });
    row
}

#[derive(Clone)]
struct PaletteCard {
    child: gtk::FlowBoxChild,
    palette: &'static crate::terminal_palette::TerminalPalette,
}

fn palette_for_flow_child(
    cards: &[PaletteCard],
    child: &gtk::FlowBoxChild,
) -> &'static crate::terminal_palette::TerminalPalette {
    cards
        .iter()
        .find(|card| card.child == *child)
        .map(|card| card.palette)
        .expect("palette card")
}

fn palette_card_order(
    first: &crate::terminal_palette::TerminalPalette,
    second: &crate::terminal_palette::TerminalPalette,
    show_all: bool,
) -> gtk::Ordering {
    let first_index = crate::terminal_palette::terminal_palette_display_index(first.id, show_all);
    let second_index = crate::terminal_palette::terminal_palette_display_index(second.id, show_all);
    match first_index.cmp(&second_index) {
        std::cmp::Ordering::Less => gtk::Ordering::Smaller,
        std::cmp::Ordering::Equal => gtk::Ordering::Equal,
        std::cmp::Ordering::Greater => gtk::Ordering::Larger,
    }
}

fn update_palette_card_visibility(cards: &[PaletteCard], show_all: bool, query: &str) {
    let query = query.trim().to_lowercase();
    for card in cards {
        let matches_query = query.is_empty() || card.palette.name.to_lowercase().contains(&query);
        card.child
            .set_visible((show_all || card.palette.primary) && matches_query);
    }
}

fn set_palette_visibility_toggle(button: &gtk::Button, show_all: bool) {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();
    let icon = gtk::Image::builder()
        .icon_name(if show_all {
            "pan-up-symbolic"
        } else {
            "pan-down-symbolic"
        })
        .pixel_size(14)
        .build();
    let label = gtk::Label::builder()
        .label(if show_all {
            "Show Fewer Palettes"
        } else {
            "Show All Palettes"
        })
        .build();
    content.append(&icon);
    content.append(&label);
    button.set_child(Some(&content));
}

fn palette_preview_card(
    palette: &'static crate::terminal_palette::TerminalPalette,
    selected_palette: std::rc::Rc<std::cell::RefCell<String>>,
    previews: std::rc::Rc<std::cell::RefCell<Vec<gtk::DrawingArea>>>,
    canvas: TerminalCanvas,
    force_snapshot: std::rc::Rc<std::cell::Cell<bool>>,
    pending_style_refresh: std::rc::Rc<std::cell::Cell<bool>>,
) -> gtk::Button {
    let preview = gtk::DrawingArea::builder()
        .width_request(188)
        .height_request(154)
        .build();
    preview.set_can_target(false);
    {
        let selected_palette = selected_palette.clone();
        preview.set_draw_func(move |_, context, width, height| {
            draw_palette_preview(
                context,
                PalettePreview {
                    palette,
                    selected: selected_palette.borrow().as_str() == palette.id,
                    width: f64::from(width),
                    height: f64::from(height),
                },
            );
        });
    }
    previews.borrow_mut().push(preview.clone());
    let button = gtk::Button::builder()
        .css_classes(["palette-card-button"])
        .tooltip_text(palette.name)
        .build();
    button.update_property(&[gtk::accessible::Property::Label(palette.name)]);
    set_pointer_cursor(&button);
    button.set_child(Some(&preview));
    button.connect_clicked(move |_| {
        let changed = selected_palette.borrow().as_str() != palette.id;
        if changed {
            canvas.begin_palette_transition();
            crate::terminal_palette::set_default_terminal_palette(palette.id);
            pending_style_refresh.set(true);
        }
        *selected_palette.borrow_mut() = palette.id.to_string();
        for preview in previews.borrow().iter() {
            preview.queue_draw();
        }
        if changed {
            force_snapshot.set(true);
            canvas.widget().queue_draw();
        }
    });
    button
}

struct PalettePreview {
    palette: &'static crate::terminal_palette::TerminalPalette,
    selected: bool,
    width: f64,
    height: f64,
}

fn draw_palette_preview(context: &gtk::cairo::Context, preview: PalettePreview) {
    rounded_rectangle(
        context,
        0.5,
        0.5,
        preview.width - 1.0,
        preview.height - 1.0,
        9.0,
    );
    set_source_hex(context, preview.palette.background);
    let _ = context.fill_preserve();
    if preview.selected {
        context.set_source_rgb(53.0 / 255.0, 132.0 / 255.0, 228.0 / 255.0);
        context.set_line_width(2.0);
        let _ = context.stroke();
    } else {
        context.new_path();
    }

    let foreground = crate::terminal_palette::TerminalRgb::from_hex(preview.palette.foreground);
    context.select_font_face(
        "sans",
        gtk::cairo::FontSlant::Normal,
        gtk::cairo::FontWeight::Bold,
    );
    context.set_font_size(18.0);
    set_source_rgb_alpha(context, foreground, 1.0);
    context.move_to(12.0, 28.0);
    let _ = context.show_text(&ellipsize_palette_title(preview.palette.name));

    context.select_font_face(
        "monospace",
        gtk::cairo::FontSlant::Normal,
        gtk::cairo::FontWeight::Normal,
    );
    context.set_font_size(16.0);
    set_source_rgb_alpha(context, foreground, 0.82);
    for (index, text) in ["The quick brown", "fox jumps over", "the lazy dog"]
        .iter()
        .enumerate()
    {
        context.move_to(12.0, 62.0 + index as f64 * 21.0);
        let _ = context.show_text(text);
    }

    let swatch_width = 24.0;
    let swatch_height = 16.0;
    let swatch_gap = 4.0;
    let swatch_y = preview.height - 28.0;
    for (index, color) in preview.palette.indexed[1..=6].iter().enumerate() {
        set_source_hex(context, color);
        rounded_rectangle(
            context,
            12.0 + index as f64 * (swatch_width + swatch_gap),
            swatch_y,
            swatch_width,
            swatch_height,
            4.0,
        );
        let _ = context.fill();
    }

    if preview.selected {
        draw_palette_check(context, preview.width - 20.0, 20.0);
    }
}

fn ellipsize_palette_title(title: &str) -> String {
    const MAX_COLUMNS: usize = 14;
    if title.chars().count() <= MAX_COLUMNS {
        return title.to_string();
    }
    let mut value = title.chars().take(MAX_COLUMNS).collect::<String>();
    value.push_str("...");
    value
}

fn draw_palette_check(context: &gtk::cairo::Context, x: f64, y: f64) {
    context.arc(x, y, 9.0, 0.0, std::f64::consts::TAU);
    context.set_source_rgb(53.0 / 255.0, 132.0 / 255.0, 228.0 / 255.0);
    let _ = context.fill();
    context.set_source_rgb(1.0, 1.0, 1.0);
    context.set_line_width(2.0);
    context.move_to(x - 4.0, y);
    context.line_to(x - 1.0, y + 3.0);
    context.line_to(x + 5.0, y - 5.0);
    let _ = context.stroke();
}

fn rounded_rectangle(
    context: &gtk::cairo::Context,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    radius: f64,
) {
    let right = x + width;
    let bottom = y + height;
    context.new_sub_path();
    context.arc(
        right - radius,
        y + radius,
        radius,
        -std::f64::consts::FRAC_PI_2,
        0.0,
    );
    context.arc(
        right - radius,
        bottom - radius,
        radius,
        0.0,
        std::f64::consts::FRAC_PI_2,
    );
    context.arc(
        x + radius,
        bottom - radius,
        radius,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    context.arc(
        x + radius,
        y + radius,
        radius,
        std::f64::consts::PI,
        std::f64::consts::PI * 1.5,
    );
    context.close_path();
}

fn set_source_hex(context: &gtk::cairo::Context, color: &str) {
    set_source_rgb_alpha(
        context,
        crate::terminal_palette::TerminalRgb::from_hex(color),
        1.0,
    );
}

fn set_source_rgb_alpha(
    context: &gtk::cairo::Context,
    color: crate::terminal_palette::TerminalRgb,
    alpha: f64,
) {
    context.set_source_rgba(
        color.red_unit(),
        color.green_unit(),
        color.blue_unit(),
        alpha,
    );
}

fn font_rows(
    parent: &adw::ApplicationWindow,
    canvas: TerminalCanvas,
) -> (adw::ActionRow, adw::ActionRow) {
    let system_font_switch = gtk::Switch::builder()
        .active(crate::terminal_font::use_system_font())
        .valign(gtk::Align::Center)
        .build();
    set_pointer_cursor(&system_font_switch);
    let system_font_row = adw::ActionRow::builder()
        .title("Use System Font")
        .activatable_widget(&system_font_switch)
        .build();
    system_font_row.add_suffix(&system_font_switch);

    let custom_font_row = adw::ActionRow::builder()
        .title("Custom Font")
        .activatable(true)
        .build();
    let custom_font_label = gtk::Label::builder()
        .label(crate::terminal_font::custom_font_label())
        .halign(gtk::Align::End)
        .build();
    custom_font_label.set_can_target(false);
    let custom_font_arrow = gtk::Image::builder()
        .icon_name("go-next-symbolic")
        .pixel_size(16)
        .build();
    custom_font_arrow.set_can_target(false);
    custom_font_row.add_suffix(&custom_font_label);
    custom_font_row.add_suffix(&custom_font_arrow);
    custom_font_row.set_visible(!crate::terminal_font::use_system_font());

    {
        let canvas = canvas.clone();
        let custom_font_row = custom_font_row.clone();
        system_font_switch.connect_active_notify(move |switch| {
            let use_system_font = switch.is_active();
            crate::terminal_font::set_use_system_font(use_system_font);
            custom_font_row.set_visible(!use_system_font);
            canvas.widget().queue_draw();
        });
    }
    {
        let parent = parent.clone();
        let canvas = canvas.clone();
        custom_font_row.connect_activated(move |_| {
            show_custom_font_dialog(&parent, canvas.clone(), custom_font_label.clone());
        });
    }

    (system_font_row, custom_font_row)
}

fn show_custom_font_dialog(
    parent: &adw::ApplicationWindow,
    canvas: TerminalCanvas,
    custom_font_label: gtk::Label,
) {
    let dialog = gtk::FontChooserDialog::new(Some("Custom Font"), Some(parent));
    dialog.set_modal(true);
    dialog.set_font(&crate::terminal_font::custom_font());
    dialog.add_button("Cancel", gtk::ResponseType::Cancel);
    dialog.add_button("Select", gtk::ResponseType::Accept);
    dialog.set_default_response(gtk::ResponseType::Accept);
    dialog.connect_response(move |dialog, response| {
        if response == gtk::ResponseType::Accept
            && let Some(font) = dialog.font()
        {
            crate::terminal_font::set_custom_font(font.as_str());
            custom_font_label.set_label(&crate::terminal_font::custom_font_label());
            canvas.widget().queue_draw();
        }
        dialog.close();
    });
    dialog.present();
}

fn cursor_blinking_row(canvas: TerminalCanvas) -> adw::ComboRow {
    let labels = crate::config::CursorBlinking::ALL.map(crate::config::CursorBlinking::label);
    let model = gtk::StringList::new(&labels);
    let row = adw::ComboRow::builder()
        .title("Cursor Blinking")
        .model(&model)
        .selected(crate::config::cursor_blinking().selected_index())
        .build();
    row.set_widget_name("chelotype-cursor-blinking-row");
    row.connect_selected_notify(move |row| {
        let mode = crate::config::CursorBlinking::from_selected_index(row.selected());
        crate::config::write_value("cursor_blinking", mode.config_value());
        canvas.refresh_cursor_options();
    });
    row
}

fn set_pointer_cursor(widget: &impl IsA<gtk::Widget>) {
    widget.set_cursor_from_name(Some("pointer"));
}

fn populate_cursor_shape_grid(
    grid: &gtk::Grid,
    canvas: TerminalCanvas,
    cursor_animation_grid: gtk::Grid,
    animation_settings: AnimationSettingsGroup,
) {
    while let Some(child) = grid.first_child() {
        grid.remove(&child);
    }
    let mut first_button = None::<gtk::ToggleButton>;
    for (index, shape) in crate::config::CursorShape::ALL.iter().copied().enumerate() {
        let button = cursor_shape_tile(shape);
        if let Some(first_button) = &first_button {
            button.set_group(Some(first_button));
        } else {
            first_button = Some(button.clone());
        }
        button.set_active(shape == crate::config::cursor_shape());
        {
            let canvas = canvas.clone();
            let cursor_animation_grid = cursor_animation_grid.clone();
            let animation_settings = animation_settings.clone();
            button.connect_toggled(move |button| {
                if !button.is_active() {
                    return;
                }
                crate::config::write_value("cursor_shape", shape.config_value());
                populate_cursor_animation_grid(
                    &cursor_animation_grid,
                    shape,
                    canvas.clone(),
                    animation_settings.clone(),
                );
                populate_animation_settings(
                    &animation_settings,
                    crate::config::cursor_style(),
                    shape,
                    canvas.clone(),
                );
                canvas.refresh_cursor_options();
            });
        }
        grid.attach(&button, index as i32, 0, 1, 1);
    }
}

fn populate_cursor_animation_grid(
    grid: &gtk::Grid,
    shape: crate::config::CursorShape,
    canvas: TerminalCanvas,
    animation_settings: AnimationSettingsGroup,
) {
    while let Some(child) = grid.first_child() {
        grid.remove(&child);
    }
    let mut first_button = None::<gtk::ToggleButton>;
    for (index, style) in crate::config::CursorStyle::ALL.iter().copied().enumerate() {
        let button = cursor_animation_tile(shape, style);
        if let Some(first_button) = &first_button {
            button.set_group(Some(first_button));
        } else {
            first_button = Some(button.clone());
        }
        button.set_active(style == crate::config::cursor_style());
        {
            let canvas = canvas.clone();
            let animation_settings = animation_settings.clone();
            button.connect_toggled(move |button| {
                if !button.is_active() {
                    return;
                }
                crate::config::write_value("cursor_style", style.config_value());
                crate::config::write_value(
                    "cursor_animation",
                    if style == crate::config::CursorStyle::Steady {
                        "off"
                    } else {
                        "on"
                    },
                );
                populate_animation_settings(&animation_settings, style, shape, canvas.clone());
                canvas.refresh_cursor_options();
            });
        }
        grid.attach(&button, (index % 2) as i32, (index / 2) as i32, 1, 1);
    }
}

fn cursor_shape_tile(shape: crate::config::CursorShape) -> gtk::ToggleButton {
    preference_preview_tile(shape.label(), &cursor_shape_preview(shape))
}

fn preference_preview_tile(label: &str, preview: &impl IsA<gtk::Widget>) -> gtk::ToggleButton {
    let button = gtk::ToggleButton::builder()
        .css_classes(["preview-card-button"])
        .hexpand(true)
        .halign(gtk::Align::Fill)
        .vexpand(false)
        .build();
    set_pointer_cursor(&button);
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .hexpand(true)
        .halign(gtk::Align::Fill)
        .margin_top(16)
        .margin_bottom(8)
        .margin_start(16)
        .margin_end(16)
        .build();
    content.set_can_target(false);
    preview.set_can_target(false);
    content.append(preview);
    let label = gtk::Label::builder()
        .label(label)
        .halign(gtk::Align::Center)
        .build();
    label.set_can_target(false);
    content.append(&label);
    button.set_child(Some(&content));
    button
}

fn cursor_animation_tile(
    shape: crate::config::CursorShape,
    style: crate::config::CursorStyle,
) -> gtk::ToggleButton {
    preference_preview_tile(style.label(), &cursor_animation_preview(shape, style))
}

fn cursor_animation_preview(
    shape: crate::config::CursorShape,
    style: crate::config::CursorStyle,
) -> gtk::DrawingArea {
    let preview = terminal_preview_canvas(290, 169, style, shape, 6.5);
    preview.set_render(preview_render_frame(
        ANIMATION_PREVIEW_LINES,
        animation_preview_target(0),
    ));
    add_terminal_preview_tick(preview.clone(), Some(ANIMATION_PREVIEW_LINES));
    preview.widget().clone()
}

#[derive(Clone, Copy)]
struct PreviewCursorTarget {
    line: i32,
    column: i32,
}

const ANIMATION_PREVIEW_LINES: &[&str] = &[
    "$ cargo run",
    "  Compiling chelotype v0.1.0",
    "  Finished dev profile",
    "",
    "$ echo smooth cursor previews",
    "smooth cursor previews",
    "",
    "$ git status --short",
    " M src/canvas.rs",
];

const ANIMATION_PREVIEW_TARGETS: [PreviewCursorTarget; 5] = [
    PreviewCursorTarget { line: 0, column: 2 },
    PreviewCursorTarget {
        line: 0,
        column: 11,
    },
    PreviewCursorTarget {
        line: 4,
        column: 28,
    },
    PreviewCursorTarget {
        line: 7,
        column: 15,
    },
    PreviewCursorTarget { line: 8, column: 2 },
];

fn terminal_preview_canvas(
    width: i32,
    height: i32,
    style: crate::config::CursorStyle,
    shape: crate::config::CursorShape,
    font_size_pt: f64,
) -> TerminalCanvas {
    let preview = TerminalCanvas::new();
    preview.widget().set_width_request(width);
    preview.widget().set_height_request(height);
    preview.widget().set_hexpand(true);
    preview.widget().set_halign(gtk::Align::Fill);
    preview.widget().set_can_target(false);
    preview.widget().set_focusable(false);
    preview.widget().set_cursor_from_name(None);
    preview.widget().set_overflow(gtk::Overflow::Hidden);
    preview.add_preview_corners();
    preview.set_cursor_options_override(Some((style, shape)));
    preview.set_font_size_override(Some(font_size_pt));
    preview
}

fn add_terminal_preview_tick(
    preview: TerminalCanvas,
    animated_lines: Option<&'static [&'static str]>,
) {
    let started_at = std::rc::Rc::new(std::cell::Cell::new(None::<i64>));
    let widget = preview.widget().clone();
    widget.add_tick_callback(move |_, frame_clock| {
        if let Some(lines) = animated_lines {
            let now = frame_clock.frame_time();
            let start = started_at.get().unwrap_or(now);
            started_at.set(Some(start));
            preview.set_render(preview_render_frame(
                lines,
                animation_preview_target((now - start) as u64),
            ));
        }
        preview.tick_cursor_visual();
        gtk::glib::ControlFlow::Continue
    });
}

fn animation_preview_target(elapsed_us: u64) -> PreviewCursorTarget {
    let segment = (elapsed_us / 850_000) as usize;
    ANIMATION_PREVIEW_TARGETS[segment % ANIMATION_PREVIEW_TARGETS.len()]
}

fn preview_render_frame(lines: &[&str], cursor: PreviewCursorTarget) -> RenderFrame {
    let content = RenderableContentOwned {
        lines: lines.iter().map(|line| preview_cells(line)).collect(),
        line_metadata: vec![crate::terminal_grid::TerminalLineMetadata::default(); lines.len()],
        cursor_line: cursor.line,
        cursor_col: cursor.column,
        cursor_visible: true,
        display_offset: 0,
        colors: crate::terminal_grid::TerminalColors::default(),
        mouse: MouseMode::default(),
    };
    Renderer::render_frame_for_paint(&content, None)
}

fn preview_cells(line: &str) -> Vec<crate::terminal_grid::TerminalCell> {
    let palette = crate::terminal_palette::default_terminal_palette();
    let mut cells = Vec::new();
    let mut color = palette.foreground;
    let mut bold = false;
    for segment in line.split_inclusive(' ') {
        let trimmed = segment.trim_end();
        if trimmed == "$" || trimmed == "❯" {
            color = palette.indexed[3];
            bold = false;
        } else if trimmed == "Compiling" || trimmed == "Finished" {
            color = palette.indexed[2];
            bold = true;
        } else if trimmed.starts_with("smooth") {
            color = palette.indexed[12];
            bold = false;
        } else if trimmed == "M" {
            color = palette.indexed[1];
            bold = true;
        } else if trimmed.starts_with("src/") || trimmed.starts_with("v0.") {
            color = palette.indexed[8];
            bold = false;
        } else if !trimmed.is_empty() {
            color = palette.foreground;
            bold = false;
        }
        cells.extend(
            segment
                .chars()
                .map(|ch| crate::terminal_grid::TerminalCell {
                    text: ch.to_string().into(),
                    fg: Some(color.to_string()),
                    bold,
                    ..crate::terminal_grid::TerminalCell::blank()
                }),
        );
    }
    cells
}

fn scrolling_preview_tile(enabled: bool, canvas: TerminalCanvas) -> gtk::ToggleButton {
    let preview = scrolling_preview_panel(enabled);
    let button = preference_preview_tile(if enabled { "Smooth" } else { "Instant" }, &preview);
    if !enabled {
        button.set_widget_name("chelotype-scrolling-instant-card");
        preview.set_widget_name("chelotype-scrolling-instant-preview");
        if let Some(content) = button.child()
            && let Some(label) = content.last_child()
        {
            label.set_widget_name("chelotype-scrolling-instant-label");
        }
    }
    button.connect_toggled(move |button| {
        if !button.is_active() {
            return;
        }
        crate::config::write_value("smooth_scrolling", if enabled { "on" } else { "off" });
        canvas.widget().queue_draw();
    });
    button
}

fn scrolling_preview_panel(smooth: bool) -> gtk::DrawingArea {
    let state = std::rc::Rc::new(std::cell::RefCell::new(ScrollingPreviewState::new()));
    let preview = gtk::DrawingArea::builder()
        .width_request(290)
        .height_request(169)
        .hexpand(true)
        .halign(gtk::Align::Fill)
        .build();
    {
        let state = state.clone();
        preview.set_draw_func(move |_, context, width, height| {
            let state = state.borrow();
            draw_scrolling_preview_panel(
                context,
                PreviewScrollPanel {
                    x: 0.0,
                    y: 0.0,
                    width: f64::from(width),
                    height: f64::from(height),
                    scroll_lines: if smooth {
                        state.smooth_lines()
                    } else {
                        state.direct_lines()
                    },
                },
            );
        });
    }
    {
        let state = state.clone();
        let last_tick = std::rc::Rc::new(std::cell::Cell::new(None::<i64>));
        preview.add_tick_callback(move |preview, frame_clock| {
            let now = frame_clock.frame_time();
            let elapsed = last_tick
                .replace(Some(now))
                .and_then(|previous| {
                    (now >= previous)
                        .then(|| std::time::Duration::from_micros((now - previous) as u64))
                })
                .unwrap_or(crate::frame_timing::TARGET_FRAME_DURATION);
            state.borrow_mut().advance(elapsed);
            preview.queue_draw();
            gtk::glib::ControlFlow::Continue
        });
    }
    preview
}

struct PreviewScrollPanel {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    scroll_lines: f64,
}

fn draw_scrolling_preview_panel(context: &gtk::cairo::Context, panel: PreviewScrollPanel) {
    let line_height = 17.0;
    let cell_width = 8.0;
    let terminal_padding_y = 8.0;
    let available_terminal_height = panel.height.max(line_height);
    let visible_rows = ((available_terminal_height - terminal_padding_y * 2.0) / line_height)
        .floor()
        .max(1.0);
    let visible_height = visible_rows * line_height + terminal_padding_y * 2.0;
    let background = crate::terminal_palette::default_terminal_palette().background_rgb();
    let _ = context.save();
    rounded_rectangle(context, panel.x, panel.y, panel.width, panel.height, 6.0);
    context.clip();
    context.set_source_rgb(
        background.red_unit(),
        background.green_unit(),
        background.blue_unit(),
    );
    context.rectangle(panel.x, panel.y, panel.width, panel.height);
    let _ = context.fill();

    let max_scroll_lines = (PREVIEW_SCROLL_LINES.len() as f64 - visible_rows).max(0.0);
    let scroll_lines = panel.scroll_lines.clamp(0.0, max_scroll_lines);
    let scroll_offset = scroll_lines * line_height;
    let _ = context.save();
    context.rectangle(panel.x, panel.y, panel.width, visible_height);
    context.clip();
    context.translate(panel.x + 12.0, panel.y + terminal_padding_y - scroll_offset);
    context.select_font_face(
        "monospace",
        gtk::cairo::FontSlant::Normal,
        gtk::cairo::FontWeight::Normal,
    );
    context.set_font_size(13.0);
    for (index, line) in PREVIEW_SCROLL_LINES.iter().enumerate() {
        let baseline = (index as f64 + 0.78) * line_height;
        let visible_baseline = baseline - scroll_offset;
        if visible_baseline < 0.0 || visible_baseline > visible_height - terminal_padding_y {
            continue;
        }
        match *line {
            PreviewScrollLine::Prompt(text) => {
                context.set_source_rgb(181.0 / 255.0, 189.0 / 255.0, 104.0 / 255.0);
                context.move_to(0.0, baseline);
                let _ = context.show_text("$");
                context.set_source_rgb(218.0 / 255.0, 225.0 / 255.0, 232.0 / 255.0);
                context.move_to(cell_width * 2.0, baseline);
                let _ = context.show_text(text);
            }
            PreviewScrollLine::Output(text) => {
                context.set_source_rgb(125.0 / 255.0, 211.0 / 255.0, 252.0 / 255.0);
                context.move_to(0.0, baseline);
                let _ = context.show_text(text);
            }
            PreviewScrollLine::Dim(text) => {
                context.set_source_rgb(148.0 / 255.0, 163.0 / 255.0, 184.0 / 255.0);
                context.move_to(0.0, baseline);
                let _ = context.show_text(text);
            }
        }
    }
    let _ = context.restore();
    let _ = context.restore();
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScrollingPreviewPhase {
    InitialPause,
    Forward,
    BottomPause,
    Reverse,
    TopPause,
}

struct ScrollingPreviewState {
    profile: Vec<WheelProfileEvent>,
    phase: ScrollingPreviewPhase,
    phase_elapsed_ms: f64,
    event_index: usize,
    direct_lines: f64,
    smooth_base_lines: f64,
    smooth_offset_lines: f64,
    smooth_scroll: crate::smooth_scroll::SmoothScroll,
}

impl ScrollingPreviewState {
    fn new() -> Self {
        Self::with_profile(preview_wheel_profile_events())
    }

    fn with_profile(profile: Vec<WheelProfileEvent>) -> Self {
        Self {
            profile,
            phase: ScrollingPreviewPhase::InitialPause,
            phase_elapsed_ms: 0.0,
            event_index: 0,
            direct_lines: 0.0,
            smooth_base_lines: 0.0,
            smooth_offset_lines: 0.0,
            smooth_scroll: crate::smooth_scroll::SmoothScroll::default(),
        }
    }

    fn advance(&mut self, elapsed: std::time::Duration) {
        let frame_duration = crate::frame_timing::animation_frame_duration(Some(elapsed));
        self.phase_elapsed_ms += frame_duration.as_secs_f64() * 1000.0;
        match self.phase {
            ScrollingPreviewPhase::InitialPause => {
                if self.phase_elapsed_ms >= SCROLLING_PREVIEW_INITIAL_PAUSE_MS {
                    self.phase = ScrollingPreviewPhase::Forward;
                    self.phase_elapsed_ms = 0.0;
                    self.event_index = 0;
                }
            }
            ScrollingPreviewPhase::Forward => {
                while self.event_index < self.profile.len()
                    && self.profile[self.event_index].elapsed_ms <= self.phase_elapsed_ms
                {
                    let lines = self.profile[self.event_index].lines;
                    self.direct_lines += lines;
                    self.smooth_scroll.enqueue_pixels(lines);
                    self.event_index += 1;
                }
                if self.event_index == self.profile.len() {
                    self.phase = ScrollingPreviewPhase::BottomPause;
                    self.phase_elapsed_ms = 0.0;
                    self.event_index = 0;
                }
            }
            ScrollingPreviewPhase::BottomPause => {
                if self.phase_elapsed_ms >= SCROLLING_PREVIEW_PAUSE_MS
                    && !self.smooth_scroll.is_active()
                {
                    self.phase = ScrollingPreviewPhase::Reverse;
                    self.phase_elapsed_ms = 0.0;
                    self.event_index = 0;
                }
            }
            ScrollingPreviewPhase::Reverse => {
                while self.event_index < self.profile.len()
                    && self.profile[self.event_index].elapsed_ms <= self.phase_elapsed_ms
                {
                    let lines = self.profile[self.event_index].lines;
                    self.direct_lines = (self.direct_lines - lines).max(0.0);
                    self.smooth_scroll.enqueue_pixels(-lines);
                    self.event_index += 1;
                }
                if self.event_index == self.profile.len() {
                    self.phase = ScrollingPreviewPhase::TopPause;
                    self.phase_elapsed_ms = 0.0;
                    self.event_index = 0;
                }
            }
            ScrollingPreviewPhase::TopPause => {
                if self.phase_elapsed_ms >= SCROLLING_PREVIEW_PAUSE_MS
                    && !self.smooth_scroll.is_active()
                {
                    self.profile = preview_wheel_profile_events();
                    self.phase = ScrollingPreviewPhase::Forward;
                    self.phase_elapsed_ms = 0.0;
                    self.event_index = 0;
                    self.direct_lines = 0.0;
                    self.smooth_base_lines = 0.0;
                    self.smooth_offset_lines = 0.0;
                    self.smooth_scroll.cancel();
                }
            }
        }
        if let Some(frame) = self.smooth_scroll.advance(frame_duration, 1.0) {
            self.smooth_base_lines += f64::from(frame.line_delta);
            self.smooth_offset_lines = frame.offset_px;
            if !self.smooth_scroll.is_active() {
                self.smooth_offset_lines = 0.0;
            }
        }
    }

    fn direct_lines(&self) -> f64 {
        self.direct_lines
    }

    fn smooth_lines(&self) -> f64 {
        (self.smooth_base_lines + self.smooth_offset_lines).max(0.0)
    }
}

const SCROLLING_PREVIEW_INITIAL_PAUSE_MS: f64 = 500.0;
const SCROLLING_PREVIEW_PAUSE_MS: f64 = 700.0;

fn preview_wheel_profile_events() -> Vec<WheelProfileEvent> {
    let events = wheel_profile_events()
        .into_iter()
        .map(|event| WheelProfileEvent {
            elapsed_ms: event.elapsed_ms,
            lines: event.lines.abs(),
        })
        .filter(|event| event.lines != 0.0)
        .collect();
    normalize_preview_wheel_profile(events)
}

fn normalize_preview_wheel_profile(events: Vec<WheelProfileEvent>) -> Vec<WheelProfileEvent> {
    let total_lines = events.iter().map(|event| event.lines).sum::<f64>();
    if total_lines == 0.0 {
        return events;
    }
    let target_lines = total_lines.round().max(1.0);
    let scale = target_lines / total_lines;
    events
        .into_iter()
        .map(|event| WheelProfileEvent {
            elapsed_ms: event.elapsed_ms,
            lines: event.lines * scale,
        })
        .collect()
}

fn wheel_profile_events() -> Vec<WheelProfileEvent> {
    static PROFILE: std::sync::OnceLock<std::sync::Mutex<WheelProfileCache>> =
        std::sync::OnceLock::new();
    let path = crate::config::wheel_profile_path();
    let modified = path
        .as_ref()
        .and_then(|path| std::fs::metadata(path).ok())
        .and_then(|metadata| metadata.modified().ok());
    let cache = PROFILE.get_or_init(|| {
        std::sync::Mutex::new(WheelProfileCache {
            path: None,
            modified: None,
            events: default_wheel_profile(),
        })
    });
    let mut cache = cache.lock().expect("wheel profile cache lock");
    if cache.path == path && cache.modified == modified {
        return cache.events.clone();
    }
    let events = path
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|content| parse_wheel_profile(&content))
        .filter(|events| !events.is_empty())
        .unwrap_or_else(default_wheel_profile);
    cache.path = path;
    cache.modified = modified;
    cache.events = events.clone();
    events
}

fn parse_wheel_profile(content: &str) -> Vec<WheelProfileEvent> {
    content
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .filter_map(|line| {
            let (elapsed_ms, lines) = line.split_once('\t')?;
            Some(WheelProfileEvent {
                elapsed_ms: elapsed_ms.parse::<f64>().ok()?,
                lines: lines.parse::<f64>().ok()?,
            })
        })
        .filter(|event| event.elapsed_ms >= 0.0 && event.lines != 0.0)
        .take(128)
        .collect()
}

fn default_wheel_profile() -> Vec<WheelProfileEvent> {
    vec![
        WheelProfileEvent {
            elapsed_ms: 0.0,
            lines: 3.0,
        },
        WheelProfileEvent {
            elapsed_ms: 18.0,
            lines: 3.0,
        },
        WheelProfileEvent {
            elapsed_ms: 43.0,
            lines: 3.0,
        },
        WheelProfileEvent {
            elapsed_ms: 76.0,
            lines: 3.0,
        },
    ]
}

#[derive(Clone, Copy)]
struct WheelProfileEvent {
    elapsed_ms: f64,
    lines: f64,
}

struct WheelProfileCache {
    path: Option<std::path::PathBuf>,
    modified: Option<std::time::SystemTime>,
    events: Vec<WheelProfileEvent>,
}

enum PreviewScrollLine {
    Prompt(&'static str),
    Output(&'static str),
    Dim(&'static str),
}

const PREVIEW_SCROLL_LINES: &[PreviewScrollLine] = &[
    PreviewScrollLine::Prompt("git log --oneline"),
    PreviewScrollLine::Output("a31c7e9 tune smooth scroll"),
    PreviewScrollLine::Output("91e02aa render cursor previews"),
    PreviewScrollLine::Output("6bd7a13 add settings panel"),
    PreviewScrollLine::Output("5ed4f20 profile wheel timing"),
    PreviewScrollLine::Output("472da8b remove fake mp4 previews"),
    PreviewScrollLine::Output("21c9aae clamp scroll limits"),
    PreviewScrollLine::Dim(""),
    PreviewScrollLine::Prompt("cargo test smooth_scroll"),
    PreviewScrollLine::Output("running 17 tests"),
    PreviewScrollLine::Output("test first_active_frame ... ok"),
    PreviewScrollLine::Output("test accelerates_backlog ... ok"),
    PreviewScrollLine::Output("test clamps_limits ... ok"),
    PreviewScrollLine::Output("test scroll_burst ... ok"),
    PreviewScrollLine::Output("test direct_scroll_config ... ok"),
    PreviewScrollLine::Dim(""),
    PreviewScrollLine::Prompt("scripts/record-wheel-profile.sh"),
    PreviewScrollLine::Output("Recording wheel profile."),
    PreviewScrollLine::Output("axis_value120 120 -> 3.0000 lines"),
    PreviewScrollLine::Output("axis_value120 120 -> 3.0000 lines"),
    PreviewScrollLine::Output("axis_value120 120 -> 3.0000 lines"),
    PreviewScrollLine::Output("Saved 18 events over 1042ms"),
    PreviewScrollLine::Dim(""),
    PreviewScrollLine::Prompt("cargo run"),
    PreviewScrollLine::Output("Compiling chelotype v0.1.0"),
    PreviewScrollLine::Output("Finished dev profile"),
    PreviewScrollLine::Output("Launching chelotype"),
    PreviewScrollLine::Output("Loaded wheel-profile.tsv"),
    PreviewScrollLine::Output("Using smooth scroll integrator"),
    PreviewScrollLine::Dim(""),
    PreviewScrollLine::Prompt("git status --short"),
    PreviewScrollLine::Output(" M src/app.rs"),
    PreviewScrollLine::Output(" M src/smooth_scroll.rs"),
    PreviewScrollLine::Output("?? scripts/record-wheel-profile.sh"),
    PreviewScrollLine::Dim(""),
    PreviewScrollLine::Prompt("tail -f perf.log"),
    PreviewScrollLine::Output("gtk_frame_interval p50 4166us"),
    PreviewScrollLine::Output("gtk_paint p95 380us"),
    PreviewScrollLine::Output("scroll frame offset stable"),
    PreviewScrollLine::Dim(""),
    PreviewScrollLine::Prompt("journalctl --user -f"),
    PreviewScrollLine::Output("chelotype[4211]: frame clock tick"),
    PreviewScrollLine::Output("chelotype[4211]: redraw dirty rows"),
    PreviewScrollLine::Output("chelotype[4211]: cursor visual stable"),
    PreviewScrollLine::Output("chelotype[4211]: smooth target updated"),
    PreviewScrollLine::Output("chelotype[4211]: scrollback viewport 15"),
    PreviewScrollLine::Output("chelotype[4211]: scrollback viewport 21"),
    PreviewScrollLine::Dim(""),
    PreviewScrollLine::Prompt("rg -n smooth_scroll src"),
    PreviewScrollLine::Output("src/app.rs: enqueue wheel pixels"),
    PreviewScrollLine::Output("src/app.rs: advance animation frame"),
    PreviewScrollLine::Output("src/canvas.rs: draw visual offset"),
    PreviewScrollLine::Output("src/smooth_scroll.rs: clamp target"),
    PreviewScrollLine::Output("src/smooth_scroll.rs: apply line delta"),
    PreviewScrollLine::Output("src/smooth_scroll.rs: settle subpixel"),
    PreviewScrollLine::Dim(""),
    PreviewScrollLine::Prompt("git diff --stat"),
    PreviewScrollLine::Output("src/app.rs              | 120 +++++"),
    PreviewScrollLine::Output("src/smooth_scroll.rs    | 210 ++++++++"),
    PreviewScrollLine::Output("src/canvas.rs           |  52 ++"),
    PreviewScrollLine::Output("tests/gtk_e2e_tests.rs  |  88 ++++"),
    PreviewScrollLine::Output("scripts/profile-240hz.sh| 160 +++++++"),
    PreviewScrollLine::Dim(""),
    PreviewScrollLine::Prompt("cargo test --test gtk_e2e_tests"),
    PreviewScrollLine::Output("running 62 tests"),
    PreviewScrollLine::Output("gtk_e2e_opens_settings ... ok"),
    PreviewScrollLine::Output("gtk_e2e_smooth_scroll ... ok"),
    PreviewScrollLine::Output("gtk_e2e_cursor_preview ... ok"),
    PreviewScrollLine::Output("gtk_e2e_split_panes ... ok"),
    PreviewScrollLine::Output("test result: ok"),
    PreviewScrollLine::Dim(""),
    PreviewScrollLine::Prompt("scripts/profile-240hz.sh --release"),
    PreviewScrollLine::Output("scenario: scroll-burst"),
    PreviewScrollLine::Output("backend: weston-headless"),
    PreviewScrollLine::Output("gtk_render p95 57us"),
    PreviewScrollLine::Output("gtk_paint p95 383us"),
    PreviewScrollLine::Output("scroll_frames count 30"),
    PreviewScrollLine::Output("scroll_burst enqueues 8"),
    PreviewScrollLine::Dim(""),
    PreviewScrollLine::Prompt("printf '%s\\n' \"$WAYLAND_DISPLAY\""),
    PreviewScrollLine::Output("wayland-0"),
    PreviewScrollLine::Dim(""),
    PreviewScrollLine::Prompt("tail -n 12 wheel-profile.tsv"),
    PreviewScrollLine::Output("0      3.0000"),
    PreviewScrollLine::Output("24     3.0000"),
    PreviewScrollLine::Output("57     3.0000"),
    PreviewScrollLine::Output("93     3.0000"),
    PreviewScrollLine::Output("128    3.0000"),
    PreviewScrollLine::Output("181    3.0000"),
    PreviewScrollLine::Output("264    3.0000"),
    PreviewScrollLine::Output("351    3.0000"),
    PreviewScrollLine::Dim(""),
    PreviewScrollLine::Prompt("top -p $(pidof chelotype)"),
    PreviewScrollLine::Output("PID USER      PR  NI    VIRT    RES"),
    PreviewScrollLine::Output("4211 chelokot 20   0  812.3m  72.1m"),
    PreviewScrollLine::Output("CPU%  MEM%  TIME+ COMMAND"),
    PreviewScrollLine::Output(" 2.1   0.4  0:03.42 chelotype"),
    PreviewScrollLine::Dim(""),
    PreviewScrollLine::Prompt("echo ready"),
    PreviewScrollLine::Output("ready"),
];

fn cursor_shape_preview(shape: crate::config::CursorShape) -> gtk::DrawingArea {
    let preview = terminal_preview_canvas(290, 46, crate::config::CursorStyle::Steady, shape, 10.0);
    preview.set_render(preview_render_frame(
        &["let cursor = shape"],
        PreviewCursorTarget {
            line: 0,
            column: 11,
        },
    ));
    add_terminal_preview_tick(preview.clone(), None);
    preview.widget().clone()
}

#[derive(Clone)]
struct AnimationSettingsGroup {
    container: gtk::Box,
}

impl AnimationSettingsGroup {
    fn new() -> Self {
        Self {
            container: gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(12)
                .margin_top(4)
                .margin_bottom(12)
                .build(),
        }
    }

    fn clear(&self) {
        while let Some(child) = self.container.first_child() {
            self.container.remove(&child);
        }
    }

    fn add(&self, row: &impl IsA<gtk::Widget>) {
        self.container.append(row);
    }
}

fn populate_animation_settings(
    container: &AnimationSettingsGroup,
    style: crate::config::CursorStyle,
    shape: crate::config::CursorShape,
    canvas: TerminalCanvas,
) {
    container.clear();
    container
        .container
        .set_visible(style != crate::config::CursorStyle::Steady);
    container.container.set_margin_bottom(0);
    if style == crate::config::CursorStyle::Steady {
        return;
    }
    container.add(&animation_slider_row(
        AnimationSliderSpec {
            title: "Animation speed",
            key: "cursor_animation_duration_ms",
            value: f64::from(crate::config::cursor_animation_duration_ms()),
            min: 40.0,
            max: 500.0,
            step: 1.0,
            digits: 0,
            unit: "ms",
            default: f64::from(crate::config::DEFAULT_CURSOR_ANIMATION_DURATION_MS),
        },
        canvas.clone(),
    ));
    let advanced_expander = adw::ExpanderRow::builder()
        .title("Advanced animation settings")
        .expanded(false)
        .build();
    let advanced_content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    let mut has_advanced_settings = false;
    match style {
        crate::config::CursorStyle::Neovide => {
            advanced_content.append(&animation_slider_row(
                AnimationSliderSpec {
                    title: "Trail length",
                    key: "cursor_neovide_trail_size",
                    value: crate::config::cursor_neovide_trail_size(),
                    min: 0.0,
                    max: 1.0,
                    step: 0.01,
                    digits: 2,
                    unit: "",
                    default: crate::config::DEFAULT_NEOVIDE_TRAIL_SIZE,
                },
                canvas.clone(),
            ));
            advanced_content.append(&animation_slider_row(
                AnimationSliderSpec {
                    title: "Short jump distance",
                    key: "cursor_neovide_short_jump_distance",
                    value: crate::config::cursor_neovide_short_jump_distance(),
                    min: 0.0,
                    max: 8.0,
                    step: 0.1,
                    digits: 1,
                    unit: "cells",
                    default: crate::config::DEFAULT_NEOVIDE_SHORT_JUMP_DISTANCE,
                },
                canvas.clone(),
            ));
            advanced_content.append(&animation_slider_row(
                AnimationSliderSpec {
                    title: "Short jump speed",
                    key: "cursor_neovide_short_animation_duration_ms",
                    value: f64::from(crate::config::cursor_neovide_short_animation_duration_ms()),
                    min: 10.0,
                    max: 150.0,
                    step: 1.0,
                    digits: 0,
                    unit: "ms",
                    default: f64::from(crate::config::DEFAULT_NEOVIDE_SHORT_ANIMATION_DURATION_MS),
                },
                canvas.clone(),
            ));
            if shape == crate::config::CursorShape::Block {
                advanced_content.append(&animation_slider_row(
                    AnimationSliderSpec {
                        title: "Block cursor opacity",
                        key: "cursor_neovide_block_opacity",
                        value: crate::config::cursor_neovide_block_opacity(),
                        min: 0.1,
                        max: 1.0,
                        step: 0.01,
                        digits: 2,
                        unit: "",
                        default: crate::config::DEFAULT_NEOVIDE_BLOCK_OPACITY,
                    },
                    canvas,
                ));
            }
            has_advanced_settings = true;
        }
        crate::config::CursorStyle::Smooth
        | crate::config::CursorStyle::Snappy
        | crate::config::CursorStyle::Steady => {}
    }
    if has_advanced_settings {
        container.container.set_margin_bottom(12);
        advanced_expander.add_row(&advanced_content);
        let advanced_list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        advanced_list.set_widget_name("chelotype-advanced-animation-settings");
        advanced_list.append(&advanced_expander);
        container.add(&advanced_list);
    }
}

struct AnimationSliderSpec {
    title: &'static str,
    key: &'static str,
    value: f64,
    min: f64,
    max: f64,
    step: f64,
    digits: i32,
    unit: &'static str,
    default: f64,
}

fn animation_slider_row(spec: AnimationSliderSpec, canvas: TerminalCanvas) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(5)
        .margin_top(12)
        .margin_bottom(12)
        .build();
    if spec.title == "Animation speed" {
        row.set_widget_name("chelotype-animation-speed-row");
    }
    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .height_request(34)
        .build();
    let label = gtk::Label::builder()
        .label(spec.title)
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .build();
    let value_label = gtk::Label::builder()
        .halign(gtk::Align::End)
        .valign(gtk::Align::Center)
        .build();
    set_slider_value_label(&value_label, spec.value, spec.digits, spec.unit);
    let reset = gtk::Button::builder().css_classes(["flat"]).build();
    reset.set_tooltip_text(Some("Reset"));
    reset.update_property(&[gtk::accessible::Property::Label("Reset to default")]);
    let reset_icon = gtk::Image::builder()
        .icon_name("edit-undo-symbolic")
        .pixel_size(16)
        .build();
    reset_icon.set_can_target(false);
    reset.set_child(Some(&reset_icon));
    reset.set_visible(!slider_value_is_default(
        spec.value,
        spec.default,
        spec.step,
    ));
    set_pointer_cursor(&reset);
    let reset_slot = gtk::Box::builder()
        .height_request(34)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    reset_slot.append(&reset);
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, spec.min, spec.max, spec.step);
    set_pointer_cursor(&scale);
    scale.set_value(spec.value);
    scale.set_digits(spec.digits);
    scale.set_hexpand(true);
    {
        let scale = scale.clone();
        reset.connect_clicked(move |_| {
            scale.set_value(spec.default);
        });
    }
    {
        let reset = reset.clone();
        let value_label = value_label.clone();
        scale.connect_value_changed(move |scale| {
            let value = scale.value();
            let formatted = if spec.digits == 0 {
                format!("{value:.0}")
            } else {
                format!("{value:.2}")
            };
            crate::config::write_value(spec.key, &formatted);
            set_slider_value_label(&value_label, value, spec.digits, spec.unit);
            reset.set_visible(!slider_value_is_default(value, spec.default, spec.step));
            canvas.widget().queue_draw();
        });
    }
    header.append(&label);
    header.append(&value_label);
    header.append(&reset_slot);
    row.append(&header);
    row.append(&scale);
    row
}

fn slider_value_is_default(value: f64, default: f64, step: f64) -> bool {
    (value - default).abs() < (step / 2.0).max(f64::EPSILON)
}

fn set_slider_value_label(label: &gtk::Label, value: f64, digits: i32, unit: &str) {
    label.set_label(&format_slider_mark(value, digits, unit));
}

fn format_slider_mark(value: f64, digits: i32, unit: &str) -> String {
    if digits == 0 {
        if unit.is_empty() {
            format!("{value:.0}")
        } else {
            format!("{value:.0} {unit}")
        }
    } else {
        if unit.is_empty() {
            format!("{value:.2}")
        } else {
            format!("{value:.2} {unit}")
        }
    }
}

type PendingInputLatency =
    std::rc::Rc<std::cell::RefCell<std::collections::VecDeque<PendingInputSample>>>;

#[derive(Clone, Copy, Debug)]
struct PendingInputSample {
    started: std::time::Instant,
    allocations: crate::allocation_trace::AllocationSnapshot,
}

fn mark_pending_input_latency(pending_input_latency: &PendingInputLatency) {
    pending_input_latency
        .borrow_mut()
        .push_back(PendingInputSample {
            started: std::time::Instant::now(),
            allocations: crate::allocation_trace::snapshot(),
        });
}

fn record_pending_input_latency(pending_input_latency: &PendingInputLatency) {
    let finished = std::time::Instant::now();
    let allocations = crate::allocation_trace::snapshot();
    for sample in pending_input_latency.borrow_mut().drain(..) {
        crate::perf_trace::record_duration(
            "input_to_render",
            finished.saturating_duration_since(sample.started),
        );
        crate::perf_trace::record_counter(
            "input_allocs_to_render",
            allocations
                .allocations
                .saturating_sub(sample.allocations.allocations),
        );
        crate::perf_trace::record_counter(
            "input_alloc_bytes_to_render",
            allocations.bytes.saturating_sub(sample.allocations.bytes),
        );
    }
}

fn text_for_viewport_selection(
    content: &RenderableContentOwned,
    selection: SelectionRange,
) -> Option<String> {
    let text = selected_text_with_metadata(&content.lines, &content.line_metadata, selection);
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

fn visible_or_reanchored_selection(
    content: &RenderableContentOwned,
    selection: &std::rc::Rc<std::cell::Cell<Option<SelectionRange>>>,
    expected_text: Option<&str>,
) -> Option<SelectionRange> {
    if let Some(visible) = visible_selection_matching_text(content, selection.get(), expected_text)
    {
        return Some(visible);
    }
    visible_selection_for_content(content, selection.get())?;
    let expected_text = expected_text?;
    let reanchored = find_text_range(&content.lines, expected_text)?;
    selection.set(Some(anchor_range_to_display(
        reanchored,
        content.display_offset,
    )));
    Some(reanchored)
}

fn select_active_input(
    content: &std::rc::Rc<std::cell::RefCell<Option<RenderableContentOwned>>>,
    selection: &std::rc::Rc<std::cell::Cell<Option<SelectionRange>>>,
    selection_text: &std::rc::Rc<std::cell::RefCell<Option<String>>>,
    selection_dirty: &std::rc::Rc<std::cell::Cell<bool>>,
    keyboard_selection: &std::rc::Rc<std::cell::Cell<Option<DirectedSelectionRange>>>,
) {
    let Some(content) = content.borrow().clone() else {
        return;
    };
    let Some(range) = active_input_line_range(&content) else {
        return;
    };
    set_viewport_selection(
        &content,
        range,
        selection,
        selection_text,
        selection_dirty,
        keyboard_selection,
    );
}

fn select_command_block_output(
    content: &std::rc::Rc<std::cell::RefCell<Option<RenderableContentOwned>>>,
    direction: crate::command_blocks::CommandBlockDirection,
    selection: &std::rc::Rc<std::cell::Cell<Option<SelectionRange>>>,
    selection_text: &std::rc::Rc<std::cell::RefCell<Option<String>>>,
    selection_dirty: &std::rc::Rc<std::cell::Cell<bool>>,
    keyboard_selection: &std::rc::Rc<std::cell::Cell<Option<DirectedSelectionRange>>>,
) {
    let Some(content) = content.borrow().clone() else {
        return;
    };
    let Some(range) = command_block_output_range_near_cursor(&content, direction) else {
        return;
    };
    set_viewport_selection(
        &content,
        range,
        selection,
        selection_text,
        selection_dirty,
        keyboard_selection,
    );
}

fn select_mouse_click_range(
    content: &std::rc::Rc<std::cell::RefCell<Option<RenderableContentOwned>>>,
    selection: &std::rc::Rc<std::cell::Cell<Option<SelectionRange>>>,
    selection_text: &std::rc::Rc<std::cell::RefCell<Option<String>>>,
    selection_dirty: &std::rc::Rc<std::cell::Cell<bool>>,
    keyboard_selection: &std::rc::Rc<std::cell::Cell<Option<DirectedSelectionRange>>>,
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
    let range = if press_count >= 3 && Some(row) == usize::try_from(content.cursor_line).ok() {
        active_input_line_range(&content)
    } else if press_count >= 3 {
        line_range(&content.lines, row)
    } else {
        word_range_at(&content.lines, row, column)
    };
    let Some(range) = range else {
        return false;
    };
    set_viewport_selection(
        &content,
        range,
        selection,
        selection_text,
        selection_dirty,
        keyboard_selection,
    );
    true
}

fn command_block_output_selection_at_rail(
    content: &RenderableContentOwned,
    metrics: Option<CellMetrics>,
    target: PointerPanePosition,
    x: f64,
) -> Option<SelectionRange> {
    let metrics = metrics?;
    let x = x - metrics.offset_x;
    if metrics.width <= 0.0 || target.position.column != 0 {
        return None;
    }
    let origin_col = target.pane.map(|pane| pane.origin_col).unwrap_or(0);
    let local_x = x - origin_col as f64 * metrics.width;
    if !(0.0..=(metrics.width * 0.55)).contains(&local_x) {
        return None;
    }
    let row = usize::from(target.position.row);
    command_blocks(content)
        .into_iter()
        .find(|block| row >= block.prompt_start_row && row <= block.end_row)
        .and_then(|block| command_block_output_range(content, &block))
}

fn command_block_output_text_at_rail(
    content: &RenderableContentOwned,
    metrics: Option<CellMetrics>,
    target: PointerPanePosition,
    x: f64,
) -> Option<String> {
    let range = command_block_output_selection_at_rail(content, metrics, target, x)?;
    text_for_viewport_selection(content, range)
}

fn set_viewport_selection(
    content: &RenderableContentOwned,
    range: SelectionRange,
    selection: &std::rc::Rc<std::cell::Cell<Option<SelectionRange>>>,
    selection_text: &std::rc::Rc<std::cell::RefCell<Option<String>>>,
    selection_dirty: &std::rc::Rc<std::cell::Cell<bool>>,
    keyboard_selection: &std::rc::Rc<std::cell::Cell<Option<DirectedSelectionRange>>>,
) {
    keyboard_selection.set(None);
    selection.set(Some(anchor_range_to_display(range, content.display_offset)));
    *selection_text.borrow_mut() = text_for_viewport_selection(content, range);
    selection_dirty.set(true);
}

fn clear_selection(
    selection: &std::rc::Rc<std::cell::Cell<Option<SelectionRange>>>,
    selection_text: &std::rc::Rc<std::cell::RefCell<Option<String>>>,
    selection_dirty: &std::rc::Rc<std::cell::Cell<bool>>,
    keyboard_selection: &std::rc::Rc<std::cell::Cell<Option<DirectedSelectionRange>>>,
) {
    crate::logging::debug_log("clear selection");
    keyboard_selection.set(None);
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
    keyboard_selection: &std::rc::Rc<std::cell::Cell<Option<DirectedSelectionRange>>>,
    data: Vec<u8>,
) {
    if let Some(fresh_content) = workspace.borrow_mut().snapshot_active_renderable() {
        *content.borrow_mut() = Some(fresh_content);
    }
    let Some(selection_range) = selection.get() else {
        let content = content.borrow();
        let _ = write_active_input_edit(workspace, content.as_ref(), &data);
        return;
    };
    let Some(content) = content.borrow().clone() else {
        clear_selection(
            selection,
            selection_text,
            selection_dirty,
            keyboard_selection,
        );
        let _ = write_active_input_edit(workspace, None, &data);
        return;
    };
    selection.set(Some(selection_range));
    let expected_selection_text = selection_text.borrow().clone();
    let Some(viewport_selection) =
        visible_or_reanchored_selection(&content, selection, expected_selection_text.as_deref())
    else {
        clear_selection(
            selection,
            selection_text,
            selection_dirty,
            keyboard_selection,
        );
        let _ = write_active_input_edit(workspace, Some(&content), &data);
        return;
    };
    let Some(selected) = text_for_viewport_selection(&content, viewport_selection) else {
        clear_selection(
            selection,
            selection_text,
            selection_dirty,
            keyboard_selection,
        );
        let _ = write_active_input_edit(workspace, Some(&content), &data);
        return;
    };
    if !selection_within_active_input(&content, viewport_selection) {
        clear_selection(
            selection,
            selection_text,
            selection_dirty,
            keyboard_selection,
        );
        if data.as_slice() != [0x7f] && data.as_slice() != b"\x1b[3~" {
            let _ = write_active_input_edit(workspace, Some(&content), &data);
        }
        return;
    }
    let target = MouseGridPosition {
        column: viewport_selection.start.column.min(u16::MAX as usize) as u16,
        row: viewport_selection.start.row.min(u16::MAX as usize) as u16,
    };
    let target_point = GridPoint {
        row: viewport_selection.start.row + content.display_offset,
        column: viewport_selection.start.column,
    };
    let movement = if let Ok(true) = write_active_input_cursor_target(workspace, &content, target) {
        Vec::new()
    } else if let Some(bytes) = keyboard_selection
        .get()
        .and_then(|range| cursor_movement_bytes_between_points(range.focus, target_point))
    {
        bytes
    } else if content.cursor_line == i32::from(target.row)
        && content.cursor_col == i32::from(target.column)
    {
        Vec::new()
    } else if let Some(bytes) = cursor_movement_bytes_for_content(&content, target) {
        bytes
    } else {
        clear_selection(
            selection,
            selection_text,
            selection_dirty,
            keyboard_selection,
        );
        let _ = write_active_input_edit(workspace, Some(&content), &data);
        return;
    };

    let mut replacement = movement;
    for _ in selected.chars() {
        replacement.extend_from_slice(b"\x1b[3~");
    }
    if data.as_slice() != [0x7f] && data.as_slice() != b"\x1b[3~" {
        replacement.extend_from_slice(&data);
    }
    clear_selection(
        selection,
        selection_text,
        selection_dirty,
        keyboard_selection,
    );
    let _ = write_active_input_edit(workspace, Some(&content), &replacement);
}

fn write_active_input_edit(
    workspace: &std::rc::Rc<std::cell::RefCell<TerminalWorkspace>>,
    content: Option<&RenderableContentOwned>,
    data: &[u8],
) -> std::io::Result<()> {
    if crate::shell::default_shell_has_input_edit_bridge()
        && content.is_some_and(shell_input_bridge_active)
        && terminal_bytes_edit_input(data)
    {
        let mut combined = crate::shell::INPUT_UNDO_CAPTURE_SEQUENCE.to_vec();
        combined.extend_from_slice(data);
        workspace.borrow_mut().write_active(&combined)
    } else {
        workspace.borrow_mut().write_active(data)
    }
}

fn write_active_input_cursor_target(
    workspace: &std::rc::Rc<std::cell::RefCell<TerminalWorkspace>>,
    content: &RenderableContentOwned,
    target: MouseGridPosition,
) -> std::io::Result<bool> {
    if !shell_input_bridge_active(content) {
        return Ok(false);
    }
    let Some(offset) = input_buffer_offset_for_position(content, target) else {
        return Ok(false);
    };
    workspace
        .borrow_mut()
        .write_active_input_cursor_target(offset)
}

fn terminal_bytes_edit_input(data: &[u8]) -> bool {
    if data.is_empty() || data == b"\n" || data == b"\r" || data == [0x03] {
        return false;
    }
    true
}

fn write_input_undo(
    workspace: &std::rc::Rc<std::cell::RefCell<TerminalWorkspace>>,
    content: &std::rc::Rc<std::cell::RefCell<Option<RenderableContentOwned>>>,
) {
    let content = content.borrow();
    let bytes = if crate::shell::default_shell_has_input_edit_bridge()
        && content.as_ref().is_some_and(shell_input_bridge_active)
    {
        crate::shell::INPUT_UNDO_SEQUENCE
    } else {
        b"\x1a"
    };
    let _ = workspace.borrow_mut().write_active(bytes);
}

fn write_input_redo(
    workspace: &std::rc::Rc<std::cell::RefCell<TerminalWorkspace>>,
    content: &std::rc::Rc<std::cell::RefCell<Option<RenderableContentOwned>>>,
) {
    let content = content.borrow();
    if crate::shell::default_shell_has_input_edit_bridge()
        && content.as_ref().is_some_and(shell_input_bridge_active)
    {
        let _ = workspace
            .borrow_mut()
            .write_active(crate::shell::INPUT_REDO_SEQUENCE);
    }
}

fn shell_input_bridge_active(content: &RenderableContentOwned) -> bool {
    let Some(row) = usize::try_from(content.cursor_line).ok() else {
        return false;
    };
    content
        .line_metadata
        .get(row)
        .is_some_and(|metadata| metadata.semantic_prompt != TerminalSemanticPrompt::None)
}

fn current_mouse_mode(
    content: &std::rc::Rc<std::cell::RefCell<Option<RenderableContentOwned>>>,
    fallback: MouseMode,
) -> MouseMode {
    content
        .borrow()
        .as_ref()
        .map(|content| content.mouse)
        .unwrap_or(fallback)
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
    keyboard_selection: &std::rc::Rc<std::cell::Cell<Option<DirectedSelectionRange>>>,
    cursor_move: KeyboardMove,
) {
    if let Some(fresh_content) = workspace.borrow_mut().snapshot_active_renderable() {
        *content.borrow_mut() = Some(fresh_content);
    }
    let Some(content) = content.borrow().clone() else {
        return;
    };
    let terminal_cursor = active_cursor_point(&content);
    let directed_focus = keyboard_selection
        .get()
        .and_then(|range| range.viewport_focus(content.display_offset, content.lines.len()));
    let current_override = if cursor_move.selecting {
        directed_focus.or(terminal_cursor)
    } else {
        None
    };
    if !cursor_move.selecting
        && let Some(target) = keyboard_selection_collapse_target(
            selection.get(),
            content.display_offset,
            content.lines.len(),
            cursor_move.direction,
        )
    {
        let source = directed_focus.or(terminal_cursor).map(|point| GridPoint {
            row: point.row + content.display_offset,
            column: point.column,
        });
        clear_selection(
            selection,
            selection_text,
            selection_dirty,
            keyboard_selection,
        );
        let target_position = MouseGridPosition {
            row: target.row.min(u16::MAX as usize) as u16,
            column: target.column.min(u16::MAX as usize) as u16,
        };
        if let Ok(true) = write_active_input_cursor_target(workspace, &content, target_position) {
            return;
        }
        if let Some(bytes) = source
            .and_then(|source| cursor_movement_bytes_between_points(source, target))
            .or_else(|| cursor_movement_bytes_for_content(&content, target_position))
        {
            let _ = workspace.borrow_mut().write_active(&bytes);
        }
        return;
    }
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
            keyboard_selection,
        );
    } else if selection.get().is_some() {
        clear_selection(
            selection,
            selection_text,
            selection_dirty,
            keyboard_selection,
        );
    }
    if let Ok(true) = write_active_input_cursor_target(workspace, &content, target) {
        return;
    }
    if let Some(bytes) = keyboard_cursor_bytes(&content, current_override, target) {
        let _ = workspace.borrow_mut().write_active(&bytes);
    } else if !cursor_move.selecting {
        let _ = workspace
            .borrow_mut()
            .write_active(&cursor_move_terminal_bytes(
                cursor_move.direction,
                cursor_move.unit,
            ));
    }
}

fn select_keyboard_cursor_range(
    content: &RenderableContentOwned,
    target: MouseGridPosition,
    selection: &std::rc::Rc<std::cell::Cell<Option<SelectionRange>>>,
    selection_text: &std::rc::Rc<std::cell::RefCell<Option<String>>>,
    selection_dirty: &std::rc::Rc<std::cell::Cell<bool>>,
    keyboard_selection: &std::rc::Rc<std::cell::Cell<Option<DirectedSelectionRange>>>,
) {
    let Some(directed) =
        directed_selection_for_target(content, target, keyboard_selection.get(), selection.get())
    else {
        return;
    };
    if let Some(absolute) = directed.range() {
        keyboard_selection.set(Some(directed));
        selection.set(Some(absolute));
        let visible = visible_selection_for_content(content, Some(absolute));
        *selection_text.borrow_mut() =
            visible.and_then(|range| text_for_viewport_selection(content, range));
    } else {
        keyboard_selection.set(None);
        selection.set(None);
        *selection_text.borrow_mut() = None;
    }
    selection_dirty.set(true);
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

fn install_style(canvas: &gtk::DrawingArea, provider: &gtk::CssProvider) {
    refresh_style(provider);
    gtk::style_context_add_provider_for_display(
        &canvas.display(),
        provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn refresh_style(provider: &gtk::CssProvider) {
    provider.load_from_data(&app_style_css(
        crate::terminal_palette::default_terminal_palette(),
    ));
}

fn app_style_css(palette: &crate::terminal_palette::TerminalPalette) -> String {
    let css = "
        @define-color chelotype_background __BACKGROUND__;
        @define-color chelotype_foreground __FOREGROUND__;
        @define-color chelotype_chrome_bg mix(__BACKGROUND__, __FOREGROUND__, 0.06);
        @define-color chelotype_chrome_border alpha(__FOREGROUND__, 0.16);
        @define-color chelotype_chrome_hover alpha(__FOREGROUND__, 0.13);
        @define-color chelotype_chrome_active alpha(__FOREGROUND__, 0.19);

        .term-root,
        preferencespage,
        preferencesgroup {
            background-color: @chelotype_background;
            color: @chelotype_foreground;
        }

        .terminal-header,
        .term-root,
        preferencespage,
        preferencesgroup,
        .term-tab-bar,
        tabbar,
        tabbox,
        viewswitcherbar,
        popover.tab-menu contents {
            transition: background-color 300ms ease-out, color 300ms ease-out, border-color 300ms ease-out, box-shadow 300ms ease-out;
        }

        .terminal-header {
            background-color: @chelotype_chrome_bg;
            color: @chelotype_foreground;
            border-bottom: none;
            box-shadow: none;
            padding-top: 0;
            padding-bottom: 0;
        }
        drawingarea.term-canvas {
            background-color: transparent;
        }
        .term-tab-bar {
            background-color: transparent;
            color: @chelotype_foreground;
            box-shadow: none;
            border: none;
        }
        .term-tab-bar scrolledwindow,
        .term-tab-bar undershoot,
        tabbar,
        tabbox,
        .term-tab-bar > revealer > box,
        .term-tab-bar tabbox {
            background-color: transparent;
            color: @chelotype_foreground;
            box-shadow: none;
            border: none;
        }
        tabbar tab {
            color: @chelotype_foreground;
        }
        tabbar tab:hover {
            background-color: @chelotype_chrome_hover;
        }
        tabbar tab:selected {
            background-color: @chelotype_chrome_active;
        }
        popover.tab-menu contents {
            background-color: @chelotype_chrome_bg;
            color: @chelotype_foreground;
            padding: 0;
            min-width: 17.5rem;
        }
        popover.tab-menu searchentry,
        popover.tab-menu entry.search {
            margin: 0.5rem;
        }
        popover.tab-menu .launch-targets {
            background-color: transparent;
            padding: 0.375rem;
        }
        popover.tab-menu .launch-target-row {
            border-radius: 0.5625rem;
            min-height: 2rem;
            padding: 0;
            margin: 0;
        }
        popover.tab-menu .launch-target-row:hover {
            background-color: @chelotype_chrome_hover;
        }
        popover.tab-menu .launch-target-row:active {
            background-color: @chelotype_chrome_active;
        }
        popover.tab-menu .launch-target-row > * {
            padding: 0 0.75rem;
        }
        popover.tab-menu .launch-targets-header {
            padding: 0.75rem 0.375rem 0.375rem 0.75rem;
        }
        .terminal-rename-entry {
            min-width: 13rem;
            margin: 0.5rem;
        }
        viewswitcherbar {
            background-color: @chelotype_chrome_bg;
            color: @chelotype_foreground;
            border-top: 1px solid @chelotype_chrome_border;
        }
        button.palette-card-button {
            min-width: 0;
            min-height: 0;
            padding: 0;
            margin: 0;
            background-color: transparent;
            border: none;
            box-shadow: none;
        }
        button.preview-card-button {
            min-width: 0;
            min-height: 0;
            padding: 0;
        }
        flowboxchild.palette-flow-child {
            padding: 0;
            margin: 0;
        }
    ";
    css.replace("__BACKGROUND__", palette.background)
        .replace("__FOREGROUND__", palette.foreground)
}

fn trace_geometry(path: &std::path::Path, widget: &gtk::DrawingArea, metrics: TerminalMetrics) {
    let content = format!(
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
    write_trace_file(path, &content);
}

fn trace_tabs(
    path: &std::path::Path,
    root: &impl IsA<gtk::Widget>,
    tab_bar: &adw::TabBar,
    launch_menu_button: &gtk::MenuButton,
    workspace: &TerminalWorkspace,
) {
    let tabs = workspace.tabs();
    let selected_index = tabs.iter().position(|tab| tab.active).unwrap_or(0);
    let tab_bar_bounds = widget_trace_bounds(tab_bar, root);
    let launch_menu_bounds = widget_trace_bounds(launch_menu_button, root);
    let mut content = format!(
        "tab_bar_x={}\ntab_bar_y={}\ntab_bar_width={}\ntab_bar_height={}\nlaunch_menu_x={}\nlaunch_menu_y={}\nlaunch_menu_width={}\nlaunch_menu_height={}\ntab_count={}\nselected_index={}",
        tab_bar_bounds.x,
        tab_bar_bounds.y,
        tab_bar_bounds.width,
        tab_bar_bounds.height,
        launch_menu_bounds.x,
        launch_menu_bounds.y,
        launch_menu_bounds.width,
        launch_menu_bounds.height,
        tabs.len(),
        selected_index,
    );
    for tab in tabs {
        if let Some(title) = workspace.tab_title(tab.id) {
            content.push_str(&format!("\ntab_{}_title={title}", tab.index));
        }
        if let Some(target) = workspace.tab_launch_target(tab.id) {
            content.push_str(&format!(
                "\ntab_{}_launch_target={}",
                tab.index,
                target.id()
            ));
        }
    }
    write_trace_file(path, &content);
}

struct WidgetTraceBounds {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

fn widget_trace_bounds(
    widget: &impl IsA<gtk::Widget>,
    root: &impl IsA<gtk::Widget>,
) -> WidgetTraceBounds {
    if let Some(bounds) = widget.compute_bounds(root) {
        return WidgetTraceBounds {
            x: bounds.x().round() as i32,
            y: bounds.y().round() as i32,
            width: bounds.width().round() as i32,
            height: bounds.height().round() as i32,
        };
    }
    let widget = widget.as_ref();
    WidgetTraceBounds {
        x: widget.allocation().x(),
        y: widget.allocation().y(),
        width: widget.allocated_width(),
        height: widget.allocated_height(),
    }
}

fn write_trace_file(path: &std::path::Path, content: &str) {
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("trace")
    ));
    if std::fs::write(&tmp, content).is_ok() {
        let _ = std::fs::rename(tmp, path);
    }
}

fn record_tick_work(started: std::time::Instant) {
    crate::perf_trace::record_duration("gtk_tick_work", started.elapsed());
}

fn advance_smooth_scroll_frame(
    workspace: &std::rc::Rc<std::cell::RefCell<TerminalWorkspace>>,
    canvas: &TerminalCanvas,
    metrics: CellMetrics,
    smooth_scroll: &std::rc::Rc<std::cell::RefCell<crate::smooth_scroll::SmoothScroll>>,
    frame_duration: std::time::Duration,
    scroll_trace: Option<&std::path::Path>,
) -> bool {
    let Some(frame) = smooth_scroll
        .borrow_mut()
        .advance(frame_duration, metrics.height)
    else {
        return false;
    };
    if frame.line_delta != 0
        && !workspace
            .borrow_mut()
            .scroll_active_changed(frame.line_delta)
            .unwrap_or(false)
    {
        smooth_scroll.borrow_mut().cancel();
        canvas.set_scroll_visual_offset_px(0.0);
        trace_smooth_scroll(scroll_trace, "limit", frame.line_delta, 0.0);
        return false;
    }
    if frame.line_delta == 0 {
        canvas.set_scroll_visual_offset_px(frame.offset_px);
    } else {
        canvas.set_scroll_visual_offset_px_deferred(frame.offset_px);
    }
    trace_smooth_scroll_frame(
        scroll_trace,
        frame.line_delta,
        frame.remaining_px,
        frame.offset_px,
        frame_duration,
    );
    if !smooth_scroll.borrow().is_active() {
        trace_smooth_scroll(scroll_trace, "idle", 0, 0.0);
    }
    frame.line_delta != 0
}

fn configure_profile_scroll_burst(
    workspace: &std::rc::Rc<std::cell::RefCell<TerminalWorkspace>>,
    canvas: &TerminalCanvas,
    cell_metrics: &std::rc::Rc<std::cell::Cell<Option<CellMetrics>>>,
    smooth_scroll: &std::rc::Rc<std::cell::RefCell<crate::smooth_scroll::SmoothScroll>>,
    scroll_trace: Option<&std::path::Path>,
) {
    if std::env::var("CHELOTYPE_PROFILE_SCROLL_BURST")
        .ok()
        .as_deref()
        != Some("1")
    {
        return;
    }
    let burst_repeats = std::env::var("CHELOTYPE_PROFILE_SCROLL_BURST_REPEATS")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(1)
        .max(1);
    let seed_rows = 160 * burst_repeats;
    let seed_command = format!("for n in $(seq 1 {seed_rows}); do echo PROFILE_SCROLL_$n; done\n");
    let workspace_for_seed = workspace.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
        let _ = workspace_for_seed
            .borrow_mut()
            .write_active(seed_command.as_bytes());
    });

    let workspace = workspace.clone();
    let canvas = canvas.clone();
    let cell_metrics = cell_metrics.clone();
    let smooth_scroll = smooth_scroll.clone();
    let scroll_trace = scroll_trace.map(std::path::PathBuf::from);
    let attempts = std::rc::Rc::new(std::cell::Cell::new(0u32));
    let remaining_bursts = std::rc::Rc::new(std::cell::Cell::new(burst_repeats));
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        attempts.set(attempts.get() + 1);
        if attempts.get() > 120 {
            eprintln!("profile scroll burst could not find scrollback");
            return glib::ControlFlow::Break;
        }
        let Some(metrics) = cell_metrics.get() else {
            return glib::ControlFlow::Continue;
        };
        let lines = 3;
        let available_lines = workspace
            .borrow()
            .available_active_scroll_lines(lines)
            .unwrap_or(0);
        if available_lines == 0 {
            return glib::ControlFlow::Continue;
        }
        crate::perf_trace::record_counter("profile_scroll_burst_start", attempts.get().into());
        let delta_px = f64::from(lines) * metrics.height;
        let mut scroll = smooth_scroll.borrow_mut();
        for _ in 0..8 {
            let enqueued_px =
                scroll.enqueue_pixels_clamped(delta_px, metrics.height, available_lines);
            if enqueued_px.abs() >= 0.5 {
                trace_smooth_scroll(
                    scroll_trace.as_deref(),
                    "enqueue",
                    lines,
                    scroll.remaining_px(),
                );
            }
        }
        drop(scroll);
        remaining_bursts.set(remaining_bursts.get().saturating_sub(1));
        canvas.widget().queue_draw();
        if remaining_bursts.get() == 0 {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

fn trace_smooth_scroll(path: Option<&std::path::Path>, event: &str, lines: i32, value: f64) {
    trace_smooth_scroll_line(path, format!("{event}\t{lines}\t{value:.2}\n"));
}

fn trace_smooth_scroll_frame(
    path: Option<&std::path::Path>,
    lines: i32,
    remaining_px: f64,
    offset_px: f64,
    frame_duration: std::time::Duration,
) {
    trace_smooth_scroll_line(
        path,
        format!(
            "frame\t{lines}\t{remaining_px:.2}\t{offset_px:.2}\t{}\n",
            frame_duration.as_micros()
        ),
    );
}

fn trace_smooth_scroll_line(path: Option<&std::path::Path>, line: String) {
    let Some(path) = path else {
        return;
    };
    let Some(sink) = scroll_trace_sink(path) else {
        return;
    };
    sink.record_line(line);
}

fn scroll_trace_sink(path: &std::path::Path) -> Option<&'static crate::trace_sink::TraceSink> {
    static SINK: std::sync::OnceLock<Option<crate::trace_sink::TraceSink>> =
        std::sync::OnceLock::new();
    SINK.get_or_init(|| crate::trace_sink::TraceSink::new(path, "chelotype-scroll-trace"))
        .as_ref()
}

fn record_frame_clock_diagnostics(
    widget: &gtk::DrawingArea,
    frame_clock: &gtk::gdk::FrameClock,
    frame_time: i64,
    last_completed_frame_timing: &std::cell::Cell<Option<i64>>,
    last_presentation_time: &std::cell::Cell<Option<i64>>,
) {
    if !crate::perf_trace::enabled() {
        return;
    }
    let fps_millihz = (frame_clock.fps() * 1000.0).round();
    if fps_millihz.is_finite() && fps_millihz > 0.0 {
        crate::perf_trace::record_counter("gdk_frame_clock_fps_millihz", fps_millihz as u64);
    }
    let (refresh_interval, presentation_time) = frame_clock.refresh_info(frame_time);
    if refresh_interval > 0 {
        crate::perf_trace::record_duration(
            "gdk_refresh_interval",
            std::time::Duration::from_micros(refresh_interval as u64),
        );
    }
    if presentation_time >= frame_time {
        crate::perf_trace::record_duration(
            "gdk_next_presentation_delta",
            std::time::Duration::from_micros((presentation_time - frame_time) as u64),
        );
    }
    if let Some(refresh_rate) = widget
        .native()
        .and_then(|native| native.surface())
        .and_then(|surface| surface.display().monitor_at_surface(&surface))
        .map(|monitor| monitor.refresh_rate())
        .filter(|refresh_rate| *refresh_rate > 0)
    {
        crate::perf_trace::record_counter("gdk_monitor_refresh_millihz", refresh_rate as u64);
    }
    let current_counter = frame_clock.frame_counter();
    let first_counter = last_completed_frame_timing
        .get()
        .map_or_else(|| frame_clock.history_start(), |counter| counter + 1)
        .max(frame_clock.history_start());
    for counter in first_counter..=current_counter {
        let Some(timing) = frame_clock.timings(counter) else {
            continue;
        };
        if !timing.is_complete() {
            break;
        }
        last_completed_frame_timing.set(Some(timing.frame_counter()));
        let timing_frame_time = timing.frame_time();
        let timing_refresh_interval = timing.refresh_interval();
        let predicted_presentation_time = timing.predicted_presentation_time();
        let actual_presentation_time = timing.presentation_time();
        if timing_refresh_interval > 0 {
            crate::perf_trace::record_duration(
                "gdk_timings_refresh_interval",
                std::time::Duration::from_micros(timing_refresh_interval as u64),
            );
        }
        if actual_presentation_time > 0 {
            crate::perf_trace::record_duration(
                "gdk_timings_frame_to_presentation",
                std::time::Duration::from_micros(
                    actual_presentation_time.abs_diff(timing_frame_time),
                ),
            );
        }
        if actual_presentation_time > 0 && predicted_presentation_time > 0 {
            crate::perf_trace::record_duration(
                "gdk_timings_presentation_error",
                std::time::Duration::from_micros(
                    actual_presentation_time.abs_diff(predicted_presentation_time),
                ),
            );
        }
        if actual_presentation_time > 0 {
            if let Some(previous_presentation_time) =
                last_presentation_time.replace(Some(actual_presentation_time))
                && actual_presentation_time >= previous_presentation_time
            {
                crate::perf_trace::record_duration(
                    "gdk_timings_presentation_interval",
                    std::time::Duration::from_micros(
                        (actual_presentation_time - previous_presentation_time) as u64,
                    ),
                );
            }
            crate::perf_trace::record_counter("gdk_timings_complete", 1);
        }
    }
}

fn cancel_click_gesture(
    drag_gesture_active: &std::cell::Cell<bool>,
    left_pointer_down: &std::cell::Cell<bool>,
    pointer_pane_capture: &std::cell::Cell<Option<PaneHit>>,
    pointer_interaction: &std::cell::RefCell<PointerInteraction>,
) -> Vec<InteractionEffect> {
    if drag_gesture_active.get() {
        crate::logging::debug_log("mouse click gesture cancel ignored during active drag");
        return Vec::new();
    }
    left_pointer_down.set(false);
    pointer_pane_capture.set(None);
    pointer_interaction.borrow_mut().cancel()
}

fn apply_preedit_to_workspace_render(
    render: &mut WorkspaceRenderFrame,
    preedit: Option<&PendingPreedit>,
) {
    if let Some(active) = render.panes.iter_mut().find(|pane| pane.active) {
        apply_preedit_to_render_frame(&mut active.frame, preedit);
    }
}

fn apply_preedit_to_render_frame(render: &mut RenderFrame, preedit: Option<&PendingPreedit>) {
    render.preedit = preedit.map(|preedit| RenderPreedit {
        text: preedit.text.clone(),
        cursor: preedit.cursor,
        columns: display_columns(&preedit.text).max(1),
        cursor_columns: display_columns_until_char(&preedit.text, preedit.cursor),
        line: render.cursor.line,
        column: render.cursor.column,
    });
}

fn apply_interaction_effects(
    effects: Vec<InteractionEffect>,
    workspace: &std::rc::Rc<std::cell::RefCell<TerminalWorkspace>>,
    selection: &std::rc::Rc<std::cell::Cell<Option<SelectionRange>>>,
    selection_text: &std::rc::Rc<std::cell::RefCell<Option<String>>>,
    selection_dirty: &std::rc::Rc<std::cell::Cell<bool>>,
    keyboard_selection: &std::rc::Rc<std::cell::Cell<Option<DirectedSelectionRange>>>,
    content: &std::rc::Rc<std::cell::RefCell<Option<RenderableContentOwned>>>,
) {
    for effect in effects {
        match effect {
            InteractionEffect::Write(bytes) => {
                let _ = workspace.borrow_mut().write_active(&bytes);
            }
            InteractionEffect::SelectionChanged(range) => {
                crate::logging::debug_log(&format!("selection changed {range:?}"));
                keyboard_selection.set(None);
                if let Some(range) = range {
                    if let Some(content) = content.borrow().as_ref() {
                        selection.set(Some(anchor_range_to_display(range, content.display_offset)));
                        *selection_text.borrow_mut() = text_for_viewport_selection(content, range);
                        selection_dirty.set(true);
                    }
                } else {
                    clear_selection(
                        selection,
                        selection_text,
                        selection_dirty,
                        keyboard_selection,
                    );
                }
            }
            InteractionEffect::MoveCursorTo(position) => {
                if let Some(content) = content.borrow().as_ref()
                    && let Ok(true) = write_active_input_cursor_target(workspace, content, position)
                {
                    continue;
                }
                if let Some(content) = content.borrow().as_ref()
                    && let Some(bytes) = cursor_movement_bytes_for_content(content, position)
                {
                    let _ = workspace.borrow_mut().write_active(&bytes);
                }
            }
        }
    }
}

fn input_selection_drag_for_content(
    content: &RenderableContentOwned,
    position: MouseGridPosition,
) -> Option<InputSelectionDrag> {
    input_position_in_editable_input(content, position).then_some(InputSelectionDrag {
        anchor: position,
        cursor: None,
    })
}

fn input_selection_cursor_target(
    anchor: MouseGridPosition,
    focus: MouseGridPosition,
) -> MouseGridPosition {
    let anchor_point = GridPoint::from(anchor);
    let focus_point = GridPoint::from(focus);
    if point_le(anchor_point, focus_point) {
        MouseGridPosition {
            row: focus.row,
            column: focus.column.saturating_add(1),
        }
    } else {
        focus
    }
}

fn follow_input_selection_cursor(
    drag: &std::rc::Rc<std::cell::Cell<Option<InputSelectionDrag>>>,
    workspace: &std::rc::Rc<std::cell::RefCell<TerminalWorkspace>>,
    content: &std::rc::Rc<std::cell::RefCell<Option<RenderableContentOwned>>>,
    canvas: &TerminalCanvas,
    focus: MouseGridPosition,
) -> bool {
    let Some(mut state) = drag.get() else {
        return false;
    };
    let Some(content) = content.borrow().as_ref().cloned() else {
        return false;
    };
    let target = input_selection_cursor_target(state.anchor, focus);
    if !input_position_in_editable_input(&content, target) {
        return false;
    }
    if let Ok(true) = write_active_input_cursor_target(workspace, &content, target) {
        state.cursor = Some(target);
        drag.set(Some(state));
        canvas.set_cursor_motion_suppressed(true);
        return true;
    }
    let Some((updated_state, bytes)) = input_selection_cursor_step(&content, state, focus) else {
        return false;
    };
    state = updated_state;
    drag.set(Some(state));
    canvas.set_cursor_motion_suppressed(true);
    let _ = workspace.borrow_mut().write_active(&bytes);
    true
}

fn input_selection_cursor_step(
    content: &RenderableContentOwned,
    mut state: InputSelectionDrag,
    focus: MouseGridPosition,
) -> Option<(InputSelectionDrag, Vec<u8>)> {
    let target = input_selection_cursor_target(state.anchor, focus);
    if !input_position_in_editable_input(content, target) {
        return None;
    }
    let bytes = input_selection_cursor_movement(content, state, target)?;
    state.cursor = Some(target);
    Some((state, bytes))
}

fn input_selection_cursor_movement(
    content: &RenderableContentOwned,
    state: InputSelectionDrag,
    target: MouseGridPosition,
) -> Option<Vec<u8>> {
    if let Some(cursor) = state.cursor {
        cursor_movement_bytes_between_editable_input_points(content, cursor, target)
    } else {
        cursor_movement_bytes_for_editable_input(content, target)
    }
}

fn point_le(left: GridPoint, right: GridPoint) -> bool {
    left.row < right.row || (left.row == right.row && left.column <= right.column)
}

#[derive(Clone, Copy)]
struct TerminalMetrics {
    size: ScreenSize,
    cell: CellMetrics,
}

#[derive(Clone, Copy)]
struct CachedTerminalMetrics {
    allocation_width: i32,
    allocation_height: i32,
    metrics: TerminalMetrics,
}

impl CachedTerminalMetrics {
    fn matches(self, allocation_width: i32, allocation_height: i32) -> bool {
        self.allocation_width == allocation_width && self.allocation_height == allocation_height
    }
}

#[derive(Clone, Copy)]
struct CellMetrics {
    width: f64,
    height: f64,
    offset_x: f64,
    offset_y: f64,
}

#[derive(Clone, Copy)]
struct PaneHit {
    id: PaneId,
    origin_col: usize,
    cols: usize,
}

#[derive(Clone, Copy)]
struct SplitResizeDrag {
    boundary_index: usize,
    start_x: f64,
    applied_delta_cols: i16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct InputSelectionDrag {
    anchor: MouseGridPosition,
    cursor: Option<MouseGridPosition>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingPreedit {
    text: String,
    cursor: usize,
}

#[derive(Clone, Copy)]
struct PointerPanePosition {
    pane: Option<PaneHit>,
    position: MouseGridPosition,
}

struct PointerPaneActivation<'a> {
    workspace: &'a std::rc::Rc<std::cell::RefCell<TerminalWorkspace>>,
    content: &'a std::rc::Rc<std::cell::RefCell<Option<RenderableContentOwned>>>,
    mouse_mode: &'a std::rc::Rc<std::cell::Cell<MouseMode>>,
    active_origin: &'a std::rc::Rc<std::cell::Cell<usize>>,
    selection: &'a std::rc::Rc<std::cell::Cell<Option<SelectionRange>>>,
    selection_text: &'a std::rc::Rc<std::cell::RefCell<Option<String>>>,
    selection_dirty: &'a std::rc::Rc<std::cell::Cell<bool>>,
    keyboard_selection: &'a std::rc::Rc<std::cell::Cell<Option<DirectedSelectionRange>>>,
}

fn terminal_metrics_for_widget(widget: &gtk::DrawingArea) -> Option<TerminalMetrics> {
    let padding = crate::canvas::terminal_canvas_padding(widget);
    let width = padding.content_width(widget);
    let height = padding.content_height(widget);
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let font_metrics = metrics_for_widget(widget)?;
    let cols = ((width / font_metrics.cell_width).floor() as i32).clamp(1, u16::MAX as i32) as u16;
    let rows =
        ((height / font_metrics.line_height).floor() as i32).clamp(1, u16::MAX as i32) as u16;
    Some(TerminalMetrics {
        size: ScreenSize::new(cols, rows).ok()?,
        cell: CellMetrics {
            width: font_metrics.cell_width,
            height: font_metrics.line_height,
            offset_x: padding.left,
            offset_y: padding.top,
        },
    })
}

fn terminal_metrics_for_widget_cached(
    widget: &gtk::DrawingArea,
    cache: &std::cell::Cell<Option<CachedTerminalMetrics>>,
    force_refresh: bool,
) -> Option<TerminalMetrics> {
    let width = widget.allocated_width();
    let height = widget.allocated_height();
    if width <= 0 || height <= 0 {
        cache.set(None);
        return None;
    }
    if !force_refresh
        && let Some(cached) = cache.get()
        && cached.matches(width, height)
    {
        return Some(cached.metrics);
    }
    let started = std::time::Instant::now();
    let metrics = terminal_metrics_for_widget(widget)?;
    crate::perf_trace::record_duration("gtk_metrics", started.elapsed());
    cache.set(Some(CachedTerminalMetrics {
        allocation_width: width,
        allocation_height: height,
        metrics,
    }));
    Some(metrics)
}

fn pointer_grid_position_for_panes(
    metrics: Option<CellMetrics>,
    panes: &[PaneHit],
    x: f64,
    y: f64,
) -> Option<PointerPanePosition> {
    let metrics = metrics?;
    let x = x - metrics.offset_x;
    let y = y - metrics.offset_y;
    if x < 0.0 || y < 0.0 || metrics.width <= 0.0 || metrics.height <= 0.0 {
        return None;
    }
    let column = (x / metrics.width).floor().max(0.0) as usize;
    let row = ((y / metrics.height).floor() as i32).clamp(0, u16::MAX as i32) as u16;
    if let Some(pane) = panes.iter().copied().find(|pane| {
        column >= pane.origin_col && column < pane.origin_col.saturating_add(pane.cols)
    }) {
        return Some(PointerPanePosition {
            pane: Some(pane),
            position: MouseGridPosition {
                column: column
                    .saturating_sub(pane.origin_col)
                    .min(u16::MAX as usize) as u16,
                row,
            },
        });
    }
    Some(PointerPanePosition {
        pane: None,
        position: MouseGridPosition {
            column: column.min(u16::MAX as usize) as u16,
            row,
        },
    })
}

fn split_resize_boundary_at(
    metrics: Option<CellMetrics>,
    panes: &[PaneHit],
    x: f64,
) -> Option<usize> {
    let metrics = metrics?;
    let x = x - metrics.offset_x;
    if panes.len() < 2 || x < 0.0 || metrics.width <= 0.0 {
        return None;
    }
    let threshold = (metrics.width * 0.6).max(8.0);
    panes.windows(2).enumerate().find_map(|(index, pair)| {
        let [left, right] = pair else {
            return None;
        };
        if left.origin_col.saturating_add(left.cols) != right.origin_col {
            return None;
        }
        let boundary_x = right.origin_col as f64 * metrics.width;
        ((x - boundary_x).abs() <= threshold).then_some(index)
    })
}

fn pointer_grid_position_for_capture_or_panes(
    metrics: Option<CellMetrics>,
    captured_pane: Option<PaneHit>,
    panes: &[PaneHit],
    x: f64,
    y: f64,
) -> Option<PointerPanePosition> {
    if let Some(pane) = captured_pane {
        pointer_grid_position_for_pane(metrics, pane, x, y)
    } else {
        pointer_grid_position_for_panes(metrics, panes, x, y)
    }
}

fn pointer_grid_position_for_pane(
    metrics: Option<CellMetrics>,
    pane: PaneHit,
    x: f64,
    y: f64,
) -> Option<PointerPanePosition> {
    let metrics = metrics?;
    let x = x - metrics.offset_x;
    let y = y - metrics.offset_y;
    if y < 0.0 || metrics.width <= 0.0 || metrics.height <= 0.0 {
        return None;
    }
    let local_x = x - pane.origin_col as f64 * metrics.width;
    let max_column = pane.cols.saturating_sub(1).min(u16::MAX as usize);
    Some(PointerPanePosition {
        pane: Some(pane),
        position: MouseGridPosition {
            column: ((local_x / metrics.width).floor() as i32).clamp(0, max_column as i32) as u16,
            row: ((y / metrics.height).floor() as i32).clamp(0, u16::MAX as i32) as u16,
        },
    })
}

fn pointer_cursor_position_for_target(
    metrics: Option<CellMetrics>,
    target: PointerPanePosition,
    x: f64,
    y: f64,
) -> Option<MouseGridPosition> {
    let metrics = metrics?;
    let x = x - metrics.offset_x;
    let y = y - metrics.offset_y;
    if x < 0.0 || y < 0.0 || metrics.width <= 0.0 || metrics.height <= 0.0 {
        return None;
    }
    let origin_col = target.pane.map(|pane| pane.origin_col).unwrap_or(0);
    let local_x = x - origin_col as f64 * metrics.width;
    if local_x < 0.0 {
        return None;
    }
    Some(MouseGridPosition {
        column: ((local_x / metrics.width).round() as i32).clamp(0, u16::MAX as i32) as u16,
        row: ((y / metrics.height).floor() as i32).clamp(0, u16::MAX as i32) as u16,
    })
}

fn activate_pointer_pane(target: PointerPanePosition, activation: PointerPaneActivation<'_>) {
    let Some(pane) = target.pane else {
        return;
    };
    let already_active = activation.workspace.borrow().active_pane_id() == pane.id;
    if !already_active && activation.workspace.borrow_mut().activate_pane(pane.id) {
        clear_selection(
            activation.selection,
            activation.selection_text,
            activation.selection_dirty,
            activation.keyboard_selection,
        );
    }
    activation.active_origin.set(pane.origin_col);
    if let Some(fresh_content) = activation
        .workspace
        .borrow_mut()
        .snapshot_active_renderable()
    {
        activation.mouse_mode.set(fresh_content.mouse);
        *activation.content.borrow_mut() = Some(fresh_content);
    }
}

fn mouse_button_from_gesture(gesture: &gtk::GestureClick) -> Option<MouseButton> {
    match gesture.current_button() {
        1 => Some(MouseButton::Left),
        2 => Some(MouseButton::Middle),
        3 => Some(MouseButton::Right),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics() -> Option<CellMetrics> {
        Some(CellMetrics {
            width: 10.0,
            height: 20.0,
            offset_x: 0.0,
            offset_y: 0.0,
        })
    }

    #[test]
    fn cached_terminal_metrics_match_only_same_allocation() {
        let metrics = TerminalMetrics {
            size: ScreenSize::new(80, 24).expect("valid size"),
            cell: CellMetrics {
                width: 10.0,
                height: 20.0,
                offset_x: 0.0,
                offset_y: 0.0,
            },
        };
        let cached = CachedTerminalMetrics {
            allocation_width: 800,
            allocation_height: 480,
            metrics,
        };

        assert!(cached.matches(800, 480));
        assert!(!cached.matches(801, 480));
        assert!(!cached.matches(800, 481));
    }

    #[test]
    fn input_selection_cursor_target_tracks_drag_focus_edge() {
        assert_eq!(
            input_selection_cursor_target(
                MouseGridPosition { row: 0, column: 3 },
                MouseGridPosition { row: 0, column: 7 },
            ),
            MouseGridPosition { row: 0, column: 8 }
        );
        assert_eq!(
            input_selection_cursor_target(
                MouseGridPosition { row: 0, column: 7 },
                MouseGridPosition { row: 0, column: 3 },
            ),
            MouseGridPosition { row: 0, column: 3 }
        );
        assert_eq!(
            input_selection_cursor_target(
                MouseGridPosition { row: 0, column: 7 },
                MouseGridPosition { row: 1, column: 2 },
            ),
            MouseGridPosition { row: 1, column: 3 }
        );
    }

    #[test]
    fn input_selection_cursor_movement_uses_logical_drag_cursor_after_first_move() {
        let content = input_content("❯ manualinput", 14);
        let initial = InputSelectionDrag {
            anchor: MouseGridPosition { row: 0, column: 2 },
            cursor: None,
        };
        assert_eq!(
            input_selection_cursor_movement(
                &content,
                initial,
                MouseGridPosition { row: 0, column: 8 },
            )
            .as_deref(),
            Some(&b"\x1b[D\x1b[D\x1b[D\x1b[D\x1b[D"[..])
        );

        let moved = InputSelectionDrag {
            cursor: Some(MouseGridPosition { row: 0, column: 8 }),
            ..initial
        };
        assert_eq!(
            input_selection_cursor_movement(
                &content,
                moved,
                MouseGridPosition { row: 0, column: 6 }
            )
            .as_deref(),
            Some(&b"\x1b[D\x1b[D"[..])
        );
    }

    #[test]
    fn input_selection_cursor_movement_does_not_fallback_from_invalid_logical_cursor() {
        let content = input_content("❯ manualinput", 14);
        let stale = InputSelectionDrag {
            anchor: MouseGridPosition { row: 0, column: 2 },
            cursor: Some(MouseGridPosition { row: 9, column: 8 }),
        };

        assert_eq!(
            input_selection_cursor_movement(
                &content,
                stale,
                MouseGridPosition { row: 0, column: 6 }
            ),
            None
        );
    }

    #[test]
    fn input_selection_cursor_step_tracks_forward_drag_from_end_cursor() {
        let content = input_content("❯ manualinput", 14);
        let initial = InputSelectionDrag {
            anchor: MouseGridPosition { row: 0, column: 2 },
            cursor: None,
        };

        let (state, bytes) =
            input_selection_cursor_step(&content, initial, MouseGridPosition { row: 0, column: 4 })
                .expect("forward input drag step");
        assert_eq!(bytes, left_arrows(8));
        assert_eq!(
            state,
            InputSelectionDrag {
                anchor: MouseGridPosition { row: 0, column: 2 },
                cursor: Some(MouseGridPosition { row: 0, column: 5 }),
            }
        );

        let (state, bytes) =
            input_selection_cursor_step(&content, state, MouseGridPosition { row: 0, column: 8 })
                .expect("forward input drag follow step");
        assert_eq!(bytes, right_arrows(4));
        assert_eq!(state.cursor, Some(MouseGridPosition { row: 0, column: 9 }));
    }

    #[test]
    fn input_selection_cursor_step_tracks_reverse_drag_from_start_cursor() {
        let content = input_content("❯ manualinput", 2);
        let initial = InputSelectionDrag {
            anchor: MouseGridPosition { row: 0, column: 10 },
            cursor: None,
        };

        let (state, bytes) =
            input_selection_cursor_step(&content, initial, MouseGridPosition { row: 0, column: 7 })
                .expect("reverse input drag step");
        assert_eq!(bytes, right_arrows(5));
        assert_eq!(state.cursor, Some(MouseGridPosition { row: 0, column: 7 }));

        let (state, bytes) =
            input_selection_cursor_step(&content, state, MouseGridPosition { row: 0, column: 4 })
                .expect("reverse input drag follow step");
        assert_eq!(bytes, left_arrows(3));
        assert_eq!(state.cursor, Some(MouseGridPosition { row: 0, column: 4 }));
    }

    #[test]
    fn input_selection_cursor_step_does_not_move_when_focus_leaves_input_and_resumes_on_return() {
        let content = command_block_content_with_active_input("❯ activeinput");
        let initial = InputSelectionDrag {
            anchor: MouseGridPosition { row: 3, column: 2 },
            cursor: None,
        };

        let (state, bytes) =
            input_selection_cursor_step(&content, initial, MouseGridPosition { row: 3, column: 3 })
                .expect("initial input drag step");
        assert_eq!(bytes, right_arrows(2));
        assert_eq!(state.cursor, Some(MouseGridPosition { row: 3, column: 4 }));

        assert_eq!(
            input_selection_cursor_step(&content, state, MouseGridPosition { row: 2, column: 5 }),
            None
        );

        let (state, bytes) =
            input_selection_cursor_step(&content, state, MouseGridPosition { row: 3, column: 4 })
                .expect("returned input drag step");
        assert_eq!(bytes, right_arrows(1));
        assert_eq!(state.cursor, Some(MouseGridPosition { row: 3, column: 5 }));
    }

    #[test]
    fn input_selection_cursor_step_does_not_move_when_drag_enters_prompt_line() {
        let content = semantic_prompt_with_active_input();
        let initial = InputSelectionDrag {
            anchor: MouseGridPosition { row: 1, column: 2 },
            cursor: None,
        };

        let (state, bytes) =
            input_selection_cursor_step(&content, initial, MouseGridPosition { row: 1, column: 4 })
                .expect("initial editable input drag step");
        assert_eq!(bytes, left_arrows(3));
        assert_eq!(state.cursor, Some(MouseGridPosition { row: 1, column: 5 }));

        assert_eq!(
            input_selection_cursor_step(&content, state, MouseGridPosition { row: 0, column: 12 }),
            None
        );

        let (state, bytes) =
            input_selection_cursor_step(&content, state, MouseGridPosition { row: 1, column: 5 })
                .expect("returned editable input drag step");
        assert_eq!(bytes, right_arrows(1));
        assert_eq!(state.cursor, Some(MouseGridPosition { row: 1, column: 6 }));
    }

    #[test]
    fn input_selection_cursor_step_supports_cursor_already_on_first_drag_edge() {
        let content = input_content("❯ manualinput", 5);
        let initial = InputSelectionDrag {
            anchor: MouseGridPosition { row: 0, column: 2 },
            cursor: None,
        };

        assert_eq!(
            input_selection_cursor_step(&content, initial, MouseGridPosition { row: 0, column: 4 }),
            None
        );

        let (state, bytes) =
            input_selection_cursor_step(&content, initial, MouseGridPosition { row: 0, column: 5 })
                .expect("next discrete input drag step");
        assert_eq!(bytes, right_arrows(1));
        assert_eq!(state.cursor, Some(MouseGridPosition { row: 0, column: 6 }));
    }

    #[test]
    fn paste_bytes_use_bracketed_paste_when_terminal_requests_it() {
        assert_eq!(
            terminal_paste_bytes(true, b"hello"),
            b"\x1b[200~hello\x1b[201~"
        );
    }

    #[test]
    fn paste_bytes_leave_end_marker_unwrapped() {
        assert_eq!(terminal_paste_bytes(true, b"a\x1b[201~b"), b"a\x1b[201~b");
    }

    #[test]
    fn paste_bytes_are_plain_without_bracketed_paste_mode() {
        assert_eq!(terminal_paste_bytes(false, b"hello"), b"hello");
    }

    #[test]
    fn input_selection_cursor_step_supports_multiline_wrapped_input_edges() {
        use crate::terminal_grid::{
            MouseMode, TerminalColors, TerminalContent, TerminalLineMetadata,
        };
        let content = TerminalContent {
            lines: vec![
                vec![crate::terminal_grid::TerminalCell::blank(); 4],
                vec![crate::terminal_grid::TerminalCell::blank(); 4],
                vec![crate::terminal_grid::TerminalCell::blank(); 4],
            ],
            line_metadata: vec![
                TerminalLineMetadata {
                    wrapped: true,
                    ..TerminalLineMetadata::default()
                },
                TerminalLineMetadata {
                    wrapped: true,
                    wrap_continuation: true,
                    ..TerminalLineMetadata::default()
                },
                TerminalLineMetadata {
                    wrap_continuation: true,
                    ..TerminalLineMetadata::default()
                },
            ],
            cursor_line: 2,
            cursor_col: 1,
            cursor_visible: true,
            display_offset: 0,
            colors: TerminalColors::default(),
            mouse: MouseMode::default(),
        };
        let initial = InputSelectionDrag {
            anchor: MouseGridPosition { row: 0, column: 1 },
            cursor: None,
        };

        let (state, bytes) =
            input_selection_cursor_step(&content, initial, MouseGridPosition { row: 1, column: 2 })
                .expect("wrapped forward input drag step");
        assert_eq!(bytes, left_arrows(2));
        assert_eq!(state.cursor, Some(MouseGridPosition { row: 1, column: 3 }));

        let (state, bytes) =
            input_selection_cursor_step(&content, state, MouseGridPosition { row: 2, column: 0 })
                .expect("wrapped forward input drag follow step");
        assert_eq!(bytes, right_arrows(2));
        assert_eq!(state.cursor, Some(MouseGridPosition { row: 2, column: 1 }));
    }

    #[test]
    fn scrolling_preview_waits_before_first_scroll() {
        let mut preview = ScrollingPreviewState::with_profile(vec![WheelProfileEvent {
            elapsed_ms: 0.0,
            lines: 3.0,
        }]);

        for _ in 0..120 {
            preview.advance(crate::frame_timing::TARGET_FRAME_DURATION);
        }

        assert_eq!(preview.phase, ScrollingPreviewPhase::InitialPause);
        assert_eq!(preview.direct_lines(), 0.0);
        assert_eq!(preview.smooth_lines(), 0.0);

        preview.advance(crate::frame_timing::TARGET_FRAME_DURATION);

        assert_eq!(preview.phase, ScrollingPreviewPhase::Forward);
    }

    #[test]
    fn scrolling_preview_profile_normalizes_to_whole_line_endpoint() {
        let events = normalize_preview_wheel_profile(vec![
            WheelProfileEvent {
                elapsed_ms: 0.0,
                lines: 1.2,
            },
            WheelProfileEvent {
                elapsed_ms: 32.0,
                lines: 1.2,
            },
            WheelProfileEvent {
                elapsed_ms: 64.0,
                lines: 1.2,
            },
        ]);

        let total = events.iter().map(|event| event.lines).sum::<f64>();

        assert!((total - 4.0).abs() < 0.000_001, "{total}");
        assert!(events.iter().all(|event| event.lines > 0.0));
    }

    #[test]
    fn scrolling_preview_keeps_completed_smooth_position_until_reverse_phase() {
        let mut preview = ScrollingPreviewState::with_profile(vec![
            WheelProfileEvent {
                elapsed_ms: 0.0,
                lines: 3.0,
            },
            WheelProfileEvent {
                elapsed_ms: 80.0,
                lines: 3.0,
            },
        ]);

        for _ in 0..240 {
            preview.advance(crate::frame_timing::TARGET_FRAME_DURATION);
            if preview.phase == ScrollingPreviewPhase::BottomPause
                && !preview.smooth_scroll.is_active()
            {
                break;
            }
        }

        assert_eq!(preview.phase, ScrollingPreviewPhase::BottomPause);
        assert_eq!(preview.direct_lines(), 6.0);
        assert_eq!(preview.smooth_lines(), 6.0);

        preview.advance(std::time::Duration::from_millis(200));

        assert_eq!(preview.phase, ScrollingPreviewPhase::BottomPause);
        assert_eq!(preview.smooth_lines(), 6.0);
    }

    #[test]
    fn scrolling_preview_returns_smooth_position_to_top_after_reverse() {
        let mut preview = ScrollingPreviewState::with_profile(vec![
            WheelProfileEvent {
                elapsed_ms: 0.0,
                lines: 3.0,
            },
            WheelProfileEvent {
                elapsed_ms: 80.0,
                lines: 3.0,
            },
        ]);

        for _ in 0..1200 {
            preview.advance(crate::frame_timing::TARGET_FRAME_DURATION);
            if preview.phase == ScrollingPreviewPhase::TopPause
                && !preview.smooth_scroll.is_active()
            {
                break;
            }
        }

        assert_eq!(preview.phase, ScrollingPreviewPhase::TopPause);
        assert_eq!(preview.direct_lines(), 0.0);
        assert_eq!(preview.smooth_lines(), 0.0);
    }

    fn pane(origin_col: usize, cols: usize) -> PaneHit {
        PaneHit {
            id: PaneId::from_raw(origin_col as u64 + 1),
            origin_col,
            cols,
        }
    }

    fn line(text: &str) -> Vec<crate::terminal_grid::TerminalCell> {
        text.chars()
            .map(|ch| crate::terminal_grid::TerminalCell {
                text: ch.to_string().into(),
                ..crate::terminal_grid::TerminalCell::blank()
            })
            .collect()
    }

    fn input_content(text: &str, cursor_col: i32) -> RenderableContentOwned {
        use crate::terminal_grid::{
            MouseMode, TerminalColors, TerminalContent, TerminalLineMetadata,
        };
        TerminalContent {
            lines: vec![line(text)],
            line_metadata: vec![TerminalLineMetadata::default()],
            cursor_line: 0,
            cursor_col,
            cursor_visible: true,
            display_offset: 0,
            colors: TerminalColors::default(),
            mouse: MouseMode::default(),
        }
    }

    fn left_arrows(count: usize) -> Vec<u8> {
        b"\x1b[D".repeat(count)
    }

    fn right_arrows(count: usize) -> Vec<u8> {
        b"\x1b[C".repeat(count)
    }

    fn command_block_content() -> RenderableContentOwned {
        command_block_content_with_active_input("❯ ")
    }

    fn command_block_content_with_active_input(active_input: &str) -> RenderableContentOwned {
        use crate::terminal_grid::{
            MouseMode, TerminalColors, TerminalContent, TerminalLineMetadata,
        };
        TerminalContent {
            lines: vec![
                line("❯ printf block"),
                line("BLOCK_OUT_1"),
                line("BLOCK_OUT_2"),
                line(active_input),
            ],
            line_metadata: vec![
                TerminalLineMetadata {
                    semantic_prompt: TerminalSemanticPrompt::Prompt,
                    ..TerminalLineMetadata::default()
                },
                TerminalLineMetadata::default(),
                TerminalLineMetadata::default(),
                TerminalLineMetadata {
                    semantic_prompt: TerminalSemanticPrompt::Prompt,
                    ..TerminalLineMetadata::default()
                },
            ],
            cursor_line: 3,
            cursor_col: 2,
            cursor_visible: true,
            display_offset: 0,
            colors: TerminalColors::default(),
            mouse: MouseMode::default(),
        }
    }

    fn semantic_prompt_with_active_input() -> RenderableContentOwned {
        use crate::terminal_grid::{
            MouseMode, TerminalColors, TerminalContent, TerminalLineMetadata,
        };
        TerminalContent {
            lines: vec![
                line("~/Documents/Projects/Chelotype on main"),
                line("❯ ls -la"),
            ],
            line_metadata: vec![
                TerminalLineMetadata {
                    semantic_prompt: TerminalSemanticPrompt::Prompt,
                    ..TerminalLineMetadata::default()
                },
                TerminalLineMetadata {
                    semantic_prompt: TerminalSemanticPrompt::Continuation,
                    ..TerminalLineMetadata::default()
                },
            ],
            cursor_line: 1,
            cursor_col: 8,
            cursor_visible: true,
            display_offset: 0,
            colors: TerminalColors::default(),
            mouse: MouseMode::default(),
        }
    }

    #[test]
    fn command_block_rail_click_selects_block_output() {
        let content = command_block_content();
        let target = PointerPanePosition {
            pane: None,
            position: MouseGridPosition { row: 1, column: 0 },
        };

        assert_eq!(
            command_block_output_selection_at_rail(&content, metrics(), target, 3.0),
            Some(SelectionRange::new(
                GridPoint { row: 1, column: 0 },
                GridPoint { row: 2, column: 11 },
            ))
        );
    }

    #[test]
    fn command_block_rail_click_ignores_text_cell_body() {
        let content = command_block_content();
        let target = PointerPanePosition {
            pane: None,
            position: MouseGridPosition { row: 1, column: 0 },
        };

        assert_eq!(
            command_block_output_selection_at_rail(&content, metrics(), target, 8.0),
            None
        );
    }

    #[test]
    fn command_block_rail_context_extracts_block_output_text() {
        let content = command_block_content();
        let target = PointerPanePosition {
            pane: None,
            position: MouseGridPosition { row: 2, column: 0 },
        };

        assert_eq!(
            command_block_output_text_at_rail(&content, metrics(), target, 3.0).as_deref(),
            Some("BLOCK_OUT_1\nBLOCK_OUT_2")
        );
    }

    #[test]
    fn preedit_overlay_uses_terminal_grid_columns_for_wide_and_combining_text() {
        let mut frame = Renderer::render_frame_with_selection(&command_block_content(), None);
        apply_preedit_to_render_frame(
            &mut frame,
            Some(&PendingPreedit {
                text: "a中e\u{0301}".to_string(),
                cursor: 2,
            }),
        );

        let preedit = frame.preedit.expect("preedit render state");
        assert_eq!(preedit.text, "a中e\u{0301}");
        assert_eq!(preedit.cursor, 2);
        assert_eq!(preedit.columns, 4);
        assert_eq!(preedit.cursor_columns, 3);
        assert_eq!(preedit.line, 3);
        assert_eq!(preedit.column, 2);
    }

    #[test]
    fn preedit_overlay_cursor_columns_do_not_advance_for_combining_mark() {
        let mut frame = Renderer::render_frame_with_selection(&command_block_content(), None);
        let text = "a中e\u{0301}";

        apply_preedit_to_render_frame(
            &mut frame,
            Some(&PendingPreedit {
                text: text.to_string(),
                cursor: 3,
            }),
        );
        let before_combining = frame
            .preedit
            .as_ref()
            .expect("preedit before combining")
            .cursor_columns;

        apply_preedit_to_render_frame(
            &mut frame,
            Some(&PendingPreedit {
                text: text.to_string(),
                cursor: 4,
            }),
        );
        let after_combining = frame
            .preedit
            .as_ref()
            .expect("preedit after combining")
            .cursor_columns;

        assert_eq!(before_combining, 4);
        assert_eq!(after_combining, 4);
    }

    #[test]
    fn captured_pane_keeps_drag_coordinates_local_after_crossing_split_boundary() {
        let left = pane(0, 40);
        let right = pane(40, 40);
        let target = pointer_grid_position_for_capture_or_panes(
            metrics(),
            Some(left),
            &[left, right],
            430.0,
            50.0,
        )
        .expect("captured pointer position");

        assert_eq!(target.pane.expect("pane").id, left.id);
        assert_eq!(target.position.column, 39);
        assert_eq!(target.position.row, 2);
    }

    #[test]
    fn uncaptured_pointer_uses_pane_under_coordinates() {
        let left = pane(0, 40);
        let right = pane(40, 40);
        let target = pointer_grid_position_for_capture_or_panes(
            metrics(),
            None,
            &[left, right],
            430.0,
            50.0,
        )
        .expect("uncaptured pointer position");

        assert_eq!(target.pane.expect("pane").id, right.id);
        assert_eq!(target.position.column, 3);
        assert_eq!(target.position.row, 2);
    }

    #[test]
    fn split_resize_boundary_hit_test_uses_rendered_pane_edges() {
        let left = pane(0, 40);
        let right = pane(40, 40);

        assert_eq!(
            split_resize_boundary_at(metrics(), &[left, right], 400.0),
            Some(0)
        );
        assert_eq!(
            split_resize_boundary_at(metrics(), &[left, right], 403.0),
            Some(0)
        );
        assert_eq!(
            split_resize_boundary_at(metrics(), &[left, right], 407.0),
            Some(0)
        );
        assert_eq!(
            split_resize_boundary_at(metrics(), &[left, right], 409.0),
            None
        );
    }

    #[test]
    fn click_gesture_cancel_does_not_abort_active_drag_selection() {
        let active = std::cell::Cell::new(true);
        let left_down = std::cell::Cell::new(true);
        let capture = std::cell::Cell::new(Some(pane(0, 80)));
        let interaction = std::cell::RefCell::new(PointerInteraction::default());

        let effects = cancel_click_gesture(&active, &left_down, &capture, &interaction);

        assert!(effects.is_empty());
        assert!(left_down.get());
        assert!(capture.get().is_some());
    }

    #[test]
    fn remembers_only_single_container_tab_on_close() {
        let container = LaunchTarget::Toolbox {
            name: "fedora-toolbox-latest".to_string(),
        };
        let mut workspace = TerminalWorkspace::spawn_titled_with_target(
            "fedora-toolbox-latest",
            portable_pty::CommandBuilder::new("/bin/sh"),
            container.clone(),
        )
        .expect("spawn workspace");

        assert_eq!(
            single_tab_launch_target_to_remember(&workspace),
            Some(container.clone())
        );

        let host = workspace
            .add_titled_tab_with("Host", portable_pty::CommandBuilder::new("/bin/sh"))
            .expect("spawn host tab");
        assert_eq!(single_tab_launch_target_to_remember(&workspace), None);

        let _ = workspace.write_active(b"exit\n");
        let _ = workspace.activate(host);
        let _ = workspace.write_active(b"exit\n");
    }
}
