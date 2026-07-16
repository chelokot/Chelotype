use crate::config::{
    CursorBlinkAnimation, CursorBlinking, CursorCornerStyle, CursorShape, CursorStyle,
};
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

const PALETTE_TRANSITION_DURATION: Duration = Duration::from_millis(300);
const MAX_TEXT_LAYOUT_CACHE_ENTRIES: usize = 4096;
const MAX_ROW_SURFACE_CACHE_BYTES: usize = 64 * 1024 * 1024;
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
    cursor_options_override: Rc<Cell<Option<CursorOptionsOverride>>>,
    cursor_blink_animation_override: Rc<Cell<Option<CursorBlinkAnimation>>>,
    cursor_blinking_enabled_override: Rc<Cell<Option<bool>>>,
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
        let cursor_options_override = Rc::new(Cell::new(None::<CursorOptionsOverride>));
        let cursor_blink_animation_override = Rc::new(Cell::new(None::<CursorBlinkAnimation>));
        let cursor_blinking_enabled_override = Rc::new(Cell::new(None::<bool>));
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
        let draw_cursor_blink_animation_override = cursor_blink_animation_override.clone();
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
                        .map(CursorOptionsOverride::resolve)
                        .unwrap_or_else(CursorOptions::from_config),
                    cursor_blink_animation: draw_cursor_blink_animation_override
                        .get()
                        .unwrap_or_else(crate::config::cursor_blink_animation),
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
                        &transition.from,
                        None,
                        &mut paint_resources,
                        paint,
                        CanvasLayer {
                            width,
                            height,
                            alpha: 1.0,
                        },
                    );
                    draw_canvas_render_layer(
                        widget,
                        context,
                        render,
                        draw_scroll_underlay.borrow().as_ref(),
                        &mut paint_resources,
                        paint,
                        CanvasLayer {
                            width,
                            height,
                            alpha: progress,
                        },
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
                        render,
                        draw_scroll_underlay.borrow().as_ref(),
                        &mut paint_resources,
                        paint,
                        CanvasLayer {
                            width,
                            height,
                            alpha: 1.0,
                        },
                    );
                }
                paint_resources
                    .stats
                    .record(paint_resources.row_surface_cache.bytes);
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
            cursor_blink_animation_override,
            cursor_blinking_enabled_override,
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

    pub fn set_cursor_options_override(&self, options: Option<CursorOptionsOverride>) {
        self.cursor_options_override.set(options);
        self.area.queue_draw();
    }

    pub fn set_font_size_override(&self, font_size_pt: Option<f64>) {
        self.font_size_override.set(font_size_pt);
        self.area.queue_draw();
    }

    pub fn set_cursor_blink_animation_override(&self, animation: Option<CursorBlinkAnimation>) {
        self.cursor_blink_animation_override.set(animation);
        self.area.queue_draw();
    }

    pub fn set_cursor_blinking_enabled_override(&self, enabled: Option<bool>) {
        self.cursor_blinking_enabled_override.set(enabled);
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
        let blink_enabled = self.cursor_blinking_enabled();
        let next = self
            .cursor_blink
            .get()
            .tick(cursor_visible, blink_enabled, now);
        let current_motion = self.cursor_motion.get();
        let next_motion = current_motion.settle_if_complete(now, self.cursor_options().shape);
        if current_motion != next_motion {
            self.cursor_motion.set(next_motion);
        }
        let smooth_blinking = cursor_visible
            && blink_enabled
            && self.cursor_blink_animation() == CursorBlinkAnimation::Smooth;
        if self.cursor_blink.get() != next
            || smooth_blinking
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
            .map(CursorOptionsOverride::resolve)
            .unwrap_or_else(CursorOptions::from_config)
    }

    fn cursor_blinking_enabled(&self) -> bool {
        if let Some(enabled) = self.cursor_blinking_enabled_override.get() {
            return enabled;
        }
        match crate::config::cursor_blinking() {
            CursorBlinking::FollowSystem => gtk::Settings::default()
                .map(|settings| settings.is_gtk_cursor_blink())
                .unwrap_or(true),
            CursorBlinking::Enabled => true,
            CursorBlinking::Disabled => false,
        }
    }

    fn cursor_blink_animation(&self) -> CursorBlinkAnimation {
        self.cursor_blink_animation_override
            .get()
            .unwrap_or_else(crate::config::cursor_blink_animation)
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
    render: &CanvasRenderFrame,
    scroll_underlay: Option<&CanvasRenderFrame>,
    paint_resources: &mut PaintResources<'_>,
    paint: CanvasPaint,
    layer: CanvasLayer,
) {
    let CanvasLayer {
        width,
        height,
        alpha,
    } = layer;
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
        opacity: paint
            .cursor_blink
            .opacity(paint.now, paint.cursor_blink_animation),
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
                cursor_paint,
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
    let Some(underlay) = scroll_underlay_paint(scroll_visual_offset_px, line_height, rect.height)
    else {
        return;
    };
    let _ = context.save();
    context.rectangle(0.0, underlay.clip_y, rect.width, underlay.clip_height);
    context.clip();
    draw_render_frame(
        widget,
        context,
        render,
        CursorPaintState::hidden(),
        metrics,
        paint_resources,
        PaintViewport {
            scroll_visual_offset_px: underlay.scroll_visual_offset_px,
            width: rect.width,
        },
    );
    let _ = context.restore();
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ScrollUnderlayPaint {
    clip_y: f64,
    clip_height: f64,
    scroll_visual_offset_px: f64,
}

fn scroll_underlay_paint(
    scroll_visual_offset_px: f64,
    line_height: f64,
    rect_height: f64,
) -> Option<ScrollUnderlayPaint> {
    if scroll_visual_offset_px.abs() < 0.1 || line_height <= 0.0 || rect_height <= 0.0 {
        return None;
    }
    let missing = scroll_visual_offset_px.abs().min(rect_height);
    let missing_lines = (scroll_visual_offset_px.abs() / line_height).ceil();
    let offset_correction = missing_lines * line_height * scroll_visual_offset_px.signum();
    let scroll_visual_offset_px = scroll_visual_offset_px - offset_correction;
    let clip_y = if offset_correction < 0.0 {
        rect_height - missing
    } else {
        0.0
    };
    Some(ScrollUnderlayPaint {
        clip_y,
        clip_height: missing,
        scroll_visual_offset_px,
    })
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

    let opacity = cursor_paint.opacity.clamp(0.0, 1.0);
    if render.cursor.visible && opacity > 0.0 {
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
                CursorPaintState {
                    opacity,
                    ..cursor_paint
                },
                metrics,
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
    opacity: f64,
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
    cursor_blink_animation: CursorBlinkAnimation,
    font_size_pt: f64,
    scroll_visual_offset_px: f64,
    now: Instant,
}

#[derive(Clone, Copy)]
struct CanvasLayer {
    width: i32,
    height: i32,
    alpha: f64,
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
    row_surface_evictions: u64,
}

impl PaintStats {
    fn record(&self, row_surface_cache_bytes: usize) {
        crate::perf_trace::record_counter("gtk_paint_rows", self.rows);
        crate::perf_trace::record_counter("gtk_layout_cache_hits", self.layout_hits);
        crate::perf_trace::record_counter("gtk_layout_cache_misses", self.layout_misses);
        crate::perf_trace::record_counter("gtk_row_surface_hits", self.row_surface_hits);
        crate::perf_trace::record_counter("gtk_row_surface_misses", self.row_surface_misses);
        crate::perf_trace::record_counter("gtk_row_surface_evictions", self.row_surface_evictions);
        crate::perf_trace::record_counter(
            "gtk_row_surface_cache_bytes",
            row_surface_cache_bytes as u64,
        );
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
    order: VecDeque<RowSurfaceOrderKey>,
    bytes: usize,
    next_id: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RowSurfaceOrderKey {
    bucket: RowSurfaceBucketKey,
    id: u64,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct RowSurfaceBucketKey {
    signature: TextLayoutCacheSignature,
    width_px: i32,
    height_px: i32,
    line_paint_key: u64,
}

struct RowSurfaceEntry {
    id: u64,
    line: RowSurfaceLineKey,
    surface: cairo::ImageSurface,
    bytes: usize,
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
        if let Some((surface, id)) = self
            .surfaces
            .get(&bucket)
            .and_then(|entries| entries.iter().find(|entry| entry.line.matches(line)))
            .map(|entry| (entry.surface.clone(), entry.id))
        {
            let key = RowSurfaceOrderKey { bucket, id };
            if let Some(index) = self.order.iter().position(|cached| cached == &key) {
                self.order.remove(index);
            }
            self.order.push_back(key);
            stats.row_surface_hits += 1;
            return Some(surface);
        }
        let surface = self.render_surface(widget, line, spec, text_layout_cache, stats)?;
        let surface_bytes = surface.stride() as usize * surface.height() as usize;
        while self.bytes.saturating_add(surface_bytes) > MAX_ROW_SURFACE_CACHE_BYTES
            && let Some(evicted) = self.order.pop_front()
        {
            self.remove(&evicted);
            stats.row_surface_evictions += 1;
        }
        if surface_bytes > MAX_ROW_SURFACE_CACHE_BYTES {
            stats.row_surface_misses += 1;
            return Some(surface);
        }
        let line_key = RowSurfaceLineKey::from(line);
        let id = self.next_id;
        self.next_id += 1;
        let key = RowSurfaceOrderKey { bucket, id };
        self.order.push_back(key);
        self.surfaces
            .entry(bucket)
            .or_default()
            .push(RowSurfaceEntry {
                id,
                line: line_key,
                surface: surface.clone(),
                bytes: surface_bytes,
            });
        self.bytes += surface_bytes;
        stats.row_surface_misses += 1;
        Some(surface)
    }

    fn remove(&mut self, key: &RowSurfaceOrderKey) {
        let Some(entries) = self.surfaces.get_mut(&key.bucket) else {
            return;
        };
        if let Some(index) = entries.iter().position(|entry| entry.id == key.id) {
            let entry = entries.remove(index);
            self.bytes = self.bytes.saturating_sub(entry.bytes);
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
    corners: CursorCornerStyle,
    width_ratio: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CursorOptionsOverride {
    pub style: Option<CursorStyle>,
    pub shape: Option<CursorShape>,
    pub corners: Option<CursorCornerStyle>,
    pub width_ratio: Option<f64>,
}

impl CursorOptionsOverride {
    fn resolve(self) -> CursorOptions {
        let config = CursorOptions::from_config();
        CursorOptions {
            style: self.style.unwrap_or(config.style),
            shape: self.shape.unwrap_or(config.shape),
            corners: self.corners.unwrap_or(config.corners),
            width_ratio: self.width_ratio.unwrap_or(config.width_ratio),
        }
    }
}

impl CursorOptions {
    fn from_config() -> Self {
        Self {
            style: crate::config::cursor_style(),
            shape: crate::config::cursor_shape(),
            corners: crate::config::cursor_corner_style(),
            width_ratio: crate::config::cursor_width_ratio(),
        }
    }
}

impl CursorPaintState {
    fn hidden() -> Self {
        Self {
            opacity: 0.0,
            pane_id: 0,
            motion: CursorMotionState::default(),
            options: CursorOptions::from_config(),
            now: Instant::now(),
        }
    }

    fn for_pane(self, pane_id: u64, active: bool) -> Self {
        Self {
            opacity: if active { self.opacity } else { 0.0 },
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
        let elapsed_periods = now.saturating_duration_since(reset_at).as_millis()
            / cursor_blink_interval().as_millis();
        Self {
            visible: elapsed_periods.is_multiple_of(2),
            cursor: self.cursor,
            reset_at: Some(reset_at),
        }
    }

    fn opacity(self, now: Instant, animation: CursorBlinkAnimation) -> f64 {
        if self.cursor.is_none() {
            return 0.0;
        }
        if animation == CursorBlinkAnimation::Instant {
            return if self.visible { 1.0 } else { 0.0 };
        }
        let Some(reset_at) = self.reset_at else {
            return if self.visible { 1.0 } else { 0.0 };
        };
        cursor_blink_smooth_opacity(
            now.saturating_duration_since(reset_at),
            cursor_blink_interval(),
        )
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
        if current_state.to_line == target.line && current_state.to_column == target.column {
            return current_state;
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
        };
        match style {
            CursorStyle::Neovide => next.sync_neovide_target(target, shape),
            CursorStyle::Smooth | CursorStyle::Snappy | CursorStyle::Steady => {}
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
        let Some(progress) = self.progress(now) else {
            return Some(CursorDrawPosition {
                pane_id: self.pane_id,
                line: self.to_line,
                column: self.to_column,
            });
        };
        let eased = match self.style {
            CursorStyle::Snappy => snappy_cursor_progress(progress),
            CursorStyle::Smooth | CursorStyle::Steady | CursorStyle::Neovide => {
                smooth_cursor_progress(progress)
            }
        };
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
            CursorStyle::Smooth | CursorStyle::Snappy => {
                self.progress(now).is_some_and(|progress| progress < 1.0)
            }
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
            CursorStyle::Smooth | CursorStyle::Snappy | CursorStyle::Steady => {}
        }
        next
    }

    fn sync_neovide_target(&mut self, target: CursorDrawPosition, shape: CursorShape) {
        let jump_vec = CursorPoint {
            x: target.column - self.from_column,
            y: target.line - self.from_line,
        };
        let short_jump = jump_vec.x.abs()
            <= crate::config::cursor_neovide_short_jump_distance() + 0.001
            && jump_vec.y.abs() <= 0.001;
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
}

fn cursor_animation_duration() -> Duration {
    Duration::from_millis(u64::from(crate::config::cursor_animation_duration_ms()))
}

fn cursor_blink_interval() -> Duration {
    Duration::from_millis(u64::from(crate::config::cursor_blink_interval_ms()))
}

fn cursor_blink_smooth_opacity(elapsed: Duration, interval: Duration) -> f64 {
    let interval_ms = interval.as_secs_f64().max(f64::EPSILON);
    let phase = (elapsed.as_secs_f64() % (interval_ms * 2.0)) / interval_ms;
    if phase < 1.0 {
        1.0 - ease_in_out_cubic(phase)
    } else {
        ease_out_quint(phase - 1.0)
    }
}

fn ease_out_quint(progress: f64) -> f64 {
    let inverse = 1.0 - progress.clamp(0.0, 1.0);
    1.0 - inverse.powi(5)
}

fn ease_in_out_cubic(progress: f64) -> f64 {
    let progress = progress.clamp(0.0, 1.0);
    if progress < 0.5 {
        4.0 * progress.powi(3)
    } else {
        1.0 - (-2.0 * progress + 2.0).powi(3) / 2.0
    }
}

fn smooth_cursor_progress(progress: f64) -> f64 {
    progress * ((((-5.4 * progress + 17.6) * progress - 20.6) * progress + 9.0) * progress + 0.4)
}

fn snappy_cursor_progress(progress: f64) -> f64 {
    let root = progress.cbrt();
    3.0 * root - (3.0 * root * root) + progress
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
    paint: CursorPaintState,
    metrics: TerminalFontMetrics,
) {
    let cursor_motion = paint.motion.for_pane(paint.pane_id);
    let cursor_options = paint.options;
    let now = paint.now;
    let opacity = paint.opacity;
    let line_height = metrics.line_height;
    let cell_width = metrics.cell_width;
    let color = parse_hex_color(&render.cursor.color).expect("valid terminal cursor color");
    let alpha = match (cursor_options.style, cursor_options.shape) {
        (CursorStyle::Neovide, shape) => neovide_cursor_alpha(shape),
        (_, CursorShape::Block) => crate::config::DEFAULT_NEOVIDE_BLOCK_OPACITY,
        (_, CursorShape::Bar) => 1.0,
    };
    context.set_source_rgba(color.red, color.green, color.blue, alpha * opacity);
    let target = CursorDrawPosition {
        pane_id: 0,
        line: f64::from(render.cursor.line.max(0)),
        column: f64::from(render.cursor.column.max(0)),
    };
    let path = cursor_motion.and_then(|motion| motion.path(now));
    match cursor_options.style {
        CursorStyle::Steady => draw_caret_at(context, target, cursor_options, metrics),
        CursorStyle::Smooth | CursorStyle::Snappy => draw_caret_at_fractional(
            context,
            path.map(|path| path.current).unwrap_or(target),
            cursor_options,
            metrics,
        ),
        CursorStyle::Neovide => {
            if let Some(motion) = cursor_motion {
                if motion.active(now) {
                    let points =
                        points_to_pixels(motion.neovide_points_grid(), line_height, cell_width);
                    draw_neovide_points(context, points);
                } else {
                    draw_caret_at(context, target, cursor_options, metrics);
                }
            } else {
                draw_caret_at(context, target, cursor_options, metrics);
            }
        }
    }
}

fn draw_neovide_points(context: &cairo::Context, points: [CursorPoint; 4]) {
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

#[cfg(test)]
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

#[cfg(test)]
fn cursor_size(shape: CursorShape, line_height: f64, cell_width: f64) -> (f64, f64) {
    cursor_size_with_ratio(
        shape,
        line_height,
        cell_width,
        crate::config::cursor_width_ratio(),
    )
}

fn cursor_size_with_ratio(
    shape: CursorShape,
    line_height: f64,
    cell_width: f64,
    width_ratio: f64,
) -> (f64, f64) {
    match shape {
        CursorShape::Bar => (
            cursor_size_cells_with_ratio(shape, width_ratio).0 * cell_width,
            line_height,
        ),
        CursorShape::Block => (
            cursor_size_cells_with_ratio(shape, width_ratio).0 * cell_width,
            line_height,
        ),
    }
}

fn cursor_size_cells(shape: CursorShape) -> (f64, f64) {
    cursor_size_cells_with_ratio(shape, crate::config::cursor_width_ratio())
}

fn cursor_size_cells_with_ratio(shape: CursorShape, width_ratio: f64) -> (f64, f64) {
    match shape {
        CursorShape::Bar => (width_ratio.clamp(0.05, 1.0), 1.0),
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

fn cursor_corners_grid(position: CursorDrawPosition, shape: CursorShape) -> [CursorPoint; 4] {
    let (width, height) = cursor_size_cells(shape);
    [
        CursorPoint {
            x: position.column,
            y: position.line,
        },
        CursorPoint {
            x: position.column + width,
            y: position.line,
        },
        CursorPoint {
            x: position.column + width,
            y: position.line + height,
        },
        CursorPoint {
            x: position.column,
            y: position.line + height,
        },
    ]
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
    options: CursorOptions,
    metrics: TerminalFontMetrics,
) {
    draw_caret_rect(
        context,
        position,
        options,
        metrics,
        CursorPixelSnap::Integer,
    );
}

fn draw_caret_at_fractional(
    context: &cairo::Context,
    position: CursorDrawPosition,
    options: CursorOptions,
    metrics: TerminalFontMetrics,
) {
    draw_caret_rect(
        context,
        position,
        options,
        metrics,
        CursorPixelSnap::Fractional,
    );
}

#[derive(Clone, Copy)]
enum CursorPixelSnap {
    Integer,
    Fractional,
}

fn draw_caret_rect(
    context: &cairo::Context,
    position: CursorDrawPosition,
    options: CursorOptions,
    metrics: TerminalFontMetrics,
    snap: CursorPixelSnap,
) {
    let CursorOptions {
        shape,
        corners,
        width_ratio,
        ..
    } = options;
    let TerminalFontMetrics {
        line_height,
        cell_width,
    } = metrics;
    let (x, y) = caret_pixel_position(position, line_height, cell_width, snap);
    let (width, height) = cursor_size_with_ratio(shape, line_height, cell_width, width_ratio);
    draw_caret_shape(context, corners, x, y, width, height);
    let _ = context.fill();
}

fn draw_caret_shape(
    context: &cairo::Context,
    corners: CursorCornerStyle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) {
    match corners {
        CursorCornerStyle::Square => context.rectangle(x, y, width, height),
        CursorCornerStyle::Rounded => {
            let radius = (width.min(height) * 0.45).max(1.0);
            draw_rounded_rectangle(context, x, y, width, height, radius);
        }
    }
}

fn snap_cursor_pixel(value: f64, snap: CursorPixelSnap) -> f64 {
    match snap {
        CursorPixelSnap::Integer => value.round(),
        CursorPixelSnap::Fractional => value,
    }
}

fn caret_pixel_position(
    position: CursorDrawPosition,
    line_height: f64,
    cell_width: f64,
    snap: CursorPixelSnap,
) -> (f64, f64) {
    (
        snap_cursor_pixel(position.column * cell_width, snap),
        snap_cursor_pixel(position.line * line_height, snap),
    )
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

    fn assert_underlay_paint(actual: Option<ScrollUnderlayPaint>, expected: ScrollUnderlayPaint) {
        let actual = actual.expect("scroll underlay paint");
        assert!((actual.clip_y - expected.clip_y).abs() < 1e-9, "{actual:?}");
        assert!(
            (actual.clip_height - expected.clip_height).abs() < 1e-9,
            "{actual:?}"
        );
        assert!(
            (actual.scroll_visual_offset_px - expected.scroll_visual_offset_px).abs() < 1e-9,
            "{actual:?}"
        );
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
    fn row_surface_cache_budget_holds_multiple_4k_viewports() {
        let row_bytes = 3840 * 24 * 4;

        assert_eq!(MAX_ROW_SURFACE_CACHE_BYTES / row_bytes, 182);
        assert!(MAX_ROW_SURFACE_CACHE_BYTES >= row_bytes * 180);
        assert!(MAX_ROW_SURFACE_CACHE_BYTES < row_bytes * 512);
    }

    #[test]
    fn scroll_underlay_paint_covers_multi_line_positive_offsets() {
        assert_underlay_paint(
            scroll_underlay_paint(51.35, 18.0, 400.0),
            ScrollUnderlayPaint {
                clip_y: 0.0,
                clip_height: 51.35,
                scroll_visual_offset_px: -2.65,
            },
        );
    }

    #[test]
    fn scroll_underlay_paint_covers_multi_line_negative_offsets() {
        assert_underlay_paint(
            scroll_underlay_paint(-59.25, 20.0, 400.0),
            ScrollUnderlayPaint {
                clip_y: 340.75,
                clip_height: 59.25,
                scroll_visual_offset_px: 0.75,
            },
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
        let hidden = state.tick(true, true, start + cursor_blink_interval());
        assert!(!hidden.visible);
        assert!(
            !hidden
                .sync(
                    cursor,
                    start + cursor_blink_interval() + Duration::from_millis(1)
                )
                .visible
        );
        let moved = hidden.sync(
            CursorIdentity {
                column: 3,
                ..cursor
            },
            start + cursor_blink_interval() + Duration::from_millis(1),
        );
        assert!(moved.visible);
        assert!(
            moved
                .tick(
                    true,
                    true,
                    start + cursor_blink_interval() + Duration::from_millis(120)
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
        let visible = state.tick(false, false, start + cursor_blink_interval());
        assert!(!visible.visible);

        let visible = state.tick(true, false, start + cursor_blink_interval());
        assert!(visible.visible);
        assert_eq!(visible.reset_at, None);
    }

    #[test]
    #[serial]
    fn smooth_cursor_blink_interpolates_opacity() {
        let dir = temp_config_dir("smooth-cursor-blink");
        unsafe {
            std::env::set_var("CHELOTYPE_CONFIG_DIR", &dir);
        }
        crate::config::write_value("cursor_blink_animation", "smooth");
        crate::config::write_value("cursor_blink_interval_ms", "1000");
        let start = Instant::now();
        let cursor = CursorIdentity {
            pane_id: 0,
            line: 1,
            column: 2,
            visible: true,
        };
        let state = CursorBlinkState::default().sync(cursor, start);

        assert_eq!(state.opacity(start, CursorBlinkAnimation::Smooth), 1.0);
        assert!(
            (state.opacity(
                start + Duration::from_millis(500),
                CursorBlinkAnimation::Smooth
            ) - 0.5)
                .abs()
                < 1e-12
        );
        assert_eq!(
            state.opacity(
                start + Duration::from_millis(1000),
                CursorBlinkAnimation::Smooth
            ),
            0.0
        );
        assert!(
            (state.opacity(
                start + Duration::from_millis(1250),
                CursorBlinkAnimation::Smooth
            ) - 0.7626953125)
                .abs()
                < 1e-12
        );
        assert!(
            (state.opacity(
                start + Duration::from_millis(1500),
                CursorBlinkAnimation::Smooth
            ) - 0.96875)
                .abs()
                < 1e-12
        );
        assert_eq!(
            state.opacity(
                start + Duration::from_millis(2000),
                CursorBlinkAnimation::Smooth
            ),
            1.0
        );

        unsafe {
            std::env::remove_var("CHELOTYPE_CONFIG_DIR");
        }
        let _ = std::fs::remove_dir_all(dir);
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
        let finished = moved
            .position(start + Duration::from_millis(1) + animation_duration)
            .expect("finished cursor position");
        assert_eq!(finished.pane_id, 0);
        assert_eq!(finished.line, 1.0);
        assert!((finished.column - 10.0).abs() < 1e-9, "{finished:?}");
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
    fn smooth_cursor_progress_matches_jetbrains_ease_curve() {
        let cases = [
            (0.0, 0.0),
            (0.05, 0.0400333125),
            (0.1, 0.11110600000000002),
            (0.2, 0.30163200000000007),
            (0.5, 0.80625),
            (1.0, 1.0),
        ];
        for (progress, expected) in cases {
            assert!(
                (smooth_cursor_progress(progress) - expected).abs() < 1e-12,
                "{progress}"
            );
        }
    }

    #[test]
    fn snappy_cursor_progress_matches_ninja_curve() {
        let cases = [
            (0.0, 0.0),
            (0.02, 0.6132833950600489),
            (0.05, 0.7480468071028801),
            (0.1, 0.8461462430742686),
            (0.2, 0.9284250749217013),
            (0.5, 0.9912200031099894),
            (1.0, 1.0),
        ];
        for (progress, expected) in cases {
            assert!(
                (snappy_cursor_progress(progress) - expected).abs() < 1e-12,
                "{progress}"
            );
        }
    }

    #[test]
    #[serial]
    fn snappy_cursor_motion_reaches_target_faster_than_smooth() {
        let dir = temp_config_dir("snappy-reaches-target");
        unsafe {
            std::env::set_var("CHELOTYPE_CONFIG_DIR", &dir);
        }
        let start = Instant::now();
        let first = CursorIdentity {
            pane_id: 7,
            line: 1,
            column: 0,
            visible: true,
        };
        let second = CursorIdentity {
            column: 100,
            ..first
        };

        let smooth = CursorMotionState::default()
            .sync(first, start, CursorStyle::Smooth, CursorShape::Bar)
            .sync(
                second,
                start + Duration::from_millis(1),
                CursorStyle::Smooth,
                CursorShape::Bar,
            );
        let snappy = CursorMotionState::default()
            .sync(first, start, CursorStyle::Snappy, CursorShape::Bar)
            .sync(
                second,
                start + Duration::from_millis(1),
                CursorStyle::Snappy,
                CursorShape::Bar,
            );
        let sample_at = start + Duration::from_millis(1) + (cursor_animation_duration() / 10);
        let smooth_position = smooth
            .position(sample_at)
            .expect("smooth cursor position")
            .column;
        let snappy_position = snappy
            .position(sample_at)
            .expect("snappy cursor position")
            .column;

        assert!(smooth_position > 0.0);
        assert!(snappy_position > 80.0, "{snappy_position}");
        assert!(
            snappy_position > smooth_position,
            "{smooth_position} {snappy_position}"
        );
        assert_eq!(snappy.for_pane(7), Some(snappy));
        assert_eq!(snappy.for_pane(8), None);

        unsafe {
            std::env::remove_var("CHELOTYPE_CONFIG_DIR");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[serial]
    fn snappy_cursor_motion_retargets_from_current_position() {
        let dir = temp_config_dir("snappy-retarget");
        unsafe {
            std::env::set_var("CHELOTYPE_CONFIG_DIR", &dir);
        }
        let start = Instant::now();
        let first = CursorIdentity {
            pane_id: 7,
            line: 1,
            column: 0,
            visible: true,
        };
        let second = CursorIdentity {
            column: 80,
            ..first
        };
        let third = CursorIdentity {
            column: 12,
            ..first
        };

        let moved = CursorMotionState::default()
            .sync(first, start, CursorStyle::Snappy, CursorShape::Bar)
            .sync(
                second,
                start + Duration::from_millis(1),
                CursorStyle::Snappy,
                CursorShape::Bar,
            );
        let retarget_at = start + Duration::from_millis(1) + (cursor_animation_duration() / 10);
        let before_retarget = moved
            .position(retarget_at)
            .expect("snappy cursor position before retarget");
        let retargeted = moved.sync(third, retarget_at, CursorStyle::Snappy, CursorShape::Bar);
        let path = retargeted.path(retarget_at).expect("snappy retarget path");

        assert_eq!(path.from, before_retarget);
        assert_eq!(path.target.column, 12.0);
        assert!(path.from.column > path.target.column);

        unsafe {
            std::env::remove_var("CHELOTYPE_CONFIG_DIR");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[serial]
    fn smooth_cursor_motion_keeps_same_target_animation_start() {
        let dir = temp_config_dir("smooth-same-target");
        unsafe {
            std::env::set_var("CHELOTYPE_CONFIG_DIR", &dir);
        }
        crate::config::write_value("cursor_animation_duration_ms", "100");
        let start = Instant::now();
        let first = CursorIdentity {
            pane_id: 7,
            line: 1,
            column: 0,
            visible: true,
        };
        let second = CursorIdentity {
            column: 80,
            ..first
        };

        let moved = CursorMotionState::default()
            .sync(first, start, CursorStyle::Smooth, CursorShape::Bar)
            .sync(
                second,
                start + Duration::from_millis(1),
                CursorStyle::Smooth,
                CursorShape::Bar,
            );
        let repeated = moved.sync(
            second,
            start + Duration::from_millis(30),
            CursorStyle::Smooth,
            CursorShape::Bar,
        );
        let finished_at = start + Duration::from_millis(101);

        assert_eq!(repeated.started_at, moved.started_at);
        assert_eq!(repeated.from_column, moved.from_column);
        let finished = repeated
            .position(finished_at)
            .expect("finished smooth cursor position");
        assert_eq!(finished.pane_id, 7);
        assert_eq!(finished.line, 1.0);
        assert!(
            (finished.column - 80.0).abs() < 1e-10,
            "{}",
            finished.column
        );

        unsafe {
            std::env::remove_var("CHELOTYPE_CONFIG_DIR");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn animated_caret_uses_fractional_pixel_position() {
        let position = CursorDrawPosition {
            pane_id: 0,
            line: 1.25,
            column: 2.5,
        };

        assert_eq!(
            caret_pixel_position(position, 20.0, 10.0, CursorPixelSnap::Integer),
            (25.0, 25.0)
        );
        assert_eq!(
            caret_pixel_position(position, 20.0, 10.0, CursorPixelSnap::Fractional),
            (25.0, 25.0)
        );

        let position = CursorDrawPosition {
            column: 2.53,
            ..position
        };
        assert_eq!(
            caret_pixel_position(position, 20.0, 10.0, CursorPixelSnap::Integer),
            (25.0, 25.0)
        );
        let (x, y) = caret_pixel_position(position, 20.0, 10.0, CursorPixelSnap::Fractional);
        assert!((x - 25.3).abs() < 1e-12, "{x}");
        assert_eq!(y, 25.0);
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
    #[serial]
    fn neovide_short_jump_distance_controls_fast_path() {
        let dir = temp_config_dir("neovide-short-jump-distance");
        unsafe {
            std::env::set_var("CHELOTYPE_CONFIG_DIR", &dir);
        }
        crate::config::write_value("cursor_animation_duration_ms", "120");
        crate::config::write_value("cursor_neovide_short_animation_duration_ms", "25");
        let start = Instant::now();
        let first = CursorIdentity {
            pane_id: 0,
            line: 1,
            column: 1,
            visible: true,
        };
        let second = CursorIdentity { column: 3, ..first };

        crate::config::write_value("cursor_neovide_short_jump_distance", "2");
        let short = CursorMotionState::default()
            .sync(first, start, CursorStyle::Neovide, CursorShape::Bar)
            .sync(
                second,
                start + Duration::from_millis(1),
                CursorStyle::Neovide,
                CursorShape::Bar,
            );
        assert!(
            short
                .neovide_corners
                .iter()
                .all(|corner| corner.animation_duration <= Duration::from_millis(25))
        );

        crate::config::write_value("cursor_neovide_short_jump_distance", "1");
        let long = CursorMotionState::default()
            .sync(first, start, CursorStyle::Neovide, CursorShape::Bar)
            .sync(
                second,
                start + Duration::from_millis(1),
                CursorStyle::Neovide,
                CursorShape::Bar,
            );
        assert!(
            long.neovide_corners
                .iter()
                .any(|corner| corner.animation_duration > Duration::from_millis(25))
        );

        unsafe {
            std::env::remove_var("CHELOTYPE_CONFIG_DIR");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn row_clip_intersection_keeps_partially_visible_rows() {
        assert!(row_intersects_clip(20.0, 10.0, 25.0, 35.0));
        assert!(row_intersects_clip(20.0, 10.0, 30.0, 40.0));
        assert!(row_intersects_clip(20.0, 10.0, 10.0, 20.0));
        assert!(!row_intersects_clip(20.0, 10.0, 30.1, 40.0));
        assert!(!row_intersects_clip(20.0, 10.0, 0.0, 19.9));
    }
}
