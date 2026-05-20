use crate::config::{CursorShape, CursorStyle};
use crate::render::{RenderFrame, RenderRegion, RenderRun, RenderStyle};
use crate::terminal_font::{TerminalFontMetrics, layout_for, metrics_for_widget};
use crate::workspace_render::WorkspaceRenderFrame;
use gtk::prelude::*;
use gtk::{cairo, pango};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant};

const CURSOR_BLINK_PERIOD: Duration = Duration::from_millis(530);
const MAX_TEXT_LAYOUT_CACHE_ENTRIES: usize = 4096;
const MAX_ROW_SURFACE_CACHE_ENTRIES: usize = 512;
#[derive(Clone)]
pub struct TerminalCanvas {
    area: gtk::DrawingArea,
    render: Rc<RefCell<Option<CanvasRenderFrame>>>,
    scroll_underlay: Rc<RefCell<Option<CanvasRenderFrame>>>,
    cursor_blink: Rc<Cell<CursorBlinkState>>,
    cursor_motion: Rc<Cell<CursorMotionState>>,
    scroll_visual_offset_px: Rc<Cell<f64>>,
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
        let scroll_underlay = Rc::new(RefCell::new(None::<CanvasRenderFrame>));
        let cursor_blink = Rc::new(Cell::new(CursorBlinkState::default()));
        let cursor_motion = Rc::new(Cell::new(CursorMotionState::default()));
        let scroll_visual_offset_px = Rc::new(Cell::new(0.0));
        let text_layout_cache = Rc::new(RefCell::new(TextLayoutCache::default()));
        let row_surface_cache = Rc::new(RefCell::new(RowSurfaceCache::default()));
        let last_paint_started = Rc::new(Cell::new(None::<Instant>));
        let draw_render = render.clone();
        let draw_scroll_underlay = scroll_underlay.clone();
        let draw_cursor_blink = cursor_blink.clone();
        let draw_cursor_motion = cursor_motion.clone();
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
            draw_background(context, width, height);
            if let Some(render) = draw_render.borrow().as_ref() {
                let mut text_layout_cache = draw_text_layout_cache.borrow_mut();
                let mut row_surface_cache = draw_row_surface_cache.borrow_mut();
                let mut paint_resources = PaintResources {
                    text_layout_cache: &mut text_layout_cache,
                    row_surface_cache: &mut row_surface_cache,
                    stats: PaintStats::default(),
                };
                draw_canvas_render(
                    widget,
                    context,
                    render,
                    draw_scroll_underlay.borrow().as_ref(),
                    &mut paint_resources,
                    CanvasPaint {
                        cursor_blink: draw_cursor_blink.get(),
                        cursor_motion: draw_cursor_motion.get(),
                        cursor_options: CursorOptions::from_config(),
                        scroll_visual_offset_px: draw_scroll_visual_offset_px.get(),
                        now,
                    },
                );
                paint_resources.stats.record();
            }
            crate::perf_trace::record_duration("gtk_paint", started.elapsed());
        });

        Self {
            area,
            render,
            scroll_underlay,
            cursor_blink,
            cursor_motion,
            scroll_visual_offset_px,
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
        let cursor_style = crate::config::cursor_style();
        self.cursor_motion
            .set(previous_motion.sync(identity, now, cursor_style));
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
        if self.scroll_visual_offset_px.get().abs() >= 0.1 {
            *self.scroll_underlay.borrow_mut() = current.clone();
        } else {
            self.scroll_underlay.borrow_mut().take();
        }
        *current = Some(render);
        self.area.queue_draw();
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
        let next = self.cursor_blink.get().tick(cursor_visible, now);
        let current_motion = self.cursor_motion.get();
        let next_motion = current_motion.settle_if_complete(now);
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
    scroll_underlay: Option<&CanvasRenderFrame>,
    paint_resources: &mut PaintResources<'_>,
    paint: CanvasPaint,
) {
    let Some(metrics) = metrics_for_widget(widget) else {
        return;
    };
    let cursor_paint = CursorPaintState {
        visible: paint.cursor_blink.visible,
        pane_id: 0,
        motion: paint.cursor_motion,
        options: paint.cursor_options,
        now: paint.now,
    };
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
                    width: f64::from(widget.allocated_width()),
                },
            );
            if let Some(CanvasRenderFrame::Single(underlay)) = scroll_underlay {
                draw_scroll_underlay_frame(
                    widget,
                    context,
                    underlay,
                    metrics,
                    paint_resources,
                    paint.scroll_visual_offset_px,
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
}

fn draw_scroll_underlay_frame(
    widget: &gtk::DrawingArea,
    context: &cairo::Context,
    render: &RenderFrame,
    metrics: TerminalFontMetrics,
    paint_resources: &mut PaintResources<'_>,
    scroll_visual_offset_px: f64,
) {
    if scroll_visual_offset_px.abs() < 0.1 {
        return;
    }
    let width = f64::from(widget.allocated_width());
    let height = f64::from(widget.allocated_height());
    draw_scroll_underlay_frame_in_rect(
        widget,
        context,
        render,
        metrics,
        paint_resources,
        scroll_visual_offset_px,
        PaintRect { width, height },
    );
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
    entries: VecDeque<RowSurfaceEntry>,
}

struct RowSurfaceEntry {
    key: RowSurfaceKey,
    surface: cairo::ImageSurface,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RowSurfaceKey {
    signature: TextLayoutCacheSignature,
    width_px: i32,
    height_px: i32,
    line: RowSurfaceLineKey,
}

#[derive(Clone, Debug, Eq, PartialEq)]
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
        let key = RowSurfaceKey {
            signature: TextLayoutCacheSignature::for_widget(widget),
            width_px: spec.width_px,
            height_px: spec.height_px,
            line: RowSurfaceLineKey::from(line),
        };
        if let Some(entry) = self.entries.iter().find(|entry| entry.key == key) {
            stats.row_surface_hits += 1;
            return Some(entry.surface.clone());
        }
        let surface = self.render_surface(widget, line, spec, text_layout_cache, stats)?;
        if self.entries.len() >= MAX_ROW_SURFACE_CACHE_ENTRIES {
            self.entries.pop_front();
        }
        self.entries.push_back(RowSurfaceEntry {
            key,
            surface: surface.clone(),
        });
        stats.row_surface_misses += 1;
        Some(surface)
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TextLayoutCacheSignature {
    font_size_tenths: u32,
    text_scale_micros: u32,
}

#[derive(Default)]
struct TextLayoutCache {
    signature: Option<TextLayoutCacheSignature>,
    layouts: HashMap<String, pango::Layout>,
}

enum CachedLayout {
    Hit(pango::Layout),
    Miss(pango::Layout),
}

impl TextLayoutCache {
    fn layout_for(&mut self, widget: &gtk::DrawingArea, markup: &str) -> CachedLayout {
        self.sync_signature(widget);
        if let Some(layout) = self.layouts.get(markup) {
            return CachedLayout::Hit(layout.clone());
        }
        if self.layouts.len() >= MAX_TEXT_LAYOUT_CACHE_ENTRIES {
            self.layouts.clear();
        }
        let layout = layout_for(widget, markup);
        self.layouts.insert(markup.to_string(), layout.clone());
        CachedLayout::Miss(layout)
    }

    fn sync_signature(&mut self, widget: &gtk::DrawingArea) {
        let signature = TextLayoutCacheSignature::for_widget(widget);
        if self.signature != Some(signature) {
            self.signature = Some(signature);
            self.layouts.clear();
        }
    }
}

impl TextLayoutCacheSignature {
    fn for_widget(widget: &gtk::DrawingArea) -> Self {
        Self {
            font_size_tenths: (crate::terminal_font::font_size_pt() * 10.0).round() as u32,
            text_scale_micros: (crate::terminal_font::text_scale_for_widget(widget) * 1_000_000.0)
                .round() as u32,
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
    pane_id: u64,
    from_line: f64,
    from_column: f64,
    to_line: f64,
    to_column: f64,
    started_at: Option<Instant>,
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

impl Default for CursorMotionState {
    fn default() -> Self {
        Self {
            initialized: false,
            visible: false,
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
        if self.cursor != Some(identity) || self.reset_at.is_none() {
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
        if style == CursorStyle::Steady || !identity.visible || !self.visible {
            return Self::settled(target, identity.visible);
        }
        let current = self.position(now);
        if current == Some(target) {
            return self;
        }
        let from = current.filter(|position| position.pane_id == target.pane_id);
        let from = from.unwrap_or(target);
        Self {
            initialized: true,
            visible: true,
            pane_id: target.pane_id,
            from_line: from.line,
            from_column: from.column,
            to_line: target.line,
            to_column: target.column,
            started_at: Some(now),
        }
    }

    fn settled(target: CursorDrawPosition, visible: bool) -> Self {
        Self {
            initialized: true,
            visible,
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
        self.progress(now).is_some_and(|progress| progress < 1.0)
    }

    fn settle_if_complete(self, now: Instant) -> Self {
        if self.progress(now).is_some_and(|progress| progress >= 1.0) {
            Self::settled(
                CursorDrawPosition {
                    pane_id: self.pane_id,
                    line: self.to_line,
                    column: self.to_column,
                },
                self.visible,
            )
        } else {
            self
        }
    }

    fn progress(self, now: Instant) -> Option<f64> {
        let started_at = self.started_at?;
        let elapsed = now.saturating_duration_since(started_at);
        Some((elapsed.as_secs_f64() / cursor_animation_duration().as_secs_f64()).clamp(0.0, 1.0))
    }
}

fn cursor_animation_duration() -> Duration {
    Duration::from_millis(u64::from(crate::config::cursor_animation_duration_ms()))
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
    draw_cursor_visual(
        context,
        target,
        path,
        cursor_options.style,
        cursor_options.shape,
        line_height,
        cell_width,
    );
}

pub(crate) fn draw_cursor_visual(
    context: &cairo::Context,
    target: CursorDrawPosition,
    path: Option<CursorDrawPath>,
    cursor_style: CursorStyle,
    cursor_shape: CursorShape,
    line_height: f64,
    cell_width: f64,
) {
    match cursor_style {
        CursorStyle::Steady => {
            draw_caret_at(context, target, cursor_shape, line_height, cell_width)
        }
        CursorStyle::Smooth => draw_caret_at(
            context,
            path.map(|path| path.current).unwrap_or(target),
            cursor_shape,
            line_height,
            cell_width,
        ),
        CursorStyle::Smear => {
            if let Some(path) = path {
                draw_smear_cursor(context, path, cursor_shape, line_height, cell_width);
            } else {
                draw_caret_at(context, target, cursor_shape, line_height, cell_width);
            }
        }
        CursorStyle::Neovide => {
            if let Some(path) = path {
                draw_neovide_cursor(context, path, cursor_shape, line_height, cell_width);
            } else {
                draw_caret_at(context, target, cursor_shape, line_height, cell_width);
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct CursorPoint {
    x: f64,
    y: f64,
}

fn draw_neovide_cursor(
    context: &cairo::Context,
    path: CursorDrawPath,
    shape: CursorShape,
    line_height: f64,
    cell_width: f64,
) {
    if path.progress >= 1.0 {
        draw_caret_at(context, path.target, shape, line_height, cell_width);
        return;
    }
    let points = neovide_corners(path, shape, line_height, cell_width);
    context.set_source_rgba(
        125.0 / 255.0,
        211.0 / 255.0,
        252.0 / 255.0,
        cursor_alpha(shape),
    );
    draw_cursor_polygon(context, points);
}

fn draw_smear_cursor(
    context: &cairo::Context,
    path: CursorDrawPath,
    shape: CursorShape,
    line_height: f64,
    cell_width: f64,
) {
    if path.progress >= 1.0 {
        draw_caret_at(context, path.target, shape, line_height, cell_width);
        return;
    }
    let mut points = smear_corners(path, shape, line_height, cell_width);
    let target_center = cursor_center(path.target, shape, line_height, cell_width);
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
    points = limit_smear_length(points, head, cell_width * 6.0);
    tail = points[0];
    for point in points {
        if squared_distance(point, head) > squared_distance(tail, head) {
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

fn limit_smear_length(
    points: [CursorPoint; 4],
    head: CursorPoint,
    max_length: f64,
) -> [CursorPoint; 4] {
    points.map(|point| {
        let distance = squared_distance(point, head).sqrt();
        if distance <= max_length {
            point
        } else {
            let factor = max_length / distance;
            CursorPoint {
                x: head.x + ((point.x - head.x) * factor),
                y: head.y + ((point.y - head.y) * factor),
            }
        }
    })
}

fn neovide_corners(
    path: CursorDrawPath,
    shape: CursorShape,
    line_height: f64,
    cell_width: f64,
) -> [CursorPoint; 4] {
    let from = cursor_corners(path.from, shape, line_height, cell_width);
    let target = cursor_corners(path.target, shape, line_height, cell_width);
    let ranks = corner_ranks(path.from, path.target, shape, line_height, cell_width);
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

fn smear_corners(
    path: CursorDrawPath,
    shape: CursorShape,
    line_height: f64,
    cell_width: f64,
) -> [CursorPoint; 4] {
    let from = cursor_corners(path.from, shape, line_height, cell_width);
    let target = cursor_corners(path.target, shape, line_height, cell_width);
    let target_center = cursor_center(path.target, shape, line_height, cell_width);
    let stiffnesses = smear_stiffnesses(from, target_center);
    let damping = crate::config::cursor_smear_damping();
    let elapsed_ms = path.elapsed.as_secs_f64() * 1000.0;
    std::array::from_fn(|index| {
        smear_corner_position(
            from[index],
            target[index],
            stiffnesses[index],
            damping,
            elapsed_ms,
        )
    })
}

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
        CursorShape::Bar => (1.25, line_height),
        CursorShape::Block => (cell_width.ceil(), line_height),
    }
}

fn cursor_alpha(shape: CursorShape) -> f64 {
    match shape {
        CursorShape::Bar => 1.0,
        CursorShape::Block => 0.72,
    }
}

fn corner_ranks(
    from: CursorDrawPosition,
    target: CursorDrawPosition,
    shape: CursorShape,
    line_height: f64,
    cell_width: f64,
) -> [usize; 4] {
    let from_center = cursor_center(from, shape, line_height, cell_width);
    let target_center = cursor_center(target, shape, line_height, cell_width);
    let travel_x = target_center.x - from_center.x;
    let travel_y = target_center.y - from_center.y;
    let travel_length = (travel_x.powi(2) + travel_y.powi(2)).sqrt();
    if travel_length <= f64::EPSILON {
        return [0, 1, 2, 3];
    }
    let travel_x = travel_x / travel_length;
    let travel_y = travel_y / travel_length;
    let relative = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
    let mut alignments = relative
        .iter()
        .enumerate()
        .map(|(index, (x, y))| (index, (x * travel_x) + (y * travel_y)))
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

fn smear_stiffnesses(from: [CursorPoint; 4], target_center: CursorPoint) -> [f64; 4] {
    let head_stiffness = crate::config::cursor_smear_stiffness();
    let trailing_stiffness = crate::config::cursor_smear_trailing_stiffness();
    let distances = from.map(|point| squared_distance(point, target_center).sqrt());
    let min_distance = distances.iter().copied().fold(f64::INFINITY, f64::min);
    let max_distance = distances.iter().copied().fold(0.0, f64::max);
    if (max_distance - min_distance).abs() <= f64::EPSILON {
        return [head_stiffness; 4];
    }
    distances.map(|distance| {
        let x = (distance - min_distance) / (max_distance - min_distance);
        (head_stiffness + ((trailing_stiffness - head_stiffness) * x.powi(3))).min(1.0)
    })
}

fn smear_corner_position(
    from: CursorPoint,
    target: CursorPoint,
    stiffness: f64,
    damping: f64,
    elapsed_ms: f64,
) -> CursorPoint {
    let mut current = from;
    let mut velocity = CursorPoint { x: 0.0, y: 0.0 };
    let mut remaining = elapsed_ms;
    while remaining > 0.0 {
        let step = remaining.min(17.0);
        let speed_correction = step / 17.0;
        let velocity_conservation = (1.0 - damping).powf(speed_correction);
        let damping_correction = 1.0 / (1.0 + (2.5 * velocity_conservation));
        let effective_stiffness =
            1.0 - (1.0 - (stiffness * damping_correction)).powf(speed_correction);
        velocity.x += (target.x - current.x) * effective_stiffness;
        velocity.y += (target.y - current.y) * effective_stiffness;
        current.x += velocity.x;
        current.y += velocity.y;
        velocity.x *= velocity_conservation;
        velocity.y *= velocity_conservation;
        remaining -= step;
    }
    current
}

fn critically_damped_progress(elapsed_ms: f64, duration_ms: f64) -> f64 {
    if duration_ms <= 1.0 || elapsed_ms >= duration_ms {
        return 1.0;
    }
    let time = (elapsed_ms / duration_ms).clamp(0.0, 1.0);
    let residual = (1.0 + (6.0 * time)) * (-6.0 * time).exp();
    (1.0 - residual).clamp(0.0, 1.0)
}

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
        markup: run_markup(&style, &preedit.text),
        style,
    };
    let layout = layout_for(widget, &run.markup);
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
            fg: Some("#e5e7eb".to_string()),
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
        let first = RenderLine {
            row: 4,
            region: RenderRegion::History,
            text: "hello".to_string(),
            markup: String::new(),
            cells: Vec::new(),
            runs: vec![run.clone()],
        };
        let shifted = RenderLine {
            row: 3,
            region: RenderRegion::History,
            text: "hello".to_string(),
            markup: String::new(),
            cells: Vec::new(),
            runs: vec![run],
        };

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
        let hidden = state.tick(true, start + CURSOR_BLINK_PERIOD);
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

        let state = CursorMotionState::default().sync(hidden, start, CursorStyle::Smooth);
        let reentered = state.sync(
            visible,
            start + Duration::from_millis(1),
            CursorStyle::Smooth,
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

        let state = CursorMotionState::default().sync(first, start, CursorStyle::Smear);
        let moved = state.sync(second, start + Duration::from_millis(1), CursorStyle::Smear);
        let path = moved
            .path(start + Duration::from_millis(1) + (cursor_animation_duration() / 2))
            .expect("cursor trail path");

        assert_eq!(path.from.pane_id, 7);
        assert_eq!(path.target.column, 14.0);
        assert!(path.current.column > path.from.column);
        assert!(path.current.column < path.target.column);
        assert_eq!(moved.for_pane(7), Some(moved));
        assert_eq!(moved.for_pane(8), None);
    }

    #[test]
    fn cursor_motion_settles_when_animation_duration_elapses() {
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

        let state = CursorMotionState::default().sync(first, start, CursorStyle::Neovide);
        let moved = state.sync(
            second,
            start + Duration::from_millis(1),
            CursorStyle::Neovide,
        );
        let finished_at = start + Duration::from_millis(1) + cursor_animation_duration();
        let settled = moved.settle_if_complete(finished_at);

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
    fn row_clip_intersection_keeps_partially_visible_rows() {
        assert!(row_intersects_clip(20.0, 10.0, 25.0, 35.0));
        assert!(row_intersects_clip(20.0, 10.0, 30.0, 40.0));
        assert!(row_intersects_clip(20.0, 10.0, 10.0, 20.0));
        assert!(!row_intersects_clip(20.0, 10.0, 30.1, 40.0));
        assert!(!row_intersects_clip(20.0, 10.0, 0.0, 19.9));
    }
}
