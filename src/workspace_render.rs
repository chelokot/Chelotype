use crate::render::{RenderFrame, Renderer};
use crate::selection::SelectionRange;
use crate::workspace::PaneRenderable;
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WorkspaceRenderFrame {
    pub panes: Vec<PaneRenderFrame>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PaneRenderFrame {
    pub pane_id: u64,
    pub index: usize,
    pub active: bool,
    pub origin_col: usize,
    pub origin_row: usize,
    pub cols: usize,
    pub rows: usize,
    pub frame: RenderFrame,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceRenderLayout {
    pub cols: usize,
    pub rows: usize,
    pub pane_cols: Vec<usize>,
}

impl WorkspaceRenderFrame {
    pub fn from_active_tab_panes(
        panes: Vec<PaneRenderable>,
        active_selection: Option<SelectionRange>,
    ) -> Self {
        Self::from_active_tab_panes_with_layout(panes, active_selection, None)
    }

    pub fn from_active_tab_panes_with_layout(
        panes: Vec<PaneRenderable>,
        active_selection: Option<SelectionRange>,
        layout: Option<WorkspaceRenderLayout>,
    ) -> Self {
        Self::from_active_tab_panes_with_layout_and_detail(
            panes,
            active_selection,
            layout,
            RenderFrameDetail::Full,
        )
    }

    pub fn from_active_tab_panes_for_paint_with_layout(
        panes: Vec<PaneRenderable>,
        active_selection: Option<SelectionRange>,
        layout: Option<WorkspaceRenderLayout>,
    ) -> Self {
        Self::from_active_tab_panes_with_layout_and_detail(
            panes,
            active_selection,
            layout,
            RenderFrameDetail::Paint,
        )
    }

    fn from_active_tab_panes_with_layout_and_detail(
        panes: Vec<PaneRenderable>,
        active_selection: Option<SelectionRange>,
        layout: Option<WorkspaceRenderLayout>,
        detail: RenderFrameDetail,
    ) -> Self {
        let pane_count = panes.len();
        Self {
            panes: panes
                .into_iter()
                .enumerate()
                .scan(0usize, |origin_col, (fallback_index, pane)| {
                    let selection = pane.active.then_some(active_selection).flatten();
                    let pane_layout = pane_layout(
                        layout.as_ref(),
                        pane_count,
                        fallback_index,
                        *origin_col,
                        &pane.content,
                    );
                    *origin_col += pane_layout.cols;
                    Some(PaneRenderFrame {
                        pane_id: pane.id.raw(),
                        index: pane.index,
                        active: pane.active,
                        origin_col: pane_layout.origin_col,
                        origin_row: pane_layout.origin_row,
                        cols: pane_layout.cols,
                        rows: pane_layout.rows,
                        frame: detail.render(&pane.content, selection),
                    })
                })
                .collect(),
        }
    }

    pub fn active_content_text(&self) -> String {
        self.panes
            .iter()
            .find(|pane| pane.active)
            .map(|pane| {
                pane.frame
                    .lines
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy)]
enum RenderFrameDetail {
    Full,
    Paint,
}

impl RenderFrameDetail {
    fn render(
        self,
        content: &crate::backend::RenderableContentOwned,
        selection: Option<SelectionRange>,
    ) -> RenderFrame {
        match self {
            Self::Full => Renderer::render_frame_with_selection(content, selection),
            Self::Paint => Renderer::render_frame_for_paint(content, selection),
        }
    }
}

#[derive(Clone, Copy)]
struct PaneLayout {
    origin_col: usize,
    origin_row: usize,
    cols: usize,
    rows: usize,
}

fn pane_layout(
    layout: Option<&WorkspaceRenderLayout>,
    pane_count: usize,
    index: usize,
    origin_col: usize,
    content: &crate::backend::RenderableContentOwned,
) -> PaneLayout {
    if let Some(layout) = layout {
        let cols = layout
            .pane_cols
            .get(index)
            .copied()
            .filter(|_| layout.pane_cols.len() == pane_count)
            .unwrap_or_else(|| {
                let base = layout.cols / pane_count.max(1);
                let remainder = layout.cols % pane_count.max(1);
                base + usize::from(index < remainder)
            });
        return PaneLayout {
            origin_col,
            origin_row: 0,
            cols,
            rows: layout.rows,
        };
    }
    PaneLayout {
        origin_col,
        origin_row: 0,
        cols: content.lines.iter().map(Vec::len).max().unwrap_or(0),
        rows: content.lines.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_grid::{MouseMode, TerminalCell, TerminalColors, TerminalContent};
    use crate::workspace::PaneId;

    fn pane(id: u64, index: usize, active: bool, text: &str) -> PaneRenderable {
        PaneRenderable {
            id: PaneId::from_raw(id),
            active,
            index,
            content: TerminalContent {
                lines: vec![
                    text.chars()
                        .map(|ch| TerminalCell {
                            text: ch.to_string().into(),
                            ..TerminalCell::blank()
                        })
                        .collect(),
                ],
                line_metadata: vec![],
                cursor_line: 0,
                cursor_col: 0,
                cursor_visible: active,
                display_offset: 0,
                colors: TerminalColors::default(),
                mouse: MouseMode::default(),
            },
        }
    }

    #[test]
    fn lays_out_active_tab_panes_across_columns() {
        let frame = WorkspaceRenderFrame::from_active_tab_panes_with_layout(
            vec![pane(10, 0, false, "left"), pane(11, 1, true, "right")],
            None,
            Some(WorkspaceRenderLayout {
                cols: 81,
                rows: 24,
                pane_cols: Vec::new(),
            }),
        );

        assert_eq!(frame.panes.len(), 2);
        assert_eq!(frame.panes[0].pane_id, 10);
        assert_eq!(frame.panes[0].origin_col, 0);
        assert_eq!(frame.panes[0].cols, 41);
        assert_eq!(frame.panes[0].rows, 24);
        assert_eq!(frame.panes[1].pane_id, 11);
        assert_eq!(frame.panes[1].origin_col, 41);
        assert_eq!(frame.panes[1].cols, 40);
        assert_eq!(frame.panes[1].rows, 24);
        assert!(frame.panes[1].active);
    }

    #[test]
    fn honors_explicit_active_tab_pane_columns() {
        let frame = WorkspaceRenderFrame::from_active_tab_panes_with_layout(
            vec![pane(10, 0, true, "left"), pane(11, 1, false, "right")],
            None,
            Some(WorkspaceRenderLayout {
                cols: 81,
                rows: 24,
                pane_cols: vec![61, 20],
            }),
        );

        assert_eq!(frame.panes[0].origin_col, 0);
        assert_eq!(frame.panes[0].cols, 61);
        assert_eq!(frame.panes[1].origin_col, 61);
        assert_eq!(frame.panes[1].cols, 20);
    }

    #[test]
    fn lays_out_unframed_panes_from_rendered_content_widths() {
        let frame = WorkspaceRenderFrame::from_active_tab_panes(
            vec![pane(10, 0, true, "wide-left"), pane(11, 1, false, "right")],
            None,
        );

        assert_eq!(frame.panes[0].origin_col, 0);
        assert_eq!(frame.panes[0].cols, "wide-left".len());
        assert_eq!(frame.panes[1].origin_col, "wide-left".len());
        assert_eq!(frame.panes[1].cols, "right".len());
    }
}
