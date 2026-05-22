use crate::config::{CursorBlinking, CursorShape, CursorStyle};
use crate::render::{RenderFrame, RenderRegion, RenderRun, RenderStyle};
use crate::terminal_font::{TerminalFontMetrics, layout_for_size, metrics_for_widget_size};
use crate::terminal_palette::default_terminal_palette;
use crate::workspace_render::WorkspaceRenderFrame;
use gtk::prelude::*;
use gtk::{cairo, pango};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant};

const CURSOR_BLINK_PERIOD: Duration = Duration::from_millis(530);
const PALETTE_TRANSITION_DURATION: Duration = Duration::from_millis(300);
const MAX_TEXT_LAYOUT_CACHE_ENTRIES: usize = 4096;
const MAX_ROW_SURFACE_CACHE_ENTRIES: usize = 512;
const TERMINAL_CANVAS_PADDING_PX: f64 = 6.0;
const TERMINAL_PREVIEW_RADIUS_PX: f64 = 6.0;

#[derive(Clone, Copy)]
pub struct TerminalCanvasPadding {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl TerminalCanvasPadding {
    pub fn content_width(self, widget: &gtk::DrawingArea) -> f64 {
        (f64::from(widget.allocated_width()) - self.left - self.right).max(0.0)
    }

    pub fn content_height(self, widget: &gtk::DrawingArea) -> f64 {
        (f64::from(widget.allocated_height()) - self.top - self.bottom).max(0.0)
    }
}

pub fn terminal_canvas_padding(widget: &gtk::DrawingArea) -> TerminalCanvasPadding {
    let _ = widget;
    TerminalCanvasPadding {
        left: TERMINAL_CANVAS_PADDING_PX,
        top: TERMINAL_CANVAS_PADDING_PX,
        right: TERMINAL_CANVAS_PADDING_PX,
        bottom: TERMINAL_CANVAS_PADDING_PX,
    }
}

#[derive(Clone)]
pub struct TerminalCanvas {
    area: gtk::DrawingArea,
    render: Rc<RefCell<Option<CanvasRenderFrame>>>,
    scroll_underlay: Rc<RefCell<Option<CanvasRenderFrame>>>,
    palette_transition: Rc<RefCell<Option<CanvasFrameTransition>>>,
    palette_transition_timer_active: Rc<Cell<bool>>,
    pending_palette_transition: Rc<RefCell<Option<CanvasRenderFrame>>>,
    cursor_blink: Rc<Cell<CursorBlinkState>>,
    cursor_motion: Rc<Cell<CursorMotionState>>,
    cursor_motion_suppressed: Rc<Cell<bool>>,
    cursor_options_override: Rc<Cell<Option<CursorOptions>>>,
    font_size_override: Rc<Cell<Option<f64>>>,
    scroll_visual_offset_px: Rc<Cell<f64>>,
}

impl TerminalCanvas {
    pub fn new() -> Self {
        let area = gtk::DrawingArea::new();
        area.set_focusable(true);
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.add_css_class("term-canvas");
        area.set_cursor_from_name(Some("text"));

        let render = Rc::new(RefCell::new(None::<CanvasRenderFrame>));
        let scroll_underlay = Rc::new(RefCell::new(None::<CanvasRenderFrame>));
        let palette_transition = Rc::new(RefCell::new(None::<CanvasFrameTransition>));
        let palette_transition_timer_active = Rc::new(Cell::new(false));
        let pending_palette_transition = Rc::new(RefCell::new(None::<CanvasRenderFrame>));
        let cursor_blink = Rc::new(Cell::new(CursorBlinkState::default()));
        let cursor_motion = Rc::new(Cell::new(CursorMotionState::default()));
        let cursor_motion_suppressed = Rc::new(Cell::new(false));
        let cursor_options_override = Rc::new(Cell::new(None::<CursorOptions>));
        let font_size_override = Rc::new(Cell::new(None::<f64>));
        let scroll_visual_offset_px = Rc::new(Cell::new(0.0));
        let text_layout_cache = Rc::new(RefCell::new(TextLayoutCache::default()));
        let row_surface_cache = Rc::new(RefCell::new(RowSurfaceCache::default()));
        let last_paint_started = Rc::new(Cell::new(None::<Instant>));
        let draw_render = render.clone();
        let draw_scroll_underlay = scroll_underlay.clone();
        let draw_palette_transition = palette_transition.clone();
        let draw_cursor_blink = cursor_blink.clone();
        let draw_cursor_motion = cursor_motion.clone();
        let draw_cursor_options_override = cursor_options_override.clone();
        let draw_font_size_override = font_size_override.clone();
        let draw_scroll_visual_offset_px = scroll_visual_offset_px.clone();
        let draw_text_layout_cache = text_layout_cache.clone();
        let draw_row_surface_cache = row_surface_cache.clone();
        let draw_last_paint_started = last_paint_started.clone();
        area.set_draw_func(move |widget, context, width, height| {
            let started = Instant::now();
            if let Some(previous) = draw_last_paint_started.replace(Some(started)) {
                crate::perf_trace::record_duration(
                    "gtk_paint_interval",
                    started.duration_since(previous),
                );
            }
            let now = Instant::now();
            let render = draw_render.borrow();
            if let Some(render) = render.as_ref() {
                let mut text_layout_cache = draw_text_layout_cache.borrow_mut();
                let mut row_surface_cache = draw_row_surface_cache.borrow_mut();
                let mut paint_resources = PaintResources {
                    text_layout_cache: &mut text_layout_cache,
                    row_surface_cache: &mut row_surface_cache,
                    stats: PaintStats::default(),
                };
                let paint = CanvasPaint {
                    cursor_blink: draw_cursor_blink.get(),
                    cursor_motion: draw_cursor_motion.get(),
                    cursor_options: draw_cursor_options_override
                        .get()
                        .unwrap_or_else(CursorOptions::from_config),
                    font_size_pt: draw_font_size_override
                        .get()
                        .unwrap_or_else(crate::terminal_font::font_size_pt),
                    scroll_visual_offset_px: draw_scroll_visual_offset_px.get(),
                    now,
                };
                let transition = draw_palette_transition.borrow().clone();
                if let Some(transition) = transition {
                    let elapsed = now.saturating_duration_since(transition.started);
                    let progress = (elapsed.as_secs_f64()
                        / PALETTE_TRANSITION_DURATION.as_secs_f64())
                    .clamp(0.0, 1.0);
                    let progress = ease_out_progress(progress);
                    draw_canvas_render_layer(
                        widget,
                        context,
                        width,
                        height,
                        &transition.from,
                        None,
                        &mut paint_resources,
                        paint,
                        1.0,
                    );
                    draw_canvas_render_layer(
                        widget,
                        context,
                        width,
                        height,
                        render,
                        draw_scroll_underlay.borrow().as_ref(),
                        &mut paint_resources,
                        paint,
                        progress,
                    );
                    if progress >= 1.0 {
                        draw_palette_transition.borrow_mut().take();
                    } else {
                        request_palette_animation_frame(widget);
                    }
                } else {
                    draw_canvas_render_layer(
                        widget,
                        context,
                        width,
                        height,
                        render,
                        draw_scroll_underlay.borrow().as_ref(),
                        &mut paint_resources,
                        paint,
                        1.0,
                    );
                }
                paint_resources.stats.record();
            }
            crate::perf_trace::record_duration("gtk_paint", started.elapsed());
        });

        Self {
            area,
            render,
            scroll_underlay,
            palette_transition,
            palette_transition_timer_active,
            pending_palette_transition,
            cursor_blink,
            cursor_motion,
            cursor_motion_suppressed,
            cursor_options_override,
            font_size_override,
            scroll_visual_offset_px,
        }
    }

    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.area
    }

    pub fn begin_palette_transition(&self) {
        if let Some(render) = self.render.borrow().as_ref() {
            self.pending_palette_transition
                .borrow_mut()
                .replace(render.clone());
        }
    }

    pub fn set_render(&self, render: RenderFrame) {
        self.set_canvas_render(CanvasRenderFrame::Single(Box::new(render)));
    }

    pub fn set_workspace_render(&self, render: WorkspaceRenderFrame) {
        self.set_canvas_render(CanvasRenderFrame::Workspace(render));
    }

    pub fn set_cursor_options_override(
        &self,
        style: Option<(crate::config::CursorStyle, crate::config::CursorShape)>,
    ) {
        self.cursor_options_override
            .set(style.map(|(style, shape)| CursorOptions { style, shape }));
        self.area.queue_draw();
    }

    pub fn set_font_size_override(&self, font_size_pt: Option<f64>) {
        self.font_size_override.set(font_size_pt);
        self.area.queue_draw();
    }

    pub fn set_cursor_motion_suppressed(&self, suppressed: bool) {
        self.cursor_motion_suppressed.set(suppressed);
    }

    pub fn add_preview_corners(&self) {
        self.area.add_css_class("term-preview-canvas");
    }

    pub fn refresh_cursor_options(&self) {
        let identity = {
            let render = self.render.borrow();
            let Some(render) = render.as_ref() else {
                self.area.queue_draw();
                return;
            };
            render
                .active_cursor_identity()
                .unwrap_or_else(CursorIdentity::hidden)
        };
        let options = self.cursor_options();
        let now = Instant::now();
        self.cursor_motion.set(CursorMotionState::settled(
            CursorDrawPosition {
                pane_id: identity.pane_id,
                line: f64::from(identity.line.max(0)),
                column: f64::from(identity.column.max(0)),
            },
            identity.visible,
            options.style,
            options.shape,
            now,
        ));
        self.cursor_blink
            .set(self.cursor_blink.get().sync(identity, now).tick(
                identity.visible,
                self.cursor_blinking_enabled(),
                now,
            ));
        self.area.queue_draw();
    }

    pub fn set_scroll_visual_offset_px(&self, offset_px: f64) {
        self.set_scroll_visual_offset_px_with_draw(offset_px, true);
    }

    pub fn set_scroll_visual_offset_px_deferred(&self, offset_px: f64) {
        self.set_scroll_visual_offset_px_with_draw(offset_px, false);
    }

    fn set_scroll_visual_offset_px_with_draw(&self, offset_px: f64, queue_draw: bool) {
        if (self.scroll_visual_offset_px.get() - offset_px).abs() < 0.1 {
            return;
        }
        self.scroll_visual_offset_px.set(offset_px);
        if offset_px.abs() < 0.1 {
            self.scroll_underlay.borrow_mut().take();
        }
        if queue_draw {
            self.area.queue_draw();
        }
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
        let cursor_options = self.cursor_options();
        let next_motion = if self.cursor_motion_suppressed.get() {
            CursorMotionState::settled(
                CursorDrawPosition {
                    pane_id: identity.pane_id,
                    line: f64::from(identity.line.max(0)),
                    column: f64::from(identity.column.max(0)),
                },
                identity.visible,
                cursor_options.style,
                cursor_options.shape,
                now,
            )
        } else {
            previous_motion.sync(identity, now, cursor_options.style, cursor_options.shape)
        };
        self.cursor_motion.set(next_motion);
        if current.as_ref() == Some(&render) {
            if previous_blink != self.cursor_blink.get()
                || previous_motion != self.cursor_motion.get()
                || self.cursor_motion.get().active(now)
                || self.scroll_visual_offset_px.get().abs() >= 0.1
            {
                self.area.queue_draw();
            }
            return;
        }
        if let Some(from) = self.pending_palette_transition.borrow_mut().take() {
            self.start_palette_transition(from, now);
        } else if self.palette_transition.borrow().is_none()
            && let Some(current_frame) = current.as_ref()
            && current_frame.background() != render.background()
        {
            self.start_palette_transition(current_frame.clone(), now);
        }
        if self.scroll_visual_offset_px.get().abs() >= 0.1 {
            *self.scroll_underlay.borrow_mut() = current.clone();
        } else {
            self.scroll_underlay.borrow_mut().take();
        }
        *current = Some(render);
        self.area.queue_draw();
    }

    fn start_palette_transition(&self, from: CanvasRenderFrame, started: Instant) {
        self.palette_transition
            .borrow_mut()
            .replace(CanvasFrameTransition { from, started });
        self.start_palette_transition_timer();
    }

    fn start_palette_transition_timer(&self) {
        if self.palette_transition_timer_active.replace(true) {
            return;
        }
        let area = self.area.clone();
        let transition = self.palette_transition.clone();
        let timer_active = self.palette_transition_timer_active.clone();
        gtk::glib::timeout_add_local(std::time::Duration::from_millis(8), move || {
            if transition.borrow().is_some() {
                area.queue_draw();
                gtk::glib::ControlFlow::Continue
            } else {
                timer_active.set(false);
                gtk::glib::ControlFlow::Break
            }
        });
    }

    pub fn tick_cursor_visual(&self) {
        let cursor_visible = {
            let render = self.render.borrow();
            let Some(render) = render.as_ref() else {
                return;
            };
            render.active_cursor_visible()
        };
        let now = Instant::now();
        let next =
            self.cursor_blink
                .get()
                .tick(cursor_visible, self.cursor_blinking_enabled(), now);
        let current_motion = self.cursor_motion.get();
        let next_motion = current_motion.settle_if_complete(now, self.cursor_options().shape);
        if current_motion != next_motion {
            self.cursor_motion.set(next_motion);
        }
        if self.cursor_blink.get() != next
            || current_motion.active(now)
            || current_motion != next_motion
        {
            self.cursor_blink.set(next);
            self.area.queue_draw();
        }
    }

    fn cursor_options(&self) -> CursorOptions {
        self.cursor_options_override
            .get()
            .unwrap_or_else(CursorOptions::from_config)
    }

    fn cursor_blinking_enabled(&self) -> bool {
        match crate::config::cursor_blinking() {
            CursorBlinking::FollowSystem => gtk::Settings::default()
                .map(|settings| settings.is_gtk_cursor_blink())
                .unwrap_or(true),
            CursorBlinking::Enabled => true,
            CursorBlinking::Disabled => false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CanvasRenderFrame {
    Single(Box<RenderFrame>),
    Workspace(WorkspaceRenderFrame),
}

impl CanvasRenderFrame {
    fn background(&self) -> Option<&str> {
        match self {
            Self::Single(render) => Some(render.background.as_str()),
            Self::Workspace(render) => render
                .panes
                .iter()
                .find(|pane| pane.active)
                .or_else(|| render.panes.first())
                .map(|pane| pane.frame.background.as_str()),
        }
    }

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

#[derive(Clone)]
struct CanvasFrameTransition {
    from: CanvasRenderFrame,
    started: Instant,
}

impl Default for TerminalCanvas {
    fn default() -> Self {
        Self::new()
    }
}

fn draw_background(context: &cairo::Context, width: i32, height: i32, color: Option<&str>) {
    let Some(color) = color.and_then(parse_hex_color) else {
        return;
    };
    context.set_source_rgb(color.red, color.green, color.blue);
    context.rectangle(0.0, 0.0, width as f64, height as f64);
    let _ = context.fill();
}

fn draw_rounded_rectangle(
    context: &cairo::Context,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    radius: f64,
) {
    let radius = radius.min(width / 2.0).min(height / 2.0);
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

fn ease_out_progress(progress: f64) -> f64 {
    1.0 - (1.0 - progress).powi(3)
}

fn request_palette_animation_frame(widget: &gtk::DrawingArea) {
    widget.queue_draw();
    if let Some(frame_clock) = widget.frame_clock() {
        frame_clock.request_phase(gtk::gdk::FrameClockPhase::UPDATE);
        frame_clock.request_phase(gtk::gdk::FrameClockPhase::PAINT);
    }
}

fn draw_canvas_render_layer(
    widget: &gtk::DrawingArea,
    context: &cairo::Context,
    width: i32,
    height: i32,
    render: &CanvasRenderFrame,
    scroll_underlay: Option<&CanvasRenderFrame>,
    paint_resources: &mut PaintResources<'_>,
    paint: CanvasPaint,
    alpha: f64,
) {
    if alpha <= 0.0 {
        return;
    }
    let alpha = alpha.min(1.0);
    let _ = context.save();
    if widget.has_css_class("term-preview-canvas") {
        draw_rounded_rectangle(
            context,
            0.0,
            0.0,
            width as f64,
            height as f64,
            TERMINAL_PREVIEW_RADIUS_PX,
        );
        context.clip();
    }
    if alpha < 1.0 {
        context.push_group();
    }
    draw_background(context, width, height, render.background());
    draw_canvas_render(
        widget,
        context,
        render,
        scroll_underlay,
        paint_resources,
        paint,
    );
    if alpha < 1.0 {
        let _ = context.pop_group_to_source();
        let _ = context.paint_with_alpha(alpha);
    }
    let _ = context.restore();
}

fn draw_canvas_render(
    widget: &gtk::DrawingArea,
    context: &cairo::Context,
    render: &CanvasRenderFrame,
    scroll_underlay: Option<&CanvasRenderFrame>,
    paint_resources: &mut PaintResources<'_>,
    paint: CanvasPaint,
) {
    let Some(metrics) = metrics_for_widget_size(widget, paint.font_size_pt) else {
        return;
    };
    let padding = terminal_canvas_padding(widget);
    let content_width = padding.content_width(widget);
    let content_height = padding.content_height(widget);
    if content_width <= 0.0 || content_height <= 0.0 {
        return;
    }
    paint_resources
        .text_layout_cache
        .set_font_size(paint.font_size_pt);
    let cursor_paint = CursorPaintState {
        visible: paint.cursor_blink.visible,
        pane_id: 0,
        motion: paint.cursor_motion,
        options: paint.cursor_options,
        now: paint.now,
    };
    let _ = context.save();
    context.rectangle(padding.left, padding.top, content_width, content_height);
    context.clip();
    context.translate(padding.left, padding.top);
    match render {
        CanvasRenderFrame::Single(render) => {
            draw_render_frame(
                widget,
                context,
                render,
                cursor_paint.with_blink(paint.cursor_blink.visible),
                metrics,
                paint_resources,
                PaintViewport {
                    scroll_visual_offset_px: paint.scroll_visual_offset_px,
                    width: content_width,
                },
            );
            if let Some(CanvasRenderFrame::Single(underlay)) = scroll_underlay {
                draw_scroll_underlay_frame_in_rect(
                    widget,
                    context,
                    underlay,
                    metrics,
                    paint_resources,
                    paint.scroll_visual_offset_px,
                    PaintRect {
                        width: content_width,
                        height: content_height,
                    },
                );
            }
        }
        CanvasRenderFrame::Workspace(render) => {
            draw_workspace_render(
                widget,
                context,
                render,
                cursor_paint,
                metrics,
                paint_resources,
                paint.scroll_visual_offset_px,
            );
            if let Some(CanvasRenderFrame::Workspace(underlay)) = scroll_underlay {
                draw_workspace_scroll_underlay(
                    widget,
                    context,
                    render,
                    underlay,
                    metrics,
                    paint_resources,
                    paint.scroll_visual_offset_px,
                );
            }
        }
    }
    let _ = context.restore();
}

fn draw_workspace_scroll_underlay(
    widget: &gtk::DrawingArea,
    context: &cairo::Context,
    render: &WorkspaceRenderFrame,
    underlay: &WorkspaceRenderFrame,
    metrics: TerminalFontMetrics,
    paint_resources: &mut PaintResources<'_>,
    scroll_visual_offset_px: f64,
) {
    if scroll_visual_offset_px.abs() < 0.1 {
        return;
    }
    let Some(active_pane) = render.panes.iter().find(|pane| pane.active) else {
        return;
    };
    let Some(underlay_pane) = underlay
        .panes
        .iter()
        .find(|pane| pane.pane_id == active_pane.pane_id)
    else {
        return;
    };
    let cell_width = metrics.cell_width;
    let line_height = metrics.line_height;
    let left = active_pane.origin_col as f64 * cell_width;
    let top = active_pane.origin_row as f64 * line_height;
    let width = active_pane.cols as f64 * cell_width;
    let height = active_pane.rows as f64 * line_height;
    let _ = context.save();
    context.rectangle(left, top, width, height);
    context.clip();
    context.translate(left, top);
    draw_scroll_underlay_frame_in_rect(
        widget,
        context,
        &underlay_pane.frame,
        metrics,
        paint_resources,
        scroll_visual_offset_px,
        PaintRect { width, height },
    );
    let _ = context.restore();
}

fn draw_scroll_underlay_frame_in_rect(
    widget: &gtk::DrawingArea,
    context: &cairo::Context,
    render: &RenderFrame,
    metrics: TerminalFontMetrics,
    paint_resources: &mut PaintResources<'_>,
    scroll_visual_offset_px: f64,
    rect: PaintRect,
) {
    if scroll_visual_offset_px.abs() < 0.1 || rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }
    let line_height = metrics.line_height;
    let _ = context.save();
    if scroll_visual_offset_px < 0.0 {
        let missing = (-scroll_visual_offset_px).min(line_height);
        context.rectangle(0.0, rect.height - missing, rect.width, missing);
        context.clip();
        draw_render_frame(
            widget,
            context,
            render,
            CursorPaintState::hidden(),
            metrics,
            paint_resources,
            PaintViewport {
                scroll_visual_offset_px: scroll_visual_offset_px + line_height,
                width: rect.width,
            },
        );
    } else {
        let missing = scroll_visual_offset_px.min(line_height);
        context.rectangle(0.0, 0.0, rect.width, missing);
        context.clip();
        draw_render_frame(
            widget,
            context,
            render,
            CursorPaintState::hidden(),
            metrics,
            paint_resources,
            PaintViewport {
                scroll_visual_offset_px: scroll_visual_offset_px - line_height,
                width: rect.width,
            },
        );
    }
    let _ = context.restore();
}

fn draw_render_frame(
    widget: &gtk::DrawingArea,
    context: &cairo::Context,
    render: &RenderFrame,
    cursor_paint: CursorPaintState,
    metrics: TerminalFontMetrics,
    paint_resources: &mut PaintResources<'_>,
    viewport: PaintViewport,
) {
    let cell_width = metrics.cell_width;
    let line_height = metrics.line_height;
    let _ = context.save();
    context.translate(0.0, viewport.scroll_visual_offset_px);
    let vertical_clip = context
        .clip_extents()
        .ok()
        .map(|(_, top, _, bottom)| (top, bottom));
    for line in &render.lines {
        let top = line.row as f64 * line_height;
        if vertical_clip.is_some_and(|(clip_top, clip_bottom)| {
            !row_intersects_clip(top, line_height, clip_top, clip_bottom)
        }) {
            continue;
        }
        paint_resources.stats.rows += 1;
        draw_render_line(
            widget,
            context,
            line,
            metrics,
            paint_resources,
            top,
            viewport.width,
        );
    }

    if render.cursor.visible && cursor_paint.visible {
        if let Some(preedit) = &render.preedit {
            draw_preedit(
                widget,
                context,
                render,
                preedit,
                line_height,
                cell_width,
                paint_resources.text_layout_cache.font_size_pt,
            );
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
        draw_preedit(
            widget,
            context,
            render,
            preedit,
            line_height,
            cell_width,
            paint_resources.text_layout_cache.font_size_pt,
        );
    }
    let _ = context.restore();
}

fn row_intersects_clip(row_top: f64, line_height: f64, clip_top: f64, clip_bottom: f64) -> bool {
    let row_bottom = row_top + line_height;
    row_bottom >= clip_top && row_top <= clip_bottom
}

fn draw_render_line(
    widget: &gtk::DrawingArea,
    context: &cairo::Context,
    line: &crate::render::RenderLine,
    metrics: TerminalFontMetrics,
    paint_resources: &mut PaintResources<'_>,
    top: f64,
    frame_width: f64,
) {
    if let Some(surface) = paint_resources.row_surface_for(widget, line, metrics, frame_width) {
        let _ = context.set_source_surface(surface, 0.0, top);
        let _ = context.paint();
    } else {
        draw_render_line_direct(
            widget,
            context,
            line,
            metrics,
            paint_resources.text_layout_cache,
            &mut paint_resources.stats,
            top,
        );
    }
}

fn draw_render_line_direct(
    widget: &gtk::DrawingArea,
    context: &cairo::Context,
    line: &crate::render::RenderLine,
    metrics: TerminalFontMetrics,
    text_layout_cache: &mut TextLayoutCache,
    stats: &mut PaintStats,
    top: f64,
) {
    let cell_width = metrics.cell_width;
    let line_height = metrics.line_height;
    for run in &line.runs {
        draw_run_background(context, run, cell_width, line_height, top);
    }
    for run in &line.runs {
        if run.text.trim().is_empty() {
            continue;
        }
        let left = run.start_column as f64 * cell_width;
        let layout = layout_for_paint(widget, text_layout_cache, stats, &run.markup);
        let _ = context.save();
        context.rectangle(left, top, run.columns as f64 * cell_width, line_height);
        context.clip();
        gtk::render_layout(&widget.style_context(), context, left, top, &layout);
        let _ = context.restore();
    }
}

fn draw_workspace_render(
    widget: &gtk::DrawingArea,
    context: &cairo::Context,
    render: &WorkspaceRenderFrame,
    cursor_paint: CursorPaintState,
    metrics: TerminalFontMetrics,
    paint_resources: &mut PaintResources<'_>,
    scroll_visual_offset_px: f64,
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
            paint_resources,
            PaintViewport {
                scroll_visual_offset_px: if pane.active {
                    scroll_visual_offset_px
                } else {
                    0.0
                },
                width,
            },
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
struct CursorPaintState {
    visible: bool,
    pane_id: u64,
    motion: CursorMotionState,
    options: CursorOptions,
    now: Instant,
}

#[derive(Clone, Copy)]
struct CanvasPaint {
    cursor_blink: CursorBlinkState,
    cursor_motion: CursorMotionState,
    cursor_options: CursorOptions,
    font_size_pt: f64,
    scroll_visual_offset_px: f64,
    now: Instant,
}

#[derive(Clone, Copy)]
struct PaintRect {
    width: f64,
    height: f64,
}

#[derive(Clone, Copy)]
struct PaintViewport {
    scroll_visual_offset_px: f64,
    width: f64,
}

#[derive(Default)]
struct PaintStats {
    rows: u64,
    layout_hits: u64,
    layout_misses: u64,
    row_surface_hits: u64,
    row_surface_misses: u64,
}

impl PaintStats {
    fn record(&self) {
        crate::perf_trace::record_counter("gtk_paint_rows", self.rows);
        crate::perf_trace::record_counter("gtk_layout_cache_hits", self.layout_hits);
        crate::perf_trace::record_counter("gtk_layout_cache_misses", self.layout_misses);
        crate::perf_trace::record_counter("gtk_row_surface_hits", self.row_surface_hits);
        crate::perf_trace::record_counter("gtk_row_surface_misses", self.row_surface_misses);
    }
}

struct PaintResources<'a> {
    text_layout_cache: &'a mut TextLayoutCache,
    row_surface_cache: &'a mut RowSurfaceCache,
    stats: PaintStats,
}

impl PaintResources<'_> {
    fn row_surface_for(
        &mut self,
        widget: &gtk::DrawingArea,
        line: &crate::render::RenderLine,
        metrics: TerminalFontMetrics,
        frame_width: f64,
    ) -> Option<cairo::ImageSurface> {
        self.row_surface_cache.surface_for(
            widget,
            line,
            metrics,
            frame_width,
            self.text_layout_cache,
            &mut self.stats,
        )
    }
}

fn layout_for_paint(
    widget: &gtk::DrawingArea,
    text_layout_cache: &mut TextLayoutCache,
    stats: &mut PaintStats,
    markup: &str,
) -> pango::Layout {
    match text_layout_cache.layout_for(widget, markup) {
        CachedLayout::Hit(layout) => {
            stats.layout_hits += 1;
            layout
        }
        CachedLayout::Miss(layout) => {
            stats.layout_misses += 1;
            layout
        }
    }
}

#[derive(Default)]
struct RowSurfaceCache {
    surfaces: HashMap<RowSurfaceBucketKey, Vec<RowSurfaceEntry>>,
    order: VecDeque<RowSurfaceKey>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RowSurfaceKey {
    bucket: RowSurfaceBucketKey,
    line: RowSurfaceLineKey,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct RowSurfaceBucketKey {
    signature: TextLayoutCacheSignature,
    width_px: i32,
    height_px: i32,
    line_paint_key: u64,
}

struct RowSurfaceEntry {
    line: RowSurfaceLineKey,
    surface: cairo::ImageSurface,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct RowSurfaceLineKey {
    region: RenderRegion,
    text: String,
    runs: Vec<RenderRun>,
}

impl RowSurfaceCache {
    fn surface_for(
        &mut self,
        widget: &gtk::DrawingArea,
        line: &crate::render::RenderLine,
        metrics: TerminalFontMetrics,
        frame_width: f64,
        text_layout_cache: &mut TextLayoutCache,
        stats: &mut PaintStats,
    ) -> Option<cairo::ImageSurface> {
        let spec = RowSurfaceSpec {
            width_px: frame_width.ceil() as i32,
            height_px: metrics.line_height.ceil() as i32,
            metrics,
        };
        if spec.width_px <= 0 || spec.height_px <= 0 {
            return None;
        }
        let bucket = RowSurfaceBucketKey {
            signature: TextLayoutCacheSignature::for_widget(widget, text_layout_cache.font_size_pt),
            width_px: spec.width_px,
            height_px: spec.height_px,
            line_paint_key: line.paint_key,
        };
        if let Some(entry) = self
            .surfaces
            .get(&bucket)
            .and_then(|entries| entries.iter().find(|entry| entry.line.matches(line)))
        {
            stats.row_surface_hits += 1;
            return Some(entry.surface.clone());
        }
        let surface = self.render_surface(widget, line, spec, text_layout_cache, stats)?;
        if self.len() >= MAX_ROW_SURFACE_CACHE_ENTRIES
            && let Some(evicted) = self.order.pop_front()
        {
            self.remove(&evicted);
        }
        let line_key = RowSurfaceLineKey::from(line);
        let key = RowSurfaceKey {
            bucket,
            line: line_key.clone(),
        };
        self.order.push_back(key);
        self.surfaces
            .entry(bucket)
            .or_default()
            .push(RowSurfaceEntry {
                line: line_key,
                surface: surface.clone(),
            });
        stats.row_surface_misses += 1;
        Some(surface)
    }

    fn len(&self) -> usize {
        self.order.len()
    }

    fn remove(&mut self, key: &RowSurfaceKey) {
        let Some(entries) = self.surfaces.get_mut(&key.bucket) else {
            return;
        };
        if let Some(index) = entries.iter().position(|entry| entry.line == key.line) {
            entries.remove(index);
        }
        if entries.is_empty() {
            self.surfaces.remove(&key.bucket);
        }
    }

    fn render_surface(
        &self,
        widget: &gtk::DrawingArea,
        line: &crate::render::RenderLine,
        spec: RowSurfaceSpec,
        text_layout_cache: &mut TextLayoutCache,
        stats: &mut PaintStats,
    ) -> Option<cairo::ImageSurface> {
        let surface =
            cairo::ImageSurface::create(cairo::Format::ARgb32, spec.width_px, spec.height_px)
                .ok()?;
        let context = cairo::Context::new(&surface).ok()?;
        context.set_operator(cairo::Operator::Clear);
        let _ = context.paint();
        context.set_operator(cairo::Operator::Over);
        draw_render_line_direct(
            widget,
            &context,
            line,
            spec.metrics,
            text_layout_cache,
            stats,
            0.0,
        );
        surface.flush();
        Some(surface)
    }
}

#[derive(Clone, Copy)]
struct RowSurfaceSpec {
    width_px: i32,
    height_px: i32,
    metrics: TerminalFontMetrics,
}

impl From<&crate::render::RenderLine> for RowSurfaceLineKey {
    fn from(line: &crate::render::RenderLine) -> Self {
        Self {
            region: line.region,
            text: line.text.clone(),
            runs: line.runs.clone(),
        }
    }
}

impl RowSurfaceLineKey {
    fn matches(&self, line: &crate::render::RenderLine) -> bool {
        self.region == line.region && self.text == line.text && self.runs == line.runs
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct TextLayoutCacheSignature {
    font_size_tenths: u32,
    text_scale_micros: u32,
    line_spacing_tenths: u32,
    column_spacing_tenths: u32,
}

#[derive(Default)]
struct TextLayoutCache {
    signature: Option<TextLayoutCacheSignature>,
    font_size_pt: f64,
    letter_spacing: i32,
    layouts: HashMap<String, pango::Layout>,
}

enum CachedLayout {
    Hit(pango::Layout),
    Miss(pango::Layout),
}

impl TextLayoutCache {
    fn set_font_size(&mut self, font_size_pt: f64) {
        if self.font_size_pt.to_bits() != font_size_pt.to_bits() {
            self.font_size_pt = font_size_pt;
            self.signature = None;
            self.layouts.clear();
        }
    }

    fn layout_for(&mut self, widget: &gtk::DrawingArea, markup: &str) -> CachedLayout {
        self.sync_signature(widget);
        if let Some(layout) = self.layouts.get(markup) {
            return CachedLayout::Hit(layout.clone());
        }
        if self.layouts.len() >= MAX_TEXT_LAYOUT_CACHE_ENTRIES {
            self.layouts.clear();
        }
        let layout = crate::terminal_font::layout_for_size_with_letter_spacing(
            widget,
            markup,
            self.font_size_pt,
            self.letter_spacing,
        );
        self.layouts.insert(markup.to_string(), layout.clone());
        CachedLayout::Miss(layout)
    }

    fn sync_signature(&mut self, widget: &gtk::DrawingArea) {
        let signature = TextLayoutCacheSignature::for_widget(widget, self.font_size_pt);
        if self.signature != Some(signature) {
            self.signature = Some(signature);
            self.letter_spacing =
                crate::terminal_font::letter_spacing_for_widget_size(widget, self.font_size_pt);
            self.layouts.clear();
        }
    }
}

impl TextLayoutCacheSignature {
    fn for_widget(widget: &gtk::DrawingArea, font_size_pt: f64) -> Self {
        Self {
            font_size_tenths: (font_size_pt * 10.0).round() as u32,
            text_scale_micros: (crate::terminal_font::text_scale_for_widget(widget) * 1_000_000.0)
                .round() as u32,
            line_spacing_tenths: (crate::config::line_spacing() * 10.0).round() as u32,
            column_spacing_tenths: (crate::config::column_spacing() * 10.0).round() as u32,
        }
    }
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
    fn hidden() -> Self {
        Self {
            visible: false,
            pane_id: 0,
            motion: CursorMotionState::default(),
            options: CursorOptions::from_config(),
            now: Instant::now(),
        }
    }

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
    visible: bool,
    style: CursorStyle,
    pane_id: u64,
    from_line: f64,
    from_column: f64,
    to_line: f64,
    to_column: f64,
    started_at: Option<Instant>,
    last_updated_at: Option<Instant>,
    neovide_corners: [NeovideCornerMotion; 4],
    smear_rect: SmearRectMotion,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CursorDrawPosition {
    pub(crate) pane_id: u64,
    pub(crate) line: f64,
    pub(crate) column: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CursorDrawPath {
    pub(crate) from: CursorDrawPosition,
    pub(crate) current: CursorDrawPosition,
    pub(crate) target: CursorDrawPosition,
    pub(crate) progress: f64,
    pub(crate) elapsed: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CursorPoint {
    x: f64,
    y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct AxisSpring {
    position: f64,
    velocity: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct NeovideCornerMotion {
    current: CursorPoint,
    previous_destination: CursorPoint,
    spring_x: AxisSpring,
    spring_y: AxisSpring,
    animation_duration: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SmearRectMotion {
    left: f64,
    right: f64,
    top: f64,
    velocity_left: f64,
    velocity_right: f64,
    velocity_top: f64,
    target_left: f64,
    target_right: f64,
    target_top: f64,
}

impl Default for AxisSpring {
    fn default() -> Self {
        Self {
            position: 0.0,
            velocity: 0.0,
        }
    }
}

impl Default for NeovideCornerMotion {
    fn default() -> Self {
        Self {
            current: CursorPoint { x: 0.0, y: 0.0 },
            previous_destination: CursorPoint { x: 0.0, y: 0.0 },
            spring_x: AxisSpring::default(),
            spring_y: AxisSpring::default(),
            animation_duration: Duration::ZERO,
        }
    }
}

impl Default for SmearRectMotion {
    fn default() -> Self {
        Self {
            left: 0.0,
            right: 1.0,
            top: 0.0,
            velocity_left: 0.0,
            velocity_right: 0.0,
            velocity_top: 0.0,
            target_left: 0.0,
            target_right: 1.0,
            target_top: 0.0,
        }
    }
}

impl Default for CursorMotionState {
    fn default() -> Self {
        Self {
            initialized: false,
            visible: false,
            style: CursorStyle::Steady,
            pane_id: 0,
            from_line: 0.0,
            from_column: 0.0,
            to_line: 0.0,
            to_column: 0.0,
            started_at: None,
            last_updated_at: None,
            neovide_corners: [NeovideCornerMotion::default(); 4],
            smear_rect: SmearRectMotion::default(),
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
        if self.cursor != Some(identity) || self.reset_at.is_none() {
            return Self {
                visible: true,
                cursor: Some(identity),
                reset_at: Some(now),
            };
        }
        self
    }

    fn tick(self, cursor_visible: bool, blink_enabled: bool, now: Instant) -> Self {
        if !cursor_visible {
            return Self {
                visible: false,
                cursor: self.cursor,
                reset_at: None,
            };
        }
        if !blink_enabled {
            return Self {
                visible: true,
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
    fn sync(
        self,
        identity: CursorIdentity,
        now: Instant,
        style: CursorStyle,
        shape: CursorShape,
    ) -> Self {
        let target = CursorDrawPosition {
            pane_id: identity.pane_id,
            line: f64::from(identity.line.max(0)),
            column: f64::from(identity.column.max(0)),
        };
        if style == CursorStyle::Steady || !identity.visible || !self.visible {
            return Self::settled(target, identity.visible, style, shape, now);
        }

        let current_state = if self.style == style {
            self.advance(now, shape)
        } else {
            Self::settled(
                self.position(now).unwrap_or(target),
                identity.visible,
                style,
                shape,
                now,
            )
        };
        if current_state.pane_id != target.pane_id {
            return Self::settled(target, identity.visible, style, shape, now);
        }

        let current = current_state.position(now);
        if current == Some(target) {
            return current_state;
        }
        let from = current.filter(|position| position.pane_id == target.pane_id);
        let from = from.unwrap_or(target);
        let mut next = Self {
            initialized: true,
            visible: true,
            style,
            pane_id: target.pane_id,
            from_line: from.line,
            from_column: from.column,
            to_line: target.line,
            to_column: target.column,
            started_at: Some(now),
            last_updated_at: Some(now),
            neovide_corners: current_state.neovide_corners,
            smear_rect: current_state.smear_rect,
        };
        match style {
            CursorStyle::Neovide => next.sync_neovide_target(target, shape),
            CursorStyle::Smear => next.sync_smear_target(target, shape),
            CursorStyle::Smooth | CursorStyle::Steady => {}
        }
        next
    }

    fn settled(
        target: CursorDrawPosition,
        visible: bool,
        style: CursorStyle,
        shape: CursorShape,
        now: Instant,
    ) -> Self {
        let neovide_corners = cursor_corners_grid(target, shape).map(NeovideCornerMotion::settled);
        Self {
            initialized: true,
            visible,
            style,
            pane_id: target.pane_id,
            from_line: target.line,
            from_column: target.column,
            to_line: target.line,
            to_column: target.column,
            started_at: None,
            last_updated_at: Some(now),
            neovide_corners,
            smear_rect: SmearRectMotion::settled(target, shape),
        }
    }

    fn position(self, now: Instant) -> Option<CursorDrawPosition> {
        if !self.initialized {
            return None;
        }
        if self.style == CursorStyle::Neovide {
            let top_left = self.neovide_points_grid()[0];
            return Some(CursorDrawPosition {
                pane_id: self.pane_id,
                line: top_left.y,
                column: top_left.x,
            });
        }
        if self.style == CursorStyle::Smear {
            return Some(CursorDrawPosition {
                pane_id: self.pane_id,
                line: self.smear_rect.top,
                column: self.smear_rect.left,
            });
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
            elapsed: self
                .started_at
                .map(|started_at| now.saturating_duration_since(started_at))
                .unwrap_or_else(cursor_animation_duration),
        })
    }

    fn for_pane(self, pane_id: u64) -> Option<Self> {
        (self.initialized && self.pane_id == pane_id).then_some(self)
    }

    fn active(self, now: Instant) -> bool {
        match self.style {
            CursorStyle::Neovide => self.neovide_corners.iter().any(|corner| corner.active()),
            CursorStyle::Smear => self.smear_rect.active(),
            CursorStyle::Smooth => self.progress(now).is_some_and(|progress| progress < 1.0),
            CursorStyle::Steady => false,
        }
    }

    fn settle_if_complete(self, now: Instant, shape: CursorShape) -> Self {
        if !self.initialized {
            return self;
        }
        if !self.active(now) {
            if self.started_at.is_none() {
                return self;
            }
            return Self::settled(
                CursorDrawPosition {
                    pane_id: self.pane_id,
                    line: self.to_line,
                    column: self.to_column,
                },
                self.visible,
                self.style,
                shape,
                now,
            );
        }
        let advanced = self.advance(now, shape);
        if !advanced.active(now) {
            Self::settled(
                CursorDrawPosition {
                    pane_id: advanced.pane_id,
                    line: advanced.to_line,
                    column: advanced.to_column,
                },
                advanced.visible,
                advanced.style,
                shape,
                now,
            )
        } else {
            advanced
        }
    }

    fn progress(self, now: Instant) -> Option<f64> {
        let started_at = self.started_at?;
        let elapsed = now.saturating_duration_since(started_at);
        Some((elapsed.as_secs_f64() / cursor_animation_duration().as_secs_f64()).clamp(0.0, 1.0))
    }

    fn advance(self, now: Instant, shape: CursorShape) -> Self {
        if !self.initialized {
            return self;
        }
        let Some(previous) = self.last_updated_at else {
            return Self {
                last_updated_at: Some(now),
                ..self
            };
        };
        let dt = now.saturating_duration_since(previous);
        if dt.is_zero() {
            return self;
        }
        let mut next = Self {
            last_updated_at: Some(now),
            ..self
        };
        match self.style {
            CursorStyle::Neovide => next.advance_neovide(dt, shape),
            CursorStyle::Smear => next.advance_smear(dt, shape),
            CursorStyle::Smooth | CursorStyle::Steady => {}
        }
        next
    }

    fn sync_neovide_target(&mut self, target: CursorDrawPosition, shape: CursorShape) {
        let jump_vec = CursorPoint {
            x: target.column - self.from_column,
            y: target.line - self.from_line,
        };
        let short_jump = jump_vec.x.abs() <= 2.001 && jump_vec.y.abs() <= 0.001;
        let target_corners = cursor_corners_grid(target, shape);
        let ranks = neovide_corner_ranks_grid(self.neovide_points_grid(), target, shape);
        for (index, corner) in self.neovide_corners.iter_mut().enumerate() {
            let duration = neovide_corner_duration(ranks[index], short_jump);
            corner.jump(target_corners[index], duration);
        }
    }

    fn advance_neovide(&mut self, dt: Duration, shape: CursorShape) {
        let target = CursorDrawPosition {
            pane_id: self.pane_id,
            line: self.to_line,
            column: self.to_column,
        };
        let target_corners = cursor_corners_grid(target, shape);
        for (corner, target_corner) in self.neovide_corners.iter_mut().zip(target_corners) {
            corner.update(target_corner, dt);
        }
    }

    fn neovide_points_grid(self) -> [CursorPoint; 4] {
        self.neovide_corners.map(|corner| corner.current)
    }

    fn sync_smear_target(&mut self, target: CursorDrawPosition, shape: CursorShape) {
        self.smear_rect
            .sync_target(target, shape, !self.smear_rect.active());
    }

    fn advance_smear(&mut self, dt: Duration, shape: CursorShape) {
        self.smear_rect.advance(dt, shape);
    }

    fn smear_points_grid(self, shape: CursorShape) -> [CursorPoint; 4] {
        self.smear_rect.limited_points(shape)
    }
}

fn cursor_animation_duration() -> Duration {
    Duration::from_millis(u64::from(crate::config::cursor_animation_duration_ms()))
}

fn neovide_short_animation_duration() -> Duration {
    Duration::from_millis(u64::from(
        crate::config::cursor_neovide_short_animation_duration_ms(),
    ))
}

fn neovide_corner_duration(rank: usize, short_jump: bool) -> Duration {
    let full = cursor_animation_duration();
    if short_jump {
        return full.min(neovide_short_animation_duration());
    }
    let trail_size = crate::config::cursor_neovide_trail_size();
    let full_ms = full.as_secs_f64() * 1000.0;
    let leading_ms = full_ms * (1.0 - trail_size).clamp(0.0, 1.0);
    let duration_ms = match rank {
        2..=3 => leading_ms,
        1 => (leading_ms + full_ms) * 0.5,
        _ => full_ms,
    };
    Duration::from_secs_f64(duration_ms.max(1.0) / 1000.0)
}

impl NeovideCornerMotion {
    fn settled(destination: CursorPoint) -> Self {
        Self {
            current: destination,
            previous_destination: destination,
            spring_x: AxisSpring::default(),
            spring_y: AxisSpring::default(),
            animation_duration: Duration::ZERO,
        }
    }

    fn jump(&mut self, destination: CursorPoint, animation_duration: Duration) {
        if self.previous_destination == destination {
            return;
        }
        self.spring_x.position = destination.x - self.current.x;
        self.spring_y.position = destination.y - self.current.y;
        self.previous_destination = destination;
        self.animation_duration = animation_duration;
    }

    fn update(&mut self, destination: CursorPoint, dt: Duration) -> bool {
        if self.previous_destination != destination {
            self.jump(destination, self.animation_duration);
        }
        let x_active = self.spring_x.update(dt, self.animation_duration);
        let y_active = self.spring_y.update(dt, self.animation_duration);
        self.current = CursorPoint {
            x: destination.x - self.spring_x.position,
            y: destination.y - self.spring_y.position,
        };
        x_active || y_active
    }

    fn active(self) -> bool {
        self.spring_x.active() || self.spring_y.active()
    }
}

impl AxisSpring {
    fn update(&mut self, dt: Duration, animation_duration: Duration) -> bool {
        let dt = dt.as_secs_f64();
        let animation_length = animation_duration.as_secs_f64();
        if animation_length <= dt {
            self.reset();
            return false;
        }
        if self.position == 0.0 {
            return false;
        }

        let zeta = 1.0;
        let omega = 4.0 / (zeta * animation_length);
        let a = self.position;
        let b = (self.position * omega) + self.velocity;
        let c = (-omega * dt).exp();
        self.position = (a + (b * dt)) * c;
        self.velocity = c * ((-a * omega) - (b * dt * omega) + b);
        if self.position.abs() < 0.01 {
            self.reset();
            false
        } else {
            true
        }
    }

    fn active(self) -> bool {
        self.position != 0.0 || self.velocity != 0.0
    }

    fn reset(&mut self) {
        self.position = 0.0;
        self.velocity = 0.0;
    }
}

impl SmearRectMotion {
    fn settled(target: CursorDrawPosition, shape: CursorShape) -> Self {
        let rect = cursor_rect_grid(target, shape);
        Self {
            left: rect.left,
            right: rect.right,
            top: rect.top,
            velocity_left: 0.0,
            velocity_right: 0.0,
            velocity_top: 0.0,
            target_left: rect.left,
            target_right: rect.right,
            target_top: rect.top,
        }
    }

    fn sync_target(&mut self, target: CursorDrawPosition, shape: CursorShape, initial_jump: bool) {
        let rect = cursor_rect_grid(target, shape);
        if self.target_left == rect.left
            && self.target_right == rect.right
            && self.target_top == rect.top
        {
            return;
        }
        self.target_left = rect.left;
        self.target_right = rect.right;
        self.target_top = rect.top;
        if initial_jump {
            let anticipation = crate::config::cursor_smear_anticipation();
            self.velocity_left = (self.left - self.target_left) * anticipation;
            self.velocity_right = (self.right - self.target_right) * anticipation;
            self.velocity_top = (self.top - self.target_top) * anticipation;
        }
    }

    fn advance(&mut self, dt: Duration, shape: CursorShape) {
        let dt_ms = dt.as_secs_f64() * 1000.0;
        if dt_ms <= 0.0 {
            return;
        }
        let target_center = (self.target_left + self.target_right) * 0.5;
        let head_stiffness = crate::config::cursor_smear_stiffness();
        let tail_stiffness = crate::config::cursor_smear_trailing_stiffness();
        let exponent = crate::config::cursor_smear_trailing_exponent();
        let left_distance = (self.left - target_center).abs();
        let right_distance = (self.right - target_center).abs();
        let min_distance = left_distance.min(right_distance);
        let max_distance = left_distance.max(right_distance);
        let left_side_stiffness = smear_stiffness_for_distance(
            left_distance,
            min_distance,
            max_distance,
            head_stiffness,
            tail_stiffness,
            exponent,
        );
        let right_side_stiffness = smear_stiffness_for_distance(
            right_distance,
            min_distance,
            max_distance,
            head_stiffness,
            tail_stiffness,
            exponent,
        );
        let damping = crate::config::cursor_smear_damping();
        smear_axis_step(
            &mut self.left,
            &mut self.velocity_left,
            self.target_left,
            left_side_stiffness,
            damping,
            dt_ms,
        );
        smear_axis_step(
            &mut self.right,
            &mut self.velocity_right,
            self.target_right,
            right_side_stiffness,
            damping,
            dt_ms,
        );
        smear_axis_step(
            &mut self.top,
            &mut self.velocity_top,
            self.target_top,
            head_stiffness,
            damping,
            dt_ms,
        );
        let target_width = cursor_size_cells(shape).0;
        if self.right < self.left + target_width {
            let center = (self.left + self.right) * 0.5;
            self.left = center - (target_width * 0.5);
            self.right = center + (target_width * 0.5);
        }
    }

    fn active(self) -> bool {
        let max_distance = (self.left - self.target_left)
            .abs()
            .max((self.right - self.target_right).abs())
            .max((self.top - self.target_top).abs());
        let max_velocity = self
            .velocity_left
            .abs()
            .max(self.velocity_right.abs())
            .max(self.velocity_top.abs());
        max_distance > SMEAR_STOP_DISTANCE_CELLS || max_velocity > SMEAR_STOP_DISTANCE_CELLS
    }

    fn limited_points(self, shape: CursorShape) -> [CursorPoint; 4] {
        let mut rect = SmearRect {
            left: self.left,
            right: self.right,
            top: self.top,
            height: cursor_size_cells(shape).1,
        };
        let max_width = cursor_size_cells(shape).0 + crate::config::cursor_smear_max_length();
        if rect.right - rect.left > max_width {
            let moving_right = (self.target_left + self.target_right) >= (self.left + self.right);
            if moving_right {
                rect.left = rect.right - max_width;
            } else {
                rect.right = rect.left + max_width;
            }
        }
        rect.points()
    }
}

fn smear_axis_step(
    current: &mut f64,
    velocity: &mut f64,
    target: f64,
    stiffness: f64,
    damping: f64,
    dt_ms: f64,
) {
    let speed_correction = dt_ms / SMEAR_BASE_FRAME_MS;
    let velocity_conservation = (1.0 - damping).powf(speed_correction);
    let damping_correction = 1.0 / (1.0 + (2.5 * velocity_conservation));
    let effective_stiffness = 1.0 - (1.0 - (stiffness * damping_correction)).powf(speed_correction);
    *velocity += (target - *current) * effective_stiffness;
    *current += *velocity;
    *velocity *= velocity_conservation;
}

fn smear_stiffness_for_distance(
    distance: f64,
    min_distance: f64,
    max_distance: f64,
    head_stiffness: f64,
    trailing_stiffness: f64,
    trailing_exponent: f64,
) -> f64 {
    if (max_distance - min_distance).abs() <= f64::EPSILON {
        return head_stiffness;
    }
    let factor = (distance - min_distance) / (max_distance - min_distance);
    (head_stiffness + ((trailing_stiffness - head_stiffness) * factor.powf(trailing_exponent)))
        .min(1.0)
}

#[derive(Clone, Copy)]
struct SmearRect {
    left: f64,
    right: f64,
    top: f64,
    height: f64,
}

impl SmearRect {
    fn points(self) -> [CursorPoint; 4] {
        [
            CursorPoint {
                x: self.left,
                y: self.top,
            },
            CursorPoint {
                x: self.right,
                y: self.top,
            },
            CursorPoint {
                x: self.right,
                y: self.top + self.height,
            },
            CursorPoint {
                x: self.left,
                y: self.top + self.height,
            },
        ]
    }
}

const SMEAR_BASE_FRAME_MS: f64 = 17.0;
const SMEAR_STOP_DISTANCE_CELLS: f64 = 0.1;

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
        CursorStyle::Neovide => {
            if let Some(motion) = cursor_motion {
                let points =
                    points_to_pixels(motion.neovide_points_grid(), line_height, cell_width);
                draw_neovide_points(context, points, cursor_options.shape);
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
        CursorStyle::Smear => {
            if let Some(motion) = cursor_motion {
                let points = points_to_pixels(
                    motion.smear_points_grid(cursor_options.shape),
                    line_height,
                    cell_width,
                );
                let target_center =
                    cursor_center(target, cursor_options.shape, line_height, cell_width);
                draw_smear_points(context, points, target_center, cursor_options.shape);
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

fn draw_neovide_points(context: &cairo::Context, points: [CursorPoint; 4], shape: CursorShape) {
    context.set_source_rgba(
        125.0 / 255.0,
        211.0 / 255.0,
        252.0 / 255.0,
        neovide_cursor_alpha(shape),
    );
    draw_cursor_polygon(context, points);
}

fn draw_smear_points(
    context: &cairo::Context,
    points: [CursorPoint; 4],
    target_center: CursorPoint,
    shape: CursorShape,
) {
    let mut head = points[0];
    let mut tail = points[0];
    for point in points {
        if squared_distance(point, target_center) < squared_distance(head, target_center) {
            head = point;
        }
        if squared_distance(point, target_center) > squared_distance(tail, target_center) {
            tail = point;
        }
    }
    let gradient = cairo::LinearGradient::new(head.x, head.y, tail.x, tail.y);
    gradient.add_color_stop_rgba(
        0.0,
        125.0 / 255.0,
        211.0 / 255.0,
        252.0 / 255.0,
        cursor_alpha(shape),
    );
    gradient.add_color_stop_rgba(1.0, 125.0 / 255.0, 211.0 / 255.0, 252.0 / 255.0, 0.16);
    let _ = context.set_source(&gradient);
    draw_cursor_polygon(context, points);
}

#[cfg(test)]
fn neovide_corners(
    path: CursorDrawPath,
    shape: CursorShape,
    line_height: f64,
    cell_width: f64,
) -> [CursorPoint; 4] {
    let from = cursor_corners(path.from, shape, line_height, cell_width);
    let target = cursor_corners(path.target, shape, line_height, cell_width);
    let ranks =
        neovide_corner_ranks_grid(cursor_corners_grid(path.from, shape), path.target, shape);
    let duration_ms = f64::from(crate::config::cursor_animation_duration_ms());
    let trail_size = crate::config::cursor_neovide_trail_size();
    let elapsed_ms = path.elapsed.as_secs_f64() * 1000.0;
    let leading_ms = duration_ms * (1.0 - trail_size).clamp(0.0, 1.0);
    let trailing_ms = duration_ms;
    std::array::from_fn(|index| {
        let corner_duration = match ranks[index] {
            2..=3 => leading_ms,
            1 => (leading_ms + trailing_ms) * 0.5,
            _ => trailing_ms,
        };
        let progress = critically_damped_progress(elapsed_ms, corner_duration);
        lerp_point(from[index], target[index], progress)
    })
}

#[cfg(test)]
fn cursor_corners(
    position: CursorDrawPosition,
    shape: CursorShape,
    line_height: f64,
    cell_width: f64,
) -> [CursorPoint; 4] {
    let center = cursor_center(position, shape, line_height, cell_width);
    let (width, height) = cursor_size(shape, line_height, cell_width);
    [
        CursorPoint {
            x: center.x - width * 0.5,
            y: center.y - height * 0.5,
        },
        CursorPoint {
            x: center.x + width * 0.5,
            y: center.y - height * 0.5,
        },
        CursorPoint {
            x: center.x + width * 0.5,
            y: center.y + height * 0.5,
        },
        CursorPoint {
            x: center.x - width * 0.5,
            y: center.y + height * 0.5,
        },
    ]
}

fn cursor_center(
    position: CursorDrawPosition,
    shape: CursorShape,
    line_height: f64,
    cell_width: f64,
) -> CursorPoint {
    let (width, height) = cursor_size(shape, line_height, cell_width);
    CursorPoint {
        x: position.column * cell_width + width * 0.5,
        y: position.line * line_height + height * 0.5,
    }
}

fn cursor_size(shape: CursorShape, line_height: f64, cell_width: f64) -> (f64, f64) {
    match shape {
        CursorShape::Bar => (cursor_size_cells(shape).0 * cell_width, line_height),
        CursorShape::Block => (cursor_size_cells(shape).0 * cell_width, line_height),
    }
}

fn cursor_size_cells(shape: CursorShape) -> (f64, f64) {
    match shape {
        CursorShape::Bar => (1.0 / 8.0, 1.0),
        CursorShape::Block => (1.0, 1.0),
    }
}

fn neovide_corner_relative_positions(shape: CursorShape) -> [CursorPoint; 4] {
    match shape {
        CursorShape::Bar => {
            let width = cursor_size_cells(shape).0;
            [
                CursorPoint { x: -0.5, y: -0.5 },
                CursorPoint {
                    x: width - 0.5,
                    y: -0.5,
                },
                CursorPoint {
                    x: width - 0.5,
                    y: 0.5,
                },
                CursorPoint { x: -0.5, y: 0.5 },
            ]
        }
        CursorShape::Block => [
            CursorPoint { x: -0.5, y: -0.5 },
            CursorPoint { x: 0.5, y: -0.5 },
            CursorPoint { x: 0.5, y: 0.5 },
            CursorPoint { x: -0.5, y: 0.5 },
        ],
    }
}

fn neovide_corner_ranks_grid(
    current_corners: [CursorPoint; 4],
    target: CursorDrawPosition,
    shape: CursorShape,
) -> [usize; 4] {
    let target_center = CursorPoint {
        x: target.column + 0.5,
        y: target.line + 0.5,
    };
    let relative = neovide_corner_relative_positions(shape);
    let mut alignments = relative
        .iter()
        .zip(current_corners)
        .enumerate()
        .map(|(index, (relative, current))| {
            let destination = CursorPoint {
                x: target_center.x + relative.x,
                y: target_center.y + relative.y,
            };
            let travel = normalize_point(CursorPoint {
                x: destination.x - current.x,
                y: destination.y - current.y,
            });
            let corner = normalize_point(*relative);
            let alignment = (travel.x * corner.x) + (travel.y * corner.y);
            (index, alignment)
        })
        .collect::<Vec<_>>();
    alignments.sort_by(|left, right| {
        left.1
            .partial_cmp(&right.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(left.0.cmp(&right.0))
    });
    let mut ranks = [0usize; 4];
    for (rank, (index, _)) in alignments.into_iter().enumerate() {
        ranks[index] = rank;
    }
    ranks
}

fn normalize_point(point: CursorPoint) -> CursorPoint {
    let length = (point.x.powi(2) + point.y.powi(2)).sqrt();
    if length <= f64::EPSILON {
        return CursorPoint { x: 0.0, y: 0.0 };
    }
    CursorPoint {
        x: point.x / length,
        y: point.y / length,
    }
}

fn cursor_rect_grid(position: CursorDrawPosition, shape: CursorShape) -> SmearRect {
    let (width, height) = cursor_size_cells(shape);
    SmearRect {
        left: position.column,
        right: position.column + width,
        top: position.line,
        height,
    }
}

fn cursor_corners_grid(position: CursorDrawPosition, shape: CursorShape) -> [CursorPoint; 4] {
    cursor_rect_grid(position, shape).points()
}

fn points_to_pixels(
    points: [CursorPoint; 4],
    line_height: f64,
    cell_width: f64,
) -> [CursorPoint; 4] {
    points.map(|point| CursorPoint {
        x: point.x * cell_width,
        y: point.y * line_height,
    })
}

fn cursor_alpha(shape: CursorShape) -> f64 {
    match shape {
        CursorShape::Bar => 1.0,
        CursorShape::Block => 0.72,
    }
}

fn neovide_cursor_alpha(shape: CursorShape) -> f64 {
    match shape {
        CursorShape::Bar => 1.0,
        CursorShape::Block => crate::config::cursor_neovide_block_opacity(),
    }
}

#[cfg(test)]
fn critically_damped_progress(elapsed_ms: f64, duration_ms: f64) -> f64 {
    if duration_ms <= 1.0 || elapsed_ms >= duration_ms {
        return 1.0;
    }
    let time = (elapsed_ms / duration_ms).clamp(0.0, 1.0);
    let residual = (1.0 + (6.0 * time)) * (-6.0 * time).exp();
    (1.0 - residual).clamp(0.0, 1.0)
}

#[cfg(test)]
fn lerp_point(from: CursorPoint, target: CursorPoint, progress: f64) -> CursorPoint {
    CursorPoint {
        x: from.x + ((target.x - from.x) * progress),
        y: from.y + ((target.y - from.y) * progress),
    }
}

fn squared_distance(left: CursorPoint, right: CursorPoint) -> f64 {
    (left.x - right.x).powi(2) + (left.y - right.y).powi(2)
}

fn draw_cursor_polygon(context: &cairo::Context, points: [CursorPoint; 4]) {
    context.move_to(points[0].x.round(), points[0].y.round());
    for point in points.iter().skip(1) {
        context.line_to(point.x.round(), point.y.round());
    }
    context.close_path();
    let _ = context.fill();
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
    let (width, height) = cursor_size(shape, line_height, cell_width);
    match shape {
        CursorShape::Bar => {
            context.set_source_rgb(125.0 / 255.0, 211.0 / 255.0, 252.0 / 255.0);
            context.rectangle(x.round(), y.round(), width, height);
        }
        CursorShape::Block => {
            context.set_source_rgba(125.0 / 255.0, 211.0 / 255.0, 252.0 / 255.0, 0.72);
            context.rectangle(x.round(), y.round(), width, height);
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
    font_size_pt: f64,
) {
    let x = preedit.column.max(0) as f64 * cell_width;
    let y = preedit.line.max(0) as f64 * line_height;
    let columns = preedit.columns.max(1);
    context.set_source_rgb(37.0 / 255.0, 41.0 / 255.0, 48.0 / 255.0);
    context.rectangle(x, y, columns as f64 * cell_width, line_height);
    let _ = context.fill();

    let style = RenderStyle {
        fg: Some(default_terminal_palette().foreground.to_string()),
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
        markup: run_markup(&style, &preedit.text),
        style,
    };
    let layout = layout_for_size(widget, &run.markup, font_size_pt);
    gtk::render_layout(&widget.style_context(), context, x, y, &layout);

    if render.cursor.visible {
        let cursor_x =
            (preedit.column.max(0) as usize + preedit.cursor_columns) as f64 * cell_width;
        context.set_source_rgb(125.0 / 255.0, 211.0 / 255.0, 252.0 / 255.0);
        context.rectangle(cursor_x.round(), y.round(), 1.25, line_height);
        let _ = context.fill();
    }
}

fn run_markup(style: &RenderStyle, text: &str) -> String {
    let mut span = String::from("<span");
    push_style_markup(&mut span, style);
    span.push('>');
    for ch in text.chars() {
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
    use crate::render::{RenderLine, RenderRegion, RenderRun};
    use serial_test::serial;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_config_dir(prefix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "chelotype-{prefix}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ))
    }

    #[test]
    fn run_markup_escapes_text_and_preserves_style() {
        let style = RenderStyle {
            fg: Some("#ff0000".to_string()),
            bg: None,
            bold: true,
            italic: true,
            underline: true,
            strikeout: true,
            selected: false,
        };
        let run = RenderRun {
            start_column: 0,
            columns: 1,
            text: "<&>".to_string(),
            markup: run_markup(&style, "<&>"),
            style,
        };
        assert!(run.markup.contains("foreground=\"#ff0000\""));
        assert!(run.markup.contains("weight=\"bold\""));
        assert!(run.markup.contains("style=\"italic\""));
        assert!(run.markup.contains("underline=\"single\""));
        assert!(run.markup.contains("strikethrough=\"true\""));
        assert!(run.markup.contains("&lt;&amp;&gt;"));
    }

    #[test]
    fn row_surface_key_reuses_same_line_content_after_row_shift() {
        let style = RenderStyle {
            fg: Some(default_terminal_palette().foreground.to_string()),
            bg: None,
            bold: false,
            italic: false,
            underline: false,
            strikeout: false,
            selected: false,
        };
        let run = RenderRun {
            start_column: 0,
            columns: 5,
            text: "hello".to_string(),
            markup: run_markup(&style, "hello"),
            style,
        };
        let first = RenderLine::new(
            4,
            RenderRegion::History,
            "hello".to_string(),
            String::new(),
            Vec::new(),
            vec![run.clone()],
        );
        let shifted = RenderLine::new(
            3,
            RenderRegion::History,
            "hello".to_string(),
            String::new(),
            Vec::new(),
            vec![run],
        );

        assert_eq!(first.paint_key, shifted.paint_key);
        assert_eq!(
            RowSurfaceLineKey::from(&first),
            RowSurfaceLineKey::from(&shifted)
        );
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
        let hidden = state.tick(true, true, start + CURSOR_BLINK_PERIOD);
        assert!(!hidden.visible);
        assert!(
            !hidden
                .sync(
                    cursor,
                    start + CURSOR_BLINK_PERIOD + Duration::from_millis(1)
                )
                .visible
        );
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
                    true,
                    start + CURSOR_BLINK_PERIOD + Duration::from_millis(120)
                )
                .visible
        );
    }

    #[test]
    fn cursor_blink_disabled_keeps_cursor_visible() {
        let start = Instant::now();
        let cursor = CursorIdentity {
            pane_id: 0,
            line: 1,
            column: 2,
            visible: true,
        };
        let state = CursorBlinkState::default().sync(cursor, start);
        let visible = state.tick(false, false, start + CURSOR_BLINK_PERIOD);
        assert!(!visible.visible);

        let visible = state.tick(true, false, start + CURSOR_BLINK_PERIOD);
        assert!(visible.visible);
        assert_eq!(visible.reset_at, None);
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

        let state =
            CursorMotionState::default().sync(first, start, CursorStyle::Smooth, CursorShape::Bar);
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
            CursorShape::Bar,
        );
        let animation_duration = cursor_animation_duration();
        let halfway = moved
            .position(start + Duration::from_millis(1) + (animation_duration / 2))
            .expect("animated cursor position");
        assert!(halfway.column > 2.0);
        assert!(halfway.column < 10.0);
        assert!(moved.active(start + Duration::from_millis(1)));
        assert!(!moved.active(start + Duration::from_millis(1) + animation_duration));
        assert_eq!(
            moved.position(start + Duration::from_millis(1) + animation_duration),
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

        let state =
            CursorMotionState::default().sync(first, start, CursorStyle::Smooth, CursorShape::Bar);
        let moved = state.sync(
            second,
            start + Duration::from_millis(1),
            CursorStyle::Steady,
            CursorShape::Bar,
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
    fn cursor_motion_does_not_animate_when_cursor_reenters_viewport() {
        let start = Instant::now();
        let hidden = CursorIdentity {
            pane_id: 0,
            line: -1,
            column: -1,
            visible: false,
        };
        let visible = CursorIdentity {
            pane_id: 0,
            line: 12,
            column: 8,
            visible: true,
        };

        let state =
            CursorMotionState::default().sync(hidden, start, CursorStyle::Smooth, CursorShape::Bar);
        let reentered = state.sync(
            visible,
            start + Duration::from_millis(1),
            CursorStyle::Smooth,
            CursorShape::Bar,
        );

        assert!(!reentered.active(start + Duration::from_millis(1)));
        assert_eq!(
            reentered.position(start + Duration::from_millis(1)),
            Some(CursorDrawPosition {
                pane_id: 0,
                line: 12.0,
                column: 8.0
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

        let state =
            CursorMotionState::default().sync(first, start, CursorStyle::Smear, CursorShape::Bar);
        let moved = state.sync(
            second,
            start + Duration::from_millis(1),
            CursorStyle::Smear,
            CursorShape::Bar,
        );
        let path = moved
            .path(start + Duration::from_millis(1) + (cursor_animation_duration() / 2))
            .expect("cursor trail path");
        let points = moved
            .advance(start + Duration::from_millis(18), CursorShape::Bar)
            .smear_points_grid(CursorShape::Bar);

        assert_eq!(path.from.pane_id, 7);
        assert_eq!(path.target.column, 14.0);
        assert_eq!(points[0].x, points[3].x, "{points:?}");
        assert_eq!(points[1].x, points[2].x, "{points:?}");
        assert_eq!(points[0].y, points[1].y, "{points:?}");
        assert_eq!(points[2].y, points[3].y, "{points:?}");
        assert!((points[2].y - points[0].y - 1.0).abs() < f64::EPSILON);
        assert!(points[1].x > points[0].x, "{points:?}");
        assert_eq!(moved.for_pane(7), Some(moved));
        assert_eq!(moved.for_pane(8), None);
    }

    #[test]
    fn smear_motion_keeps_axis_aligned_rect_across_frames_and_retargets() {
        let start = Instant::now();
        let first = CursorIdentity {
            pane_id: 7,
            line: 2,
            column: 4,
            visible: true,
        };
        let second = CursorIdentity {
            line: 4,
            column: 24,
            ..first
        };
        let third = CursorIdentity {
            line: 7,
            column: 1,
            ..first
        };

        let state =
            CursorMotionState::default().sync(first, start, CursorStyle::Smear, CursorShape::Bar);
        let moved = state.sync(
            second,
            start + Duration::from_millis(1),
            CursorStyle::Smear,
            CursorShape::Bar,
        );
        for offset_ms in [2, 18, 35, 70, 120] {
            assert_axis_aligned_cursor_points(
                moved
                    .advance(start + Duration::from_millis(offset_ms), CursorShape::Bar)
                    .smear_points_grid(CursorShape::Bar),
                1.0,
            );
        }

        let retargeted = moved
            .advance(start + Duration::from_millis(35), CursorShape::Bar)
            .sync(
                third,
                start + Duration::from_millis(36),
                CursorStyle::Smear,
                CursorShape::Bar,
            );
        for offset_ms in [37, 54, 90, 140] {
            assert_axis_aligned_cursor_points(
                retargeted
                    .advance(start + Duration::from_millis(offset_ms), CursorShape::Bar)
                    .smear_points_grid(CursorShape::Bar),
                1.0,
            );
        }
    }

    #[test]
    #[serial]
    fn cursor_motion_settles_when_animation_duration_elapses() {
        let dir = temp_config_dir("cursor-motion-settles");
        unsafe {
            std::env::set_var("CHELOTYPE_CONFIG_DIR", &dir);
        }
        let start = Instant::now();
        let first = CursorIdentity {
            pane_id: 0,
            line: 1,
            column: 2,
            visible: true,
        };
        let second = CursorIdentity {
            column: 12,
            ..first
        };

        let state =
            CursorMotionState::default().sync(first, start, CursorStyle::Neovide, CursorShape::Bar);
        let moved = state.sync(
            second,
            start + Duration::from_millis(1),
            CursorStyle::Neovide,
            CursorShape::Bar,
        );
        let finished_at = start + Duration::from_millis(1) + cursor_animation_duration();
        let settled = moved.settle_if_complete(finished_at, CursorShape::Bar);

        assert_ne!(settled, moved);
        assert!(!settled.active(finished_at));
        assert_eq!(
            settled.path(finished_at),
            Some(CursorDrawPath {
                from: CursorDrawPosition {
                    pane_id: 0,
                    line: 1.0,
                    column: 12.0,
                },
                current: CursorDrawPosition {
                    pane_id: 0,
                    line: 1.0,
                    column: 12.0,
                },
                target: CursorDrawPosition {
                    pane_id: 0,
                    line: 1.0,
                    column: 12.0,
                },
                progress: 1.0,
                elapsed: cursor_animation_duration(),
            })
        );
    }

    #[test]
    #[serial]
    fn neovide_bar_uses_skinny_quad_trail() {
        let dir = temp_config_dir("neovide-bar-quad");
        unsafe {
            std::env::set_var("CHELOTYPE_CONFIG_DIR", &dir);
        }
        crate::config::write_value("cursor_animation_duration_ms", "150");
        crate::config::write_value("cursor_neovide_trail_size", "0.65");

        let path = CursorDrawPath {
            from: CursorDrawPosition {
                pane_id: 0,
                line: 1.0,
                column: 1.0,
            },
            current: CursorDrawPosition {
                pane_id: 0,
                line: 1.0,
                column: 2.0,
            },
            target: CursorDrawPosition {
                pane_id: 0,
                line: 1.0,
                column: 8.0,
            },
            progress: 0.2,
            elapsed: Duration::from_millis(10),
        };

        let points = neovide_corners(path, CursorShape::Bar, 20.0, 10.0);

        assert!(points[1].x > points[0].x + 1.25, "{points:?}");
        assert!(points[2].x > points[3].x + 1.25, "{points:?}");
        assert!(points[3].x > points[0].x, "{points:?}");

        unsafe {
            std::env::remove_var("CHELOTYPE_CONFIG_DIR");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn neovide_corner_ranks_use_each_corner_position_after_retarget() {
        let current = [
            CursorPoint { x: 4.0, y: 1.0 },
            CursorPoint { x: 8.125, y: 1.0 },
            CursorPoint { x: 8.125, y: 2.0 },
            CursorPoint { x: 3.0, y: 2.0 },
        ];
        let target = CursorDrawPosition {
            pane_id: 0,
            line: 1.0,
            column: 2.0,
        };

        let ranks = neovide_corner_ranks_grid(current, target, CursorShape::Bar);

        assert_eq!(ranks, [2, 0, 1, 3]);
    }

    #[test]
    fn row_clip_intersection_keeps_partially_visible_rows() {
        assert!(row_intersects_clip(20.0, 10.0, 25.0, 35.0));
        assert!(row_intersects_clip(20.0, 10.0, 30.0, 40.0));
        assert!(row_intersects_clip(20.0, 10.0, 10.0, 20.0));
        assert!(!row_intersects_clip(20.0, 10.0, 30.1, 40.0));
        assert!(!row_intersects_clip(20.0, 10.0, 0.0, 19.9));
    }

    fn assert_axis_aligned_cursor_points(points: [CursorPoint; 4], expected_height: f64) {
        assert_eq!(points[0].x, points[3].x, "{points:?}");
        assert_eq!(points[1].x, points[2].x, "{points:?}");
        assert_eq!(points[0].y, points[1].y, "{points:?}");
        assert_eq!(points[2].y, points[3].y, "{points:?}");
        assert!((points[2].y - points[0].y - expected_height).abs() < 1e-9);
        assert!(points[1].x >= points[0].x, "{points:?}");
    }
}
