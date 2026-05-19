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
    pub frame: RenderFrame,
}

impl WorkspaceRenderFrame {
    pub fn from_active_tab_panes(
        panes: Vec<PaneRenderable>,
        active_selection: Option<SelectionRange>,
    ) -> Self {
        Self {
            panes: panes
                .into_iter()
                .map(|pane| {
                    let selection = pane.active.then_some(active_selection).flatten();
                    PaneRenderFrame {
                        pane_id: pane.id.raw(),
                        index: pane.index,
                        active: pane.active,
                        frame: Renderer::render_frame_with_selection(pane.content, selection),
                    }
                })
                .collect(),
        }
    }
}
