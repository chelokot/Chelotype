use crate::config::{CursorShape, CursorStyle};
use crate::render::{RenderFrame, RenderRun, RenderStyle};
use crate::terminal_font::{TerminalFontMetrics, layout_for, metrics_for_widget};
use crate::workspace_render::WorkspaceRenderFrame;
use gtk::cairo;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

const CURSOR_BLINK_PERIOD: Duration = Duration::from_millis(530);
const CURSOR_MOTION_DURATION: Duration = Duration::from_millis(145);
const CURSOR_MOTION_FRAME_DURATION: Duration = Duration::from_millis(8);
const COMMAND_BLOCK_RAIL: RgbU8 = RgbU8 {
    red: 46,
    green: 166,
    blue: 199,
};
const COMMAND_BLOCK_BG: RgbU8 = RgbU8 {
    red: 18,
    green: 23,
    blue: 31,
};

#[derive(Clone)]
pub struct TerminalCanvas {
    area: gtk::DrawingArea,
    render: Rc<RefCell<Option<CanvasRenderFrame>>>,
    cursor_blink: Rc<Cell<CursorBlinkState>>,
    cursor_motion: Rc<Cell<CursorMotionState>>,
    cursor_motion_frame_active: Rc<Cell<bool>>,
}

impl TerminalCanvas {
    pub fn new() -> Self {
        let area = gtk::DrawingArea::new();
        area.set_focusable(true);
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.set_margin_start(14);
        area.set_margin_end(10);
        area.set_margin_top(8);
        area.set_margin_bottom(12);
        area.add_css_class("term-canvas");
        area.set_cursor_from_name(Some("text"));

        let render = Rc::new(RefCell::new(None::<CanvasRenderFrame>));
        let cursor_blink = Rc::new(Cell::new(CursorBlinkState::default()));
        let cursor_motion = Rc::new(Cell::new(CursorMotionState::default()));
        let cursor_motion_frame_active = Rc::new(Cell::new(false));
        let draw_render = render.clone();
        let draw_cursor_blink = cursor_blink.clone();
        let draw_cursor_motion = cursor_motion.clone();
        area.set_draw_func(move |widget, context, width, height| {
            let started = Instant::now();
            let now = Instant::now();
            draw_background(context, width, height);
            if let Some(render) = draw_render.borrow().as_ref() {
                draw_canvas_render(
                    widget,
                    context,
                    render,
                    draw_cursor_blink.get(),
                    draw_cursor_motion.get(),
                    CursorOptions::from_config(),
                    now,
                );
            }
            crate::perf_trace::record_duration("gtk_paint", started.elapsed());
        });

        Self {
            area,
            render,
            cursor_blink,
            cursor_motion,
            cursor_motion_frame_active,
        }
    }

    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.area
    }

    pub fn set_render(&self, render: RenderFrame) {
        self.set_canvas_render(CanvasRenderFrame::Single(render));
    }

    pub fn set_workspace_render(&self, render: WorkspaceRenderFrame) {
        self.set_canvas_render(CanvasRenderFrame::Workspace(render));
    }

    fn set_canvas_render(&self, render: CanvasRenderFrame) {
        let mut current = self.render.borrow_mut();
        let now = Instant::now();
        let identity = render
            .active_cursor_identity()
            .unwrap_or_else(CursorIdentity::hidden);
        let previous_blink = self.cursor_blink.get();
        let previous_motion = self.cursor_motion.get();
        self.cursor_blink.set(previous_blink.sync(identity, now));
        let cursor_style = crate::config::cursor_style();
        self.cursor_motion
            .set(previous_motion.sync(identity, now, cursor_style));
        self.ensure_cursor_motion_timer(now);
        if current.as_ref() == Some(&render) {
            if previous_blink != self.cursor_blink.get()
                || previous_motion != self.cursor_motion.get()
                || self.cursor_motion.get().active(now)
            {
                self.area.queue_draw();
            }
            return;
        }
        *current = Some(render);
        self.area.queue_draw();
    }

    fn ensure_cursor_motion_timer(&self, now: Instant) {
        if !self.cursor_motion.get().active(now) || self.cursor_motion_frame_active.get() {
            return;
        }
        self.cursor_motion_frame_active.set(true);
        let area = self.area.downgrade();
        let cursor_motion = self.cursor_motion.clone();
        let cursor_motion_frame_active = self.cursor_motion_frame_active.clone();
        gtk::glib::timeout_add_local(CURSOR_MOTION_FRAME_DURATION, move || {
            let now = Instant::now();
            let Some(area) = area.upgrade() else {
                cursor_motion_frame_active.set(false);
                return gtk::glib::ControlFlow::Break;
            };
            area.queue_draw();
            if cursor_motion.get().active(now) {
                gtk::glib::ControlFlow::Continue
            } else {
                cursor_motion_frame_active.set(false);
                gtk::glib::ControlFlow::Break
            }
        });
    }

    pub fn tick_cursor_blink(&self) {
        let Some(render) = self.render.borrow().as_ref().cloned() else {
            return;
        };
        let now = Instant::now();
        let next = self
            .cursor_blink
            .get()
            .tick(render.active_cursor_visible(), now);
        if self.cursor_blink.get() != next || self.cursor_motion.get().active(now) {
            self.cursor_blink.set(next);
            self.area.queue_draw();
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CanvasRenderFrame {
    Single(RenderFrame),
    Workspace(WorkspaceRenderFrame),
}

impl CanvasRenderFrame {
    fn active_cursor_identity(&self) -> Option<CursorIdentity> {
        match self {
            Self::Single(render) => Some(CursorIdentity {
                pane_id: 0,
                line: render.cursor.line,
                column: render.cursor.column,
                visible: render.cursor.visible,
            }),
            Self::Workspace(render) => {
                render
                    .panes
                    .iter()
                    .find(|pane| pane.active)
                    .map(|pane| CursorIdentity {
                        pane_id: pane.pane_id,
                        line: pane.frame.cursor.line,
                        column: pane.frame.cursor.column,
                        visible: pane.frame.cursor.visible,
                    })
            }
        }
    }

    fn active_cursor_visible(&self) -> bool {
        self.active_cursor_identity()
            .is_some_and(|cursor| cursor.visible)
    }
}

impl Default for TerminalCanvas {
    fn default() -> Self {
        Self::new()
    }
}

fn draw_background(context: &cairo::Context, width: i32, height: i32) {
    context.set_source_rgb(15.0 / 255.0, 17.0 / 255.0, 21.0 / 255.0);
    context.rectangle(0.0, 0.0, width as f64, height as f64);
    let _ = context.fill();
}

fn draw_canvas_render(
    widget: &gtk::DrawingArea,
    context: &cairo::Context,
    render: &CanvasRenderFrame,
    cursor_blink: CursorBlinkState,
    cursor_motion: CursorMotionState,
    cursor_options: CursorOptions,
    now: Instant,
) {
    let Some(metrics) = metrics_for_widget(widget) else {
        return;
    };
    let cursor_paint = CursorPaintState {
        visible: cursor_blink.visible,
        pane_id: 0,
        motion: cursor_motion,
        options: cursor_options,
        now,
    };
    match render {
        CanvasRenderFrame::Single(render) => draw_render_frame(
            widget,
            context,
            render,
            cursor_paint.with_blink(cursor_blink.visible),
            metrics,
        ),
        CanvasRenderFrame::Workspace(render) => {
            draw_workspace_render(widget, context, render, cursor_paint, metrics)
        }
    }
}

fn draw_render_frame(
    widget: &gtk::DrawingArea,
    context: &cairo::Context,
    render: &RenderFrame,
    cursor_paint: CursorPaintState,
    metrics: TerminalFontMetrics,
) {
    let cell_width = metrics.cell_width;
    let line_height = metrics.line_height;
    draw_command_blocks(context, render, line_height, cell_width);
    for line in &render.lines {
        let top = line.row as f64 * line_height;
        for run in &line.runs {
            draw_run_background(context, run, cell_width, line_height, top);
        }
        for run in &line.runs {
            if run.text.trim().is_empty() {
                continue;
            }
            let left = run.start_column as f64 * cell_width;
            let layout = layout_for(widget, &run_markup(run));
            let _ = context.save();
            context.rectangle(left, top, run.columns as f64 * cell_width, line_height);
            context.clip();
            gtk::render_layout(&widget.style_context(), context, left, top, &layout);
            let _ = context.restore();
        }
    }

    if render.cursor.visible && cursor_paint.visible {
        if let Some(preedit) = &render.preedit {
            draw_preedit(widget, context, render, preedit, line_height, cell_width);
        } else {
            draw_cursor(
                context,
                render,
                cursor_paint.motion.for_pane(cursor_paint.pane_id),
                cursor_paint.options,
                cursor_paint.now,
                line_height,
                cell_width,
            );
        }
    } else if let Some(preedit) = &render.preedit {
        draw_preedit(widget, context, render, preedit, line_height, cell_width);
    }
}

fn draw_command_blocks(
    context: &cairo::Context,
    render: &RenderFrame,
    line_height: f64,
    cell_width: f64,
) {
    for block in &render.command_blocks {
        if block.end_row < block.prompt_start_row {
            continue;
        }
        let top = block.prompt_start_row as f64 * line_height;
        let height = (block.end_row - block.prompt_start_row + 1) as f64 * line_height;
        set_rgb(context, COMMAND_BLOCK_BG);
        context.rectangle(0.0, top, cell_width * 0.5, height);
        let _ = context.fill();
        set_rgb(context, COMMAND_BLOCK_RAIL);
        context.rectangle(0.0, top + 2.0, 2.0, (height - 4.0).max(1.0));
        let _ = context.fill();
    }
}

fn draw_workspace_render(
    widget: &gtk::DrawingArea,
    context: &cairo::Context,
    render: &WorkspaceRenderFrame,
    cursor_paint: CursorPaintState,
    metrics: TerminalFontMetrics,
) {
    let cell_width = metrics.cell_width;
    let line_height = metrics.line_height;
    for pane in &render.panes {
        let left = pane.origin_col as f64 * cell_width;
        let top = pane.origin_row as f64 * line_height;
        let width = pane.cols as f64 * cell_width;
        let height = pane.rows as f64 * line_height;
        let _ = context.save();
        context.rectangle(left, top, width, height);
        context.clip();
        context.translate(left, top);
        draw_render_frame(
            widget,
            context,
            &pane.frame,
            cursor_paint.for_pane(pane.pane_id, pane.active),
            metrics,
        );
        let _ = context.restore();
        if pane.index > 0 {
            draw_pane_separator(context, left, top, height);
        }
    }
}

fn draw_pane_separator(context: &cairo::Context, left: f64, top: f64, height: f64) {
    context.set_source_rgb(48.0 / 255.0, 51.0 / 255.0, 58.0 / 255.0);
    context.rectangle(left.round() - 1.0, top, 1.0, height);
    let _ = context.fill();
}

#[derive(Clone, Copy)]
struct RgbU8 {
    red: u8,
    green: u8,
    blue: u8,
}

#[derive(Clone, Copy)]
struct CursorPaintState {
    visible: bool,
    pane_id: u64,
    motion: CursorMotionState,
    options: CursorOptions,
    now: Instant,
}

#[derive(Clone, Copy)]
struct CursorOptions {
    style: CursorStyle,
    shape: CursorShape,
}

impl CursorOptions {
    fn from_config() -> Self {
        Self {
            style: crate::config::cursor_style(),
            shape: crate::config::cursor_shape(),
        }
    }
}

impl CursorPaintState {
    fn with_blink(self, visible: bool) -> Self {
        Self { visible, ..self }
    }

    fn for_pane(self, pane_id: u64, active: bool) -> Self {
        Self {
            visible: self.visible && active,
            pane_id,
            ..self
        }
    }
}

fn set_rgb(context: &cairo::Context, color: RgbU8) {
    context.set_source_rgb(
        f64::from(color.red) / 255.0,
        f64::from(color.green) / 255.0,
        f64::from(color.blue) / 255.0,
    );
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct CursorBlinkState {
    visible: bool,
    cursor: Option<CursorIdentity>,
    reset_at: Option<Instant>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CursorIdentity {
    pane_id: u64,
    line: i32,
    column: i32,
    visible: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CursorMotionState {
    initialized: bool,
    pane_id: u64,
    from_line: f64,
    from_column: f64,
    to_line: f64,
    to_column: f64,
    started_at: Option<Instant>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CursorDrawPosition {
    pane_id: u64,
    line: f64,
    column: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CursorDrawPath {
    from: CursorDrawPosition,
    current: CursorDrawPosition,
    target: CursorDrawPosition,
    progress: f64,
}

impl Default for CursorMotionState {
    fn default() -> Self {
        Self {
            initialized: false,
            pane_id: 0,
            from_line: 0.0,
            from_column: 0.0,
            to_line: 0.0,
            to_column: 0.0,
            started_at: None,
        }
    }
}

impl CursorBlinkState {
    fn sync(self, identity: CursorIdentity, now: Instant) -> Self {
        if !identity.visible {
            return Self {
                visible: false,
                cursor: Some(identity),
                reset_at: None,
            };
        }
        if self.cursor != Some(identity) || !self.visible {
            return Self {
                visible: true,
                cursor: Some(identity),
                reset_at: Some(now),
            };
        }
        self
    }

    fn tick(self, cursor_visible: bool, now: Instant) -> Self {
        if !cursor_visible {
            return Self {
                visible: false,
                cursor: self.cursor,
                reset_at: None,
            };
        }
        let reset_at = self.reset_at.unwrap_or(now);
        let elapsed_periods =
            now.saturating_duration_since(reset_at).as_millis() / CURSOR_BLINK_PERIOD.as_millis();
        Self {
            visible: elapsed_periods.is_multiple_of(2),
            cursor: self.cursor,
            reset_at: Some(reset_at),
        }
    }
}

impl CursorMotionState {
    fn sync(self, identity: CursorIdentity, now: Instant, style: CursorStyle) -> Self {
        let target = CursorDrawPosition {
            pane_id: identity.pane_id,
            line: f64::from(identity.line.max(0)),
            column: f64::from(identity.column.max(0)),
        };
        if style == CursorStyle::Steady || !identity.visible {
            return Self::settled(target);
        }
        let current = self.position(now);
        if current == Some(target) {
            return self;
        }
        let from = current.filter(|position| position.pane_id == target.pane_id);
        let from = from.unwrap_or(target);
        Self {
            initialized: true,
            pane_id: target.pane_id,
            from_line: from.line,
            from_column: from.column,
            to_line: target.line,
            to_column: target.column,
            started_at: Some(now),
        }
    }

    fn settled(target: CursorDrawPosition) -> Self {
        Self {
            initialized: true,
            pane_id: target.pane_id,
            from_line: target.line,
            from_column: target.column,
            to_line: target.line,
            to_column: target.column,
            started_at: None,
        }
    }

    fn position(self, now: Instant) -> Option<CursorDrawPosition> {
        if !self.initialized {
            return None;
        }
        let Some(progress) = self.progress(now) else {
            return Some(CursorDrawPosition {
                pane_id: self.pane_id,
                line: self.to_line,
                column: self.to_column,
            });
        };
        let eased = 1.0 - (1.0 - progress).powi(3);
        Some(CursorDrawPosition {
            pane_id: self.pane_id,
            line: self.from_line + ((self.to_line - self.from_line) * eased),
            column: self.from_column + ((self.to_column - self.from_column) * eased),
        })
    }

    fn path(self, now: Instant) -> Option<CursorDrawPath> {
        if !self.initialized {
            return None;
        }
        let target = CursorDrawPosition {
            pane_id: self.pane_id,
            line: self.to_line,
            column: self.to_column,
        };
        let current = self.position(now).unwrap_or(target);
        Some(CursorDrawPath {
            from: CursorDrawPosition {
                pane_id: self.pane_id,
                line: self.from_line,
                column: self.from_column,
            },
            current,
            target,
            progress: self.progress(now).unwrap_or(1.0),
        })
    }

    fn for_pane(self, pane_id: u64) -> Option<Self> {
        (self.initialized && self.pane_id == pane_id).then_some(self)
    }

    fn active(self, now: Instant) -> bool {
        self.progress(now).is_some_and(|progress| progress < 1.0)
    }

    fn progress(self, now: Instant) -> Option<f64> {
        let started_at = self.started_at?;
        let elapsed = now.saturating_duration_since(started_at);
        Some((elapsed.as_secs_f64() / CURSOR_MOTION_DURATION.as_secs_f64()).clamp(0.0, 1.0))
    }
}

impl CursorIdentity {
    fn hidden() -> Self {
        Self {
            pane_id: 0,
            line: 0,
            column: 0,
            visible: false,
        }
    }
}

fn draw_run_background(
    context: &cairo::Context,
    run: &RenderRun,
    cell_width: f64,
    line_height: f64,
    top: f64,
) {
    if let Some(color) = run.style.bg.as_deref().and_then(parse_hex_color) {
        context.set_source_rgb(color.red, color.green, color.blue);
        context.rectangle(
            run.start_column as f64 * cell_width,
            top,
            run.columns as f64 * cell_width,
            line_height,
        );
        let _ = context.fill();
    }
}

fn draw_cursor(
    context: &cairo::Context,
    render: &RenderFrame,
    cursor_motion: Option<CursorMotionState>,
    cursor_options: CursorOptions,
    now: Instant,
    line_height: f64,
    cell_width: f64,
) {
    let target = CursorDrawPosition {
        pane_id: 0,
        line: f64::from(render.cursor.line.max(0)),
        column: f64::from(render.cursor.column.max(0)),
    };
    let path = cursor_motion.and_then(|motion| motion.path(now));
    match cursor_options.style {
        CursorStyle::Steady => draw_caret_at(
            context,
            target,
            cursor_options.shape,
            line_height,
            cell_width,
        ),
        CursorStyle::Smooth => draw_caret_at(
            context,
            path.map(|path| path.current).unwrap_or(target),
            cursor_options.shape,
            line_height,
            cell_width,
        ),
        CursorStyle::Smear => {
            if let Some(path) = path {
                draw_smear_trail(context, path, line_height, cell_width, SmearPreset::Soft);
                draw_caret_at(
                    context,
                    path.current,
                    cursor_options.shape,
                    line_height,
                    cell_width,
                );
            } else {
                draw_caret_at(
                    context,
                    target,
                    cursor_options.shape,
                    line_height,
                    cell_width,
                );
            }
        }
        CursorStyle::Neovide => {
            if let Some(path) = path {
                draw_smear_trail(context, path, line_height, cell_width, SmearPreset::Neovide);
                draw_caret_at(
                    context,
                    path.current,
                    cursor_options.shape,
                    line_height,
                    cell_width,
                );
            } else {
                draw_caret_at(
                    context,
                    target,
                    cursor_options.shape,
                    line_height,
                    cell_width,
                );
            }
        }
    }
}

#[derive(Clone, Copy)]
enum SmearPreset {
    Soft,
    Neovide,
}

fn draw_smear_trail(
    context: &cairo::Context,
    path: CursorDrawPath,
    line_height: f64,
    cell_width: f64,
    preset: SmearPreset,
) {
    let distance_columns = (path.target.column - path.current.column).abs();
    let distance_lines = (path.target.line - path.current.line).abs();
    let distance = (distance_columns.powi(2) + distance_lines.powi(2)).sqrt();
    if distance < 0.25 || path.progress >= 1.0 {
        return;
    }
    let base_alpha = match preset {
        SmearPreset::Soft => 0.13,
        SmearPreset::Neovide => 0.24,
    };
    let width_cells = match preset {
        SmearPreset::Soft => 0.45,
        SmearPreset::Neovide => 0.72,
    };
    let alpha = base_alpha * (1.0 - path.progress).powf(0.35);
    context.set_source_rgba(
        125.0 / 255.0,
        211.0 / 255.0,
        252.0 / 255.0,
        alpha.clamp(0.0, 0.28),
    );
    if (path.current.line - path.target.line).abs() < 0.05 {
        let current_x = path.current.column * cell_width;
        let target_x = path.target.column * cell_width;
        let left = current_x.min(target_x);
        let right = current_x.max(target_x) + (cell_width * width_cells);
        let y = path.current.line * line_height;
        context.rectangle(
            left.round(),
            y.round(),
            (right - left).max(1.25),
            line_height,
        );
        let _ = context.fill();
        return;
    }
    let previous_line_cap = context.line_cap();
    let previous_line_join = context.line_join();
    let previous_line_width = context.line_width();
    context.set_line_cap(cairo::LineCap::Round);
    context.set_line_join(cairo::LineJoin::Round);
    context.set_line_width((cell_width * width_cells).max(2.5));
    context.move_to(
        (path.current.column * cell_width) + (cell_width * 0.5),
        (path.current.line * line_height) + (line_height * 0.5),
    );
    context.line_to(
        (path.target.column * cell_width) + (cell_width * 0.5),
        (path.target.line * line_height) + (line_height * 0.5),
    );
    let _ = context.stroke();
    context.set_line_cap(previous_line_cap);
    context.set_line_join(previous_line_join);
    context.set_line_width(previous_line_width);
}

fn draw_caret_at(
    context: &cairo::Context,
    position: CursorDrawPosition,
    shape: CursorShape,
    line_height: f64,
    cell_width: f64,
) {
    let x = position.column * cell_width;
    let y = position.line * line_height;
    match shape {
        CursorShape::Bar => {
            context.set_source_rgb(125.0 / 255.0, 211.0 / 255.0, 252.0 / 255.0);
            context.rectangle(x.round(), y.round(), 1.25, line_height);
        }
        CursorShape::Block => {
            context.set_source_rgba(125.0 / 255.0, 211.0 / 255.0, 252.0 / 255.0, 0.72);
            context.rectangle(x.round(), y.round(), cell_width.ceil(), line_height);
        }
    }
    let _ = context.fill();
}

fn draw_preedit(
    widget: &gtk::DrawingArea,
    context: &cairo::Context,
    render: &RenderFrame,
    preedit: &crate::render::RenderPreedit,
    line_height: f64,
    cell_width: f64,
) {
    let x = preedit.column.max(0) as f64 * cell_width;
    let y = preedit.line.max(0) as f64 * line_height;
    let columns = preedit.columns.max(1);
    context.set_source_rgb(37.0 / 255.0, 41.0 / 255.0, 48.0 / 255.0);
    context.rectangle(x, y, columns as f64 * cell_width, line_height);
    let _ = context.fill();

    let style = RenderStyle {
        fg: Some("#e5e7eb".to_string()),
        bg: None,
        bold: false,
        italic: false,
        underline: true,
        strikeout: false,
        selected: false,
    };
    let run = RenderRun {
        start_column: preedit.column.max(0) as usize,
        columns,
        text: preedit.text.clone(),
        style,
    };
    let layout = layout_for(widget, &run_markup(&run));
    gtk::render_layout(&widget.style_context(), context, x, y, &layout);

    if render.cursor.visible {
        let cursor_x =
            (preedit.column.max(0) as usize + preedit.cursor_columns) as f64 * cell_width;
        context.set_source_rgb(125.0 / 255.0, 211.0 / 255.0, 252.0 / 255.0);
        context.rectangle(cursor_x.round(), y.round(), 1.25, line_height);
        let _ = context.fill();
    }
}

fn run_markup(run: &RenderRun) -> String {
    let mut span = String::from("<span");
    push_style_markup(&mut span, &run.style);
    span.push('>');
    for ch in run.text.chars() {
        span.push_str(&markup_escape(ch));
    }
    span.push_str("</span>");
    span
}

fn push_style_markup(span: &mut String, style: &RenderStyle) {
    if let Some(fg) = &style.fg {
        span.push_str(&format!(" foreground=\"{}\"", fg));
    }
    if style.bold {
        span.push_str(" weight=\"bold\"");
    }
    if style.italic {
        span.push_str(" style=\"italic\"");
    }
    if style.underline {
        span.push_str(" underline=\"single\"");
    }
    if style.strikeout {
        span.push_str(" strikethrough=\"true\"");
    }
}

struct Rgb {
    red: f64,
    green: f64,
    blue: f64,
}

fn parse_hex_color(value: &str) -> Option<Rgb> {
    let value = value.strip_prefix('#')?;
    if value.len() != 6 {
        return None;
    }
    let red = u8::from_str_radix(&value[0..2], 16).ok()?;
    let green = u8::from_str_radix(&value[2..4], 16).ok()?;
    let blue = u8::from_str_radix(&value[4..6], 16).ok()?;
    Some(Rgb {
        red: f64::from(red) / 255.0,
        green: f64::from(green) / 255.0,
        blue: f64::from(blue) / 255.0,
    })
}

fn markup_escape(ch: char) -> String {
    match ch {
        '&' => "&amp;".to_string(),
        '<' => "&lt;".to_string(),
        '>' => "&gt;".to_string(),
        _ => ch.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::RenderRun;

    #[test]
    fn run_markup_escapes_text_and_preserves_style() {
        let run = RenderRun {
            start_column: 0,
            columns: 1,
            text: "<&>".to_string(),
            style: RenderStyle {
                fg: Some("#ff0000".to_string()),
                bg: None,
                bold: true,
                italic: true,
                underline: true,
                strikeout: true,
                selected: false,
            },
        };
        let markup = run_markup(&run);
        assert!(markup.contains("foreground=\"#ff0000\""));
        assert!(markup.contains("weight=\"bold\""));
        assert!(markup.contains("style=\"italic\""));
        assert!(markup.contains("underline=\"single\""));
        assert!(markup.contains("strikethrough=\"true\""));
        assert!(markup.contains("&lt;&amp;&gt;"));
    }

    #[test]
    fn parse_hex_color_rejects_invalid_colors() {
        assert!(parse_hex_color("#264f78").is_some());
        assert!(parse_hex_color("264f78").is_none());
        assert!(parse_hex_color("#xyzxyz").is_none());
    }

    #[test]
    fn cursor_blink_toggles_and_resets_on_cursor_movement() {
        let start = Instant::now();
        let cursor = CursorIdentity {
            pane_id: 0,
            line: 1,
            column: 2,
            visible: true,
        };
        let state = CursorBlinkState::default().sync(cursor, start);
        assert!(state.visible);
        let hidden = state.tick(true, start + CURSOR_BLINK_PERIOD);
        assert!(!hidden.visible);
        let moved = hidden.sync(
            CursorIdentity {
                column: 3,
                ..cursor
            },
            start + CURSOR_BLINK_PERIOD + Duration::from_millis(1),
        );
        assert!(moved.visible);
        assert!(
            moved
                .tick(
                    true,
                    start + CURSOR_BLINK_PERIOD + Duration::from_millis(120)
                )
                .visible
        );
    }

    #[test]
    fn cursor_motion_interpolates_between_visible_positions() {
        let start = Instant::now();
        let first = CursorIdentity {
            pane_id: 0,
            line: 1,
            column: 2,
            visible: true,
        };
        let second = CursorIdentity {
            column: 10,
            ..first
        };

        let state = CursorMotionState::default().sync(first, start, CursorStyle::Smooth);
        assert_eq!(
            state.position(start),
            Some(CursorDrawPosition {
                pane_id: 0,
                line: 1.0,
                column: 2.0
            })
        );

        let moved = state.sync(
            second,
            start + Duration::from_millis(1),
            CursorStyle::Smooth,
        );
        let halfway = moved
            .position(start + Duration::from_millis(1) + (CURSOR_MOTION_DURATION / 2))
            .expect("animated cursor position");
        assert!(halfway.column > 2.0);
        assert!(halfway.column < 10.0);
        assert!(moved.active(start + Duration::from_millis(1)));
        assert!(!moved.active(start + Duration::from_millis(1) + CURSOR_MOTION_DURATION));
        assert_eq!(
            moved.position(start + Duration::from_millis(1) + CURSOR_MOTION_DURATION),
            Some(CursorDrawPosition {
                pane_id: 0,
                line: 1.0,
                column: 10.0
            })
        );
    }

    #[test]
    fn cursor_motion_disabled_jumps_to_target() {
        let start = Instant::now();
        let first = CursorIdentity {
            pane_id: 0,
            line: 1,
            column: 2,
            visible: true,
        };
        let second = CursorIdentity {
            column: 10,
            ..first
        };

        let state = CursorMotionState::default().sync(first, start, CursorStyle::Smooth);
        let moved = state.sync(
            second,
            start + Duration::from_millis(1),
            CursorStyle::Steady,
        );

        assert!(!moved.active(start + Duration::from_millis(1)));
        assert_eq!(
            moved.position(start + Duration::from_millis(1)),
            Some(CursorDrawPosition {
                pane_id: 0,
                line: 1.0,
                column: 10.0
            })
        );
    }

    #[test]
    fn cursor_motion_exposes_trail_path_for_smear_styles() {
        let start = Instant::now();
        let first = CursorIdentity {
            pane_id: 7,
            line: 2,
            column: 4,
            visible: true,
        };
        let second = CursorIdentity {
            line: 3,
            column: 14,
            ..first
        };

        let state = CursorMotionState::default().sync(first, start, CursorStyle::Smear);
        let moved = state.sync(second, start + Duration::from_millis(1), CursorStyle::Smear);
        let path = moved
            .path(start + Duration::from_millis(1) + (CURSOR_MOTION_DURATION / 2))
            .expect("cursor trail path");

        assert_eq!(path.from.pane_id, 7);
        assert_eq!(path.target.column, 14.0);
        assert!(path.current.column > path.from.column);
        assert!(path.current.column < path.target.column);
        assert_eq!(moved.for_pane(7), Some(moved));
        assert_eq!(moved.for_pane(8), None);
    }
}
