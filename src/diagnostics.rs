use crate::backend::{ScreenSize, TerminalBackend, headless_shell_command};
use crate::cell_text::lines_to_text;
use crate::command_blocks::CommandBlock;
use crate::input::{KeyAction, key_to_action};
use crate::interaction::{
    InteractionEffect, PointerInteraction, cursor_movement_bytes_for_content,
};
use crate::mouse::{MouseButton, MouseGridPosition};
use crate::render::{RenderLine, Renderer};
use crate::selection::{GridPoint, SelectionRange, selected_text_with_metadata};
use crate::snapshot::{write_snapshot, write_workspace_render_snapshot};
use crate::terminal_palette::default_terminal_palette;
use crate::workspace::TerminalWorkspace;
use crate::workspace_render::{WorkspaceRenderFrame, WorkspaceRenderLayout};
use gtk::gdk;
use gtk::glib;
use serde::Serialize;
use std::fs::File;
use std::fs::create_dir_all;
use std::io::Write;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::{Duration, Instant};

pub fn run_headless() -> glib::ExitCode {
    match run_headless_scenario() {
        Ok(_) => glib::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("headless error: {err}");
            glib::ExitCode::from(1)
        }
    }
}

pub fn run_headless_scenario() -> std::io::Result<PathBuf> {
    let scenario = HeadlessScenario::from_env();
    if scenario.needs_workspace() {
        return run_headless_workspace_scenario(scenario);
    }
    let mut backend = TerminalBackend::spawn_headless_shell()?;
    let mut runtime = HeadlessRuntime::default();
    for action in &scenario.actions {
        runtime.apply(&mut backend, action)?;
        sleep(Duration::from_millis(scenario.step_delay_ms));
    }
    let content = wait_for_headless_content(&mut backend, &scenario.expected)?;
    let selection = scenario.selection.or(runtime.selection);
    let rendered = Renderer::render_frame_with_selection(&content, selection);
    let path = write_snapshot(&content, "headless")
        .ok_or_else(|| std::io::Error::other("snapshot write failed"))?;
    write_render_dump(
        path.clone(),
        &rendered.history_markup,
        &rendered.input_markup,
        &rendered.lines,
        &rendered.command_blocks,
        selection.map(|selection| {
            selected_text_with_metadata(&content.lines, &content.line_metadata, selection)
        }),
        selection,
    )?;
    Ok(path)
}

fn run_headless_workspace_scenario(scenario: HeadlessScenario) -> std::io::Result<PathBuf> {
    let mut workspace = TerminalWorkspace::spawn_with(headless_shell_command())?;
    let mut runtime = WorkspaceHeadlessRuntime::default();
    for action in &scenario.actions {
        runtime.apply(&mut workspace, action)?;
        sleep(Duration::from_millis(scenario.step_delay_ms));
    }
    let panes = wait_for_workspace_content(&mut workspace, &scenario.expected)?;
    let selection = scenario.selection.or(runtime.selection);
    let layout = runtime.workspace_render_layout(&workspace);
    let rendered =
        WorkspaceRenderFrame::from_active_tab_panes_with_layout(panes, selection, layout);
    write_workspace_render_dump(rendered)
}

struct HeadlessScenario {
    actions: Vec<HeadlessAction>,
    expected: Vec<String>,
    step_delay_ms: u64,
    selection: Option<SelectionRange>,
}

impl HeadlessScenario {
    fn from_env() -> Self {
        let actions = std::env::var("CHELOTYPE_HEADLESS_EVENTS")
            .ok()
            .map(|value| value.split('|').map(parse_headless_action).collect())
            .or_else(|| {
                std::env::var("CHELOTYPE_HEADLESS_ACTIONS")
                    .ok()
                    .map(|value| {
                        value
                            .split('|')
                            .map(|action| HeadlessAction::Write(decode_action(action)))
                            .collect()
                    })
            })
            .unwrap_or_else(default_actions);
        let expected = std::env::var("CHELOTYPE_HEADLESS_EXPECT")
            .ok()
            .map(|value| value.split('|').map(ToOwned::to_owned).collect())
            .unwrap_or_else(default_expected);
        let step_delay_ms = std::env::var("CHELOTYPE_HEADLESS_STEP_MS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(120);
        let selection = std::env::var("CHELOTYPE_HEADLESS_SELECTION")
            .ok()
            .and_then(|value| parse_selection_range(&value));
        Self {
            actions,
            expected,
            step_delay_ms,
            selection,
        }
    }

    fn needs_workspace(&self) -> bool {
        self.actions.iter().any(HeadlessAction::needs_workspace)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum HeadlessAction {
    Write(String),
    Key(HeadlessKey),
    Resize(ScreenSize),
    Scroll(i32),
    Wait(String),
    Split,
    PaneNext,
    PanePrevious,
    PaneResize {
        boundary_index: usize,
        delta_cols: i16,
        size: ScreenSize,
    },
    MousePress(MouseButton, MouseGridPosition),
    MouseDrag(MouseGridPosition),
    MouseRelease(MouseGridPosition),
}

impl HeadlessAction {
    fn needs_workspace(&self) -> bool {
        match self {
            Self::Split | Self::PaneNext | Self::PanePrevious | Self::PaneResize { .. } => true,
            Self::Key(key) => matches!(
                key_to_action(key.key, 0, key.modifiers),
                Some(
                    KeyAction::NewTab
                        | KeyAction::NextTab
                        | KeyAction::PreviousTab
                        | KeyAction::SplitPane
                )
            ),
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HeadlessKey {
    key: gdk::Key,
    modifiers: gdk::ModifierType,
}

#[derive(Default)]
struct HeadlessRuntime {
    pointer: PointerInteraction,
    selection: Option<SelectionRange>,
}

#[derive(Default)]
struct WorkspaceHeadlessRuntime {
    pointer: PointerInteraction,
    selection: Option<SelectionRange>,
    layout_size: Option<ScreenSize>,
}

impl WorkspaceHeadlessRuntime {
    fn apply(
        &mut self,
        workspace: &mut TerminalWorkspace,
        action: &HeadlessAction,
    ) -> std::io::Result<()> {
        match action {
            HeadlessAction::Write(text) => workspace.write_active(text.as_bytes()),
            HeadlessAction::Key(key) => {
                if let Some(action) = key_to_action(key.key, 0, key.modifiers) {
                    match action {
                        KeyAction::Write(bytes) => workspace.write_active(&bytes),
                        KeyAction::CursorMove {
                            direction,
                            unit: crate::input::CursorUnit::Cell,
                            selecting: false,
                        } => workspace.write_active(match direction {
                            crate::input::CursorDirection::Left => b"\x1b[D",
                            crate::input::CursorDirection::Right => b"\x1b[C",
                        }),
                        KeyAction::CursorMove { .. }
                        | KeyAction::SelectInput
                        | KeyAction::SelectCommandBlockOutput(_) => Ok(()),
                        KeyAction::ScrollDisplay(lines) => workspace.scroll_active(lines),
                        KeyAction::NewTab => {
                            workspace.add_tab_with(headless_shell_command())?;
                            Ok(())
                        }
                        KeyAction::CloseTab => {
                            let active = workspace.active_tab_id();
                            let _ = workspace.close(active);
                            Ok(())
                        }
                        KeyAction::NextTab => {
                            workspace.activate_next();
                            Ok(())
                        }
                        KeyAction::PreviousTab => {
                            workspace.activate_previous();
                            Ok(())
                        }
                        KeyAction::SplitPane => {
                            workspace.split_active_with(headless_shell_command())?;
                            Ok(())
                        }
                        KeyAction::CopySelection
                        | KeyAction::CutSelection
                        | KeyAction::PasteClipboard
                        | KeyAction::UndoInput
                        | KeyAction::RedoInput
                        | KeyAction::ZoomIn
                        | KeyAction::ZoomOut
                        | KeyAction::ZoomReset
                        | KeyAction::OpenSettings
                        | KeyAction::NewWindow
                        | KeyAction::OpenAbout => Ok(()),
                    }
                } else {
                    Ok(())
                }
            }
            HeadlessAction::Resize(size) => {
                self.layout_size = Some(*size);
                workspace.resize_active_tab(*size)
            }
            HeadlessAction::Scroll(lines) => workspace.scroll_active(*lines),
            HeadlessAction::Wait(expected) => {
                wait_for_workspace_content(workspace, std::slice::from_ref(expected)).map(|_| ())
            }
            HeadlessAction::Split => {
                workspace.split_active_with(headless_shell_command())?;
                Ok(())
            }
            HeadlessAction::PaneNext => {
                workspace.activate_next_pane();
                Ok(())
            }
            HeadlessAction::PanePrevious => {
                workspace.activate_previous_pane();
                Ok(())
            }
            HeadlessAction::PaneResize {
                boundary_index,
                delta_cols,
                size,
            } => workspace
                .resize_active_tab_split(*boundary_index, *delta_cols, *size)
                .map(|_| {
                    self.layout_size = Some(*size);
                }),
            HeadlessAction::MousePress(button, position) => {
                let mouse_mode = workspace
                    .snapshot_active_renderable()
                    .map(|snapshot| snapshot.mouse)
                    .unwrap_or_default();
                let effects = self.pointer.press(mouse_mode, *button, *position);
                self.apply_interaction_effects(workspace, effects)
            }
            HeadlessAction::MouseDrag(position) => {
                let mouse_mode = workspace
                    .snapshot_active_renderable()
                    .map(|snapshot| snapshot.mouse)
                    .unwrap_or_default();
                let effects = self.pointer.motion(mouse_mode, *position);
                self.apply_interaction_effects(workspace, effects)
            }
            HeadlessAction::MouseRelease(position) => {
                let mouse_mode = workspace
                    .snapshot_active_renderable()
                    .map(|snapshot| snapshot.mouse)
                    .unwrap_or_default();
                let effects = self.pointer.release(mouse_mode, *position);
                self.apply_interaction_effects(workspace, effects)
            }
        }
    }

    fn workspace_render_layout(
        &self,
        workspace: &TerminalWorkspace,
    ) -> Option<WorkspaceRenderLayout> {
        let size = self.layout_size?;
        Some(WorkspaceRenderLayout {
            cols: usize::from(size.cols),
            rows: usize::from(size.rows),
            pane_cols: workspace
                .active_tab_pane_columns(size.cols)
                .into_iter()
                .map(usize::from)
                .collect(),
        })
    }

    fn apply_interaction_effects(
        &mut self,
        workspace: &mut TerminalWorkspace,
        effects: Vec<InteractionEffect>,
    ) -> std::io::Result<()> {
        for effect in effects {
            match effect {
                InteractionEffect::Write(bytes) => workspace.write_active(&bytes)?,
                InteractionEffect::SelectionChanged(selection) => self.selection = selection,
                InteractionEffect::MoveCursorTo(position) => {
                    if let Some(snapshot) = workspace.snapshot_active_renderable()
                        && let Some(bytes) = cursor_movement_bytes_for_content(&snapshot, position)
                    {
                        workspace.write_active(&bytes)?;
                    }
                }
            }
        }
        Ok(())
    }
}

impl HeadlessRuntime {
    fn apply(
        &mut self,
        backend: &mut TerminalBackend,
        action: &HeadlessAction,
    ) -> std::io::Result<()> {
        match action {
            HeadlessAction::Write(text) => backend.write(text.as_bytes()),
            HeadlessAction::Key(key) => {
                if let Some(action) = key_to_action(key.key, 0, key.modifiers) {
                    match action {
                        KeyAction::Write(bytes) => backend.write(&bytes),
                        KeyAction::CursorMove {
                            direction,
                            unit: crate::input::CursorUnit::Cell,
                            selecting: false,
                        } => backend.write(match direction {
                            crate::input::CursorDirection::Left => b"\x1b[D",
                            crate::input::CursorDirection::Right => b"\x1b[C",
                        }),
                        KeyAction::CursorMove { .. }
                        | KeyAction::SelectInput
                        | KeyAction::SelectCommandBlockOutput(_) => Ok(()),
                        KeyAction::ScrollDisplay(lines) => backend.scroll_display(lines),
                        KeyAction::CopySelection
                        | KeyAction::CutSelection
                        | KeyAction::PasteClipboard
                        | KeyAction::UndoInput
                        | KeyAction::RedoInput
                        | KeyAction::NewTab
                        | KeyAction::CloseTab
                        | KeyAction::NextTab
                        | KeyAction::PreviousTab
                        | KeyAction::SplitPane
                        | KeyAction::ZoomIn
                        | KeyAction::ZoomOut
                        | KeyAction::ZoomReset
                        | KeyAction::OpenSettings
                        | KeyAction::NewWindow
                        | KeyAction::OpenAbout => Ok(()),
                    }
                } else {
                    Ok(())
                }
            }
            HeadlessAction::Resize(size) => backend.resize(*size),
            HeadlessAction::Scroll(lines) => backend.scroll_display(*lines),
            HeadlessAction::Wait(expected) => {
                wait_for_headless_content(backend, std::slice::from_ref(expected)).map(|_| ())
            }
            HeadlessAction::Split
            | HeadlessAction::PaneNext
            | HeadlessAction::PanePrevious
            | HeadlessAction::PaneResize { .. } => Ok(()),
            HeadlessAction::MousePress(button, position) => {
                let mouse_mode = backend
                    .snapshot_renderable()
                    .map(|snapshot| snapshot.mouse)
                    .unwrap_or_default();
                let effects = self.pointer.press(mouse_mode, *button, *position);
                self.apply_interaction_effects(backend, effects)
            }
            HeadlessAction::MouseDrag(position) => {
                let mouse_mode = backend
                    .snapshot_renderable()
                    .map(|snapshot| snapshot.mouse)
                    .unwrap_or_default();
                let effects = self.pointer.motion(mouse_mode, *position);
                self.apply_interaction_effects(backend, effects)
            }
            HeadlessAction::MouseRelease(position) => {
                let mouse_mode = backend
                    .snapshot_renderable()
                    .map(|snapshot| snapshot.mouse)
                    .unwrap_or_default();
                let effects = self.pointer.release(mouse_mode, *position);
                self.apply_interaction_effects(backend, effects)
            }
        }
    }

    fn apply_interaction_effects(
        &mut self,
        backend: &mut TerminalBackend,
        effects: Vec<InteractionEffect>,
    ) -> std::io::Result<()> {
        for effect in effects {
            match effect {
                InteractionEffect::Write(bytes) => backend.write(&bytes)?,
                InteractionEffect::SelectionChanged(selection) => self.selection = selection,
                InteractionEffect::MoveCursorTo(position) => {
                    if let Some(snapshot) = backend.snapshot_renderable()
                        && let Some(bytes) = cursor_movement_bytes_for_content(&snapshot, position)
                    {
                        backend.write(&bytes)?;
                    }
                }
            }
        }
        Ok(())
    }
}

fn default_actions() -> Vec<HeadlessAction> {
    [
        "printf '\\nchelotype headless snapshot\\n'\n",
        "echo prompt>$PS1\n",
        "echo color-test && printf '\\e[31mred\\e[0m \\e[32mgreen\\e[0m \\e[34mblue\\e[0m\\n'\n",
    ]
    .into_iter()
    .map(|action| HeadlessAction::Write(action.to_owned()))
    .collect()
}

fn default_expected() -> Vec<String> {
    [
        "chelotype headless snapshot",
        "color-test",
        "red green blue",
    ]
    .into_iter()
    .map(ToOwned::to_owned)
    .collect()
}

fn parse_headless_action(input: &str) -> HeadlessAction {
    if let Some(text) = input.strip_prefix("raw:") {
        return HeadlessAction::Write(decode_action(text));
    }
    if let Some(text) = input.strip_prefix("text:") {
        return HeadlessAction::Write(decode_action(text));
    }
    if let Some(key) = input.strip_prefix("key:")
        && let Some(key) = parse_headless_key(key)
    {
        return HeadlessAction::Key(key);
    }
    if let Some(size) = input.strip_prefix("resize:")
        && let Some(size) = parse_screen_size(size)
    {
        return HeadlessAction::Resize(size);
    }
    if let Some(lines) = input.strip_prefix("scroll:")
        && let Ok(lines) = lines.parse()
    {
        return HeadlessAction::Scroll(lines);
    }
    if let Some(expected) = input.strip_prefix("wait:") {
        return HeadlessAction::Wait(decode_action(expected));
    }
    if input == "split" {
        return HeadlessAction::Split;
    }
    if input == "pane:next" {
        return HeadlessAction::PaneNext;
    }
    if input == "pane:previous" {
        return HeadlessAction::PanePrevious;
    }
    if let Some(resize) = input.strip_prefix("pane:resize:")
        && let Some(action) = parse_pane_resize(resize)
    {
        return action;
    }
    if let Some(mouse) = input.strip_prefix("mouse:")
        && let Some(action) = parse_mouse_action(mouse)
    {
        return action;
    }
    HeadlessAction::Write(decode_action(input))
}

fn parse_headless_key(input: &str) -> Option<HeadlessKey> {
    let mut modifiers = gdk::ModifierType::empty();
    let mut key = input;
    for prefix in ["Ctrl+", "Alt+", "Shift+"] {
        if let Some(stripped) = key.strip_prefix(prefix) {
            match prefix {
                "Ctrl+" => modifiers |= gdk::ModifierType::CONTROL_MASK,
                "Alt+" => modifiers |= gdk::ModifierType::ALT_MASK,
                "Shift+" => modifiers |= gdk::ModifierType::SHIFT_MASK,
                _ => {}
            }
            key = stripped;
        }
    }
    let key = match key {
        "Enter" | "Return" => gdk::Key::Return,
        "Backspace" => gdk::Key::BackSpace,
        "Tab" => gdk::Key::Tab,
        "Left" => gdk::Key::Left,
        "Right" => gdk::Key::Right,
        "Up" => gdk::Key::Up,
        "Down" => gdk::Key::Down,
        "Home" => gdk::Key::Home,
        "End" => gdk::Key::End,
        "Delete" => gdk::Key::Delete,
        "PageUp" => gdk::Key::Page_Up,
        "PageDown" => gdk::Key::Page_Down,
        "a" => gdk::Key::a,
        "b" => gdk::Key::b,
        "c" => gdk::Key::c,
        "d" => gdk::Key::d,
        "e" => gdk::Key::e,
        "f" => gdk::Key::f,
        "x" => gdk::Key::x,
        "я" => gdk::Key::Cyrillic_ya,
        _ => return None,
    };
    Some(HeadlessKey { key, modifiers })
}

fn parse_screen_size(input: &str) -> Option<ScreenSize> {
    let (cols, rows) = input.split_once('x')?;
    ScreenSize::new(cols.parse().ok()?, rows.parse().ok()?).ok()
}

fn parse_pane_resize(input: &str) -> Option<HeadlessAction> {
    let mut parts = input.split(':');
    Some(HeadlessAction::PaneResize {
        boundary_index: parts.next()?.parse().ok()?,
        delta_cols: parts.next()?.parse().ok()?,
        size: parse_screen_size(parts.next()?)?,
    })
}

fn parse_mouse_action(input: &str) -> Option<HeadlessAction> {
    let mut parts = input.split(':');
    match parts.next()? {
        "press" => Some(HeadlessAction::MousePress(
            parse_mouse_button(parts.next()?)?,
            parse_mouse_position(parts.next()?)?,
        )),
        "drag" => Some(HeadlessAction::MouseDrag(parse_mouse_position(
            parts.next()?,
        )?)),
        "release" => Some(HeadlessAction::MouseRelease(parse_mouse_position(
            parts.next()?,
        )?)),
        _ => None,
    }
}

fn parse_mouse_button(input: &str) -> Option<MouseButton> {
    match input {
        "left" => Some(MouseButton::Left),
        "middle" => Some(MouseButton::Middle),
        "right" => Some(MouseButton::Right),
        _ => None,
    }
}

fn parse_mouse_position(input: &str) -> Option<MouseGridPosition> {
    let (column, row) = input.split_once(',')?;
    Some(MouseGridPosition {
        column: column.parse().ok()?,
        row: row.parse().ok()?,
    })
}

fn decode_action(input: &str) -> String {
    let mut output = String::new();
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => output.push('\n'),
                Some('r') => output.push('\r'),
                Some('t') => output.push('\t'),
                Some('e') => output.push('\x1b'),
                Some('x') => {
                    let first = chars.next();
                    let second = chars.next();
                    match (first, second) {
                        (Some(first), Some(second)) => {
                            let value = [first, second].iter().collect::<String>();
                            if let Ok(byte) = u8::from_str_radix(&value, 16) {
                                output.push(byte as char);
                            } else {
                                output.push_str("\\x");
                                output.push(first);
                                output.push(second);
                            }
                        }
                        (Some(first), None) => {
                            output.push_str("\\x");
                            output.push(first);
                        }
                        _ => output.push_str("\\x"),
                    }
                }
                Some('\\') => output.push('\\'),
                Some(other) => {
                    output.push('\\');
                    output.push(other);
                }
                None => output.push('\\'),
            }
        } else {
            output.push(ch);
        }
    }
    output
}

fn wait_for_headless_content(
    backend: &mut TerminalBackend,
    expected: &[String],
) -> std::io::Result<crate::backend::RenderableContentOwned> {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut last_text = String::new();
    while Instant::now() < deadline {
        if let Some(content) = backend.snapshot_renderable() {
            let text = lines_to_text(&content.lines);
            if expected.iter().all(|needle| text.contains(needle)) {
                return Ok(content);
            }
            last_text = text;
        }
        sleep(Duration::from_millis(20));
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        format!("headless content did not appear; expected={expected:?}; last={last_text:?}"),
    ))
}

fn wait_for_workspace_content(
    workspace: &mut TerminalWorkspace,
    expected: &[String],
) -> std::io::Result<Vec<crate::workspace::PaneRenderable>> {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut last_text = String::new();
    while Instant::now() < deadline {
        let panes = workspace.snapshot_active_tab_renderables();
        let text = panes
            .iter()
            .map(|pane| lines_to_text(&pane.content.lines))
            .collect::<Vec<_>>()
            .join("\n");
        if expected.iter().all(|needle| text.contains(needle)) {
            return Ok(panes);
        }
        last_text = text;
        sleep(Duration::from_millis(20));
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        format!(
            "workspace headless content did not appear; expected={expected:?}; last={last_text:?}"
        ),
    ))
}

fn write_workspace_render_dump(rendered: WorkspaceRenderFrame) -> std::io::Result<PathBuf> {
    write_workspace_render_snapshot(&rendered, "headless_workspace")
        .ok_or_else(|| std::io::Error::other("workspace render snapshot write failed"))
}

fn write_render_dump(
    base: PathBuf,
    history: &str,
    input: &str,
    lines: &[RenderLine],
    command_blocks: &[CommandBlock],
    selected_text: Option<String>,
    selection: Option<SelectionRange>,
) -> std::io::Result<()> {
    let dump = RenderDump {
        history_markup: history,
        input_markup: input,
        lines,
        command_blocks,
        selected_text: selected_text.as_deref(),
        selection,
    };
    let json = serde_json::to_vec_pretty(&dump)?;
    let input_html = if input.is_empty() {
        String::new()
    } else {
        format!("\n{input}")
    };
    let palette = default_terminal_palette();
    let html = format!(
        "<html><body style=\"background:{};color:{};font-family:'Source Code Pro',monospace;font-size:13px;white-space:pre;\">{history}{input_html}</body></html>",
        palette.background, palette.foreground
    );
    if let Some(parent) = base.parent() {
        create_dir_all(parent)?;
    }
    let mut json_file = File::create(base.with_extension("render.json"))?;
    json_file.write_all(&json)?;
    let mut file = File::create(base.with_extension("markup.html"))?;
    file.write_all(html.as_bytes())
}

#[derive(Serialize)]
struct RenderDump<'a> {
    history_markup: &'a str,
    input_markup: &'a str,
    lines: &'a [RenderLine],
    command_blocks: &'a [CommandBlock],
    selected_text: Option<&'a str>,
    selection: Option<SelectionRange>,
}

fn parse_selection_range(input: &str) -> Option<SelectionRange> {
    let (start, end) = input.split_once(':')?;
    Some(SelectionRange::between_cells(
        parse_grid_point(start)?,
        parse_grid_point(end)?,
    ))
}

fn parse_grid_point(input: &str) -> Option<GridPoint> {
    let (row, column) = input.split_once(',')?;
    Some(GridPoint {
        row: row.parse().ok()?,
        column: column.parse().ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_headless_selection_as_inclusive_cell_drag() {
        assert_eq!(
            parse_selection_range("2,3:2,5"),
            Some(SelectionRange::new(
                GridPoint { row: 2, column: 3 },
                GridPoint { row: 2, column: 6 }
            ))
        );
    }

    #[test]
    fn rejects_invalid_headless_selection() {
        assert_eq!(parse_selection_range("2,3"), None);
        assert_eq!(parse_selection_range("x,3:2,5"), None);
    }

    #[test]
    fn parses_headless_keyboard_event() {
        assert_eq!(
            parse_headless_action("key:Ctrl+d"),
            HeadlessAction::Key(HeadlessKey {
                key: gdk::Key::d,
                modifiers: gdk::ModifierType::CONTROL_MASK,
            })
        );
    }

    #[test]
    fn parses_headless_workspace_events() {
        assert_eq!(parse_headless_action("split"), HeadlessAction::Split);
        assert_eq!(parse_headless_action("pane:next"), HeadlessAction::PaneNext);
        assert_eq!(
            parse_headless_action("pane:previous"),
            HeadlessAction::PanePrevious
        );
        assert_eq!(
            parse_headless_action("pane:resize:0:20:81x12"),
            HeadlessAction::PaneResize {
                boundary_index: 0,
                delta_cols: 20,
                size: ScreenSize::new(81, 12).expect("valid size"),
            }
        );
    }

    #[test]
    fn parses_headless_resize_and_mouse_events() {
        assert_eq!(
            parse_headless_action("resize:100x24"),
            HeadlessAction::Resize(ScreenSize::new(100, 24).expect("valid size"))
        );
        assert_eq!(
            parse_headless_action("mouse:press:left:3,2"),
            HeadlessAction::MousePress(MouseButton::Left, MouseGridPosition { column: 3, row: 2 })
        );
    }
}
