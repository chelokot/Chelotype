use crate::backend::{RenderableContentOwned, ScreenSize, TerminalBackend};
use crate::containers::LaunchTarget;
use portable_pty::CommandBuilder;
use std::io;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TabId(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneId(u64);

impl PaneId {
    pub fn raw(self) -> u64 {
        self.0
    }

    pub fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TabInfo {
    pub id: TabId,
    pub active: bool,
    pub index: usize,
    pub pane_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneInfo {
    pub id: PaneId,
    pub active: bool,
    pub index: usize,
}

pub struct PaneRenderable {
    pub id: PaneId,
    pub active: bool,
    pub index: usize,
    pub content: RenderableContentOwned,
}

pub struct TerminalWorkspace {
    tabs: Vec<TerminalTab>,
    active_tab: TabId,
    next_tab_id: u64,
    next_pane_id: u64,
}

struct TerminalTab {
    id: TabId,
    title: String,
    launch_target: LaunchTarget,
    panes: Vec<TerminalPane>,
    active_pane: PaneId,
}

struct TerminalPane {
    id: PaneId,
    backend: TerminalBackend,
    width_weight: u32,
}

const DEFAULT_PANE_WEIGHT: u32 = 1;
const COMFORTABLE_MIN_PANE_COLS: usize = 8;

impl TerminalWorkspace {
    pub fn spawn_shell() -> io::Result<Self> {
        Self::spawn_launch_target(crate::containers::startup_launch_target())
    }

    pub fn spawn_launch_target(target: LaunchTarget) -> io::Result<Self> {
        Self::spawn_launch_target_with_size(target, None)
    }

    pub fn spawn_launch_target_with_size(
        target: LaunchTarget,
        size: Option<ScreenSize>,
    ) -> io::Result<Self> {
        let title = target.title();
        let command = target.command_with_size(size);
        Self::spawn_titled_with_target_size(title, command, target, size)
    }

    pub fn spawn_with(command: CommandBuilder) -> io::Result<Self> {
        Self::spawn_titled_with_target("Chelotype", command, LaunchTarget::Host)
    }

    pub(crate) fn spawn_titled_with_target(
        title: impl Into<String>,
        command: CommandBuilder,
        launch_target: LaunchTarget,
    ) -> io::Result<Self> {
        Self::spawn_titled_with_target_size(title, command, launch_target, None)
    }

    fn spawn_titled_with_target_size(
        title: impl Into<String>,
        command: CommandBuilder,
        launch_target: LaunchTarget,
        size: Option<ScreenSize>,
    ) -> io::Result<Self> {
        let first_tab_id = TabId(1);
        let first_pane_id = PaneId(1);
        Ok(Self {
            tabs: vec![TerminalTab {
                id: first_tab_id,
                title: title.into(),
                launch_target,
                panes: vec![TerminalPane {
                    id: first_pane_id,
                    backend: spawn_backend(command, size)?,
                    width_weight: DEFAULT_PANE_WEIGHT,
                }],
                active_pane: first_pane_id,
            }],
            active_tab: first_tab_id,
            next_tab_id: 2,
            next_pane_id: 2,
        })
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    pub fn active_tab_id(&self) -> TabId {
        self.active_tab
    }

    pub fn active_pane_id(&self) -> PaneId {
        self.active_tab().active_pane
    }

    pub fn add_tab_with(&mut self, command: CommandBuilder) -> io::Result<TabId> {
        self.add_titled_tab_with("Chelotype", command)
    }

    pub fn add_titled_tab_with(
        &mut self,
        title: impl Into<String>,
        command: CommandBuilder,
    ) -> io::Result<TabId> {
        self.add_titled_tab_with_target(title, command, LaunchTarget::Host)
    }

    pub fn add_launch_target_tab(&mut self, target: LaunchTarget) -> io::Result<TabId> {
        self.add_launch_target_tab_with_size(target, None)
    }

    pub fn add_launch_target_tab_with_size(
        &mut self,
        target: LaunchTarget,
        size: Option<ScreenSize>,
    ) -> io::Result<TabId> {
        let title = target.title();
        let command = target.command_with_size(size);
        self.add_titled_tab_with_target_size(title, command, target, size)
    }

    pub fn add_titled_tab_with_target(
        &mut self,
        title: impl Into<String>,
        command: CommandBuilder,
        launch_target: LaunchTarget,
    ) -> io::Result<TabId> {
        self.add_titled_tab_with_target_size(title, command, launch_target, None)
    }

    fn add_titled_tab_with_target_size(
        &mut self,
        title: impl Into<String>,
        command: CommandBuilder,
        launch_target: LaunchTarget,
        size: Option<ScreenSize>,
    ) -> io::Result<TabId> {
        let id = TabId(self.next_tab_id);
        self.next_tab_id += 1;
        let pane_id = PaneId(self.next_pane_id);
        self.next_pane_id += 1;
        self.tabs.push(TerminalTab {
            id,
            title: title.into(),
            launch_target,
            panes: vec![TerminalPane {
                id: pane_id,
                backend: spawn_backend(command, size)?,
                width_weight: DEFAULT_PANE_WEIGHT,
            }],
            active_pane: pane_id,
        });
        Ok(id)
    }

    pub fn split_active_with(&mut self, command: CommandBuilder) -> io::Result<PaneId> {
        self.split_active_with_size(command, None)
    }

    fn split_active_with_size(
        &mut self,
        command: CommandBuilder,
        size: Option<ScreenSize>,
    ) -> io::Result<PaneId> {
        let id = PaneId(self.next_pane_id);
        self.next_pane_id += 1;
        let tab = self.active_tab_mut();
        tab.panes.push(TerminalPane {
            id,
            backend: spawn_backend(command, size)?,
            width_weight: DEFAULT_PANE_WEIGHT,
        });
        tab.active_pane = id;
        Ok(id)
    }

    pub fn split_shell_active(&mut self) -> io::Result<PaneId> {
        self.split_shell_active_with_size(None)
    }

    pub fn split_shell_active_with_size(&mut self, size: Option<ScreenSize>) -> io::Result<PaneId> {
        let command = crate::shell::default_shell_command_with_size(size);
        let id = self.split_active_with_size(command, size)?;
        if let Some(size) = size {
            self.resize_active_tab(size)?;
        }
        Ok(id)
    }

    pub fn add_shell_tab(&mut self) -> io::Result<TabId> {
        self.add_tab_with(crate::shell::default_shell_command())
    }

    pub fn respawn_active_tab_with_size(&mut self, size: ScreenSize) -> io::Result<()> {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == self.active_tab) else {
            return Ok(());
        };
        let pane_id = PaneId(self.next_pane_id);
        self.next_pane_id += 1;
        let target = self.tabs[index].launch_target.clone();
        let command = target.command_with_size(Some(size));
        self.tabs[index].panes = vec![TerminalPane {
            id: pane_id,
            backend: TerminalBackend::spawn_with_size(command, size)?,
            width_weight: DEFAULT_PANE_WEIGHT,
        }];
        self.tabs[index].active_pane = pane_id;
        Ok(())
    }

    pub fn close(&mut self, id: TabId) -> bool {
        if self.tabs.len() <= 1 {
            return false;
        }
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return false;
        };
        self.tabs.remove(index);
        if self.active_tab == id {
            let next = index.min(self.tabs.len() - 1);
            self.active_tab = self.tabs[next].id;
        }
        true
    }

    pub fn close_others(&mut self, id: TabId) -> bool {
        if !self.tabs.iter().any(|tab| tab.id == id) {
            return false;
        }
        self.tabs.retain(|tab| tab.id == id);
        self.active_tab = id;
        true
    }

    pub fn rename(&mut self, id: TabId, title: impl Into<String>) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) else {
            return false;
        };
        let title = title.into();
        if title.trim().is_empty() {
            return false;
        }
        tab.title = title;
        true
    }

    pub fn move_left(&mut self, id: TabId) -> bool {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return false;
        };
        if index == 0 {
            return false;
        }
        self.tabs.swap(index - 1, index);
        true
    }

    pub fn move_right(&mut self, id: TabId) -> bool {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return false;
        };
        if index + 1 >= self.tabs.len() {
            return false;
        }
        self.tabs.swap(index, index + 1);
        true
    }

    pub fn reorder(&mut self, id: TabId, position: usize) -> bool {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return false;
        };
        let position = position.min(self.tabs.len() - 1);
        if index == position {
            return false;
        }
        let tab = self.tabs.remove(index);
        self.tabs.insert(position, tab);
        true
    }

    pub fn activate(&mut self, id: TabId) -> bool {
        if self.tabs.iter().any(|tab| tab.id == id) {
            self.active_tab = id;
            true
        } else {
            false
        }
    }

    pub fn activate_pane(&mut self, id: PaneId) -> bool {
        let tab = self.active_tab_mut();
        if tab.panes.iter().any(|pane| pane.id == id) {
            tab.active_pane = id;
            true
        } else {
            false
        }
    }

    pub fn activate_next(&mut self) {
        self.activate_relative(1);
    }

    pub fn activate_previous(&mut self) {
        self.activate_relative(-1);
    }

    pub fn activate_next_pane(&mut self) {
        self.activate_relative_pane(1);
    }

    pub fn activate_previous_pane(&mut self) {
        self.activate_relative_pane(-1);
    }

    pub fn tabs(&self) -> Vec<TabInfo> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| TabInfo {
                id: tab.id,
                active: tab.id == self.active_tab,
                index,
                pane_count: tab.panes.len(),
            })
            .collect()
    }

    pub fn active_tab_panes(&self) -> Vec<PaneInfo> {
        self.active_tab()
            .panes
            .iter()
            .enumerate()
            .map(|(index, pane)| PaneInfo {
                id: pane.id,
                active: pane.id == self.active_tab().active_pane,
                index,
            })
            .collect()
    }

    pub fn active_tab_pane_count(&self) -> usize {
        self.active_tab().panes.len()
    }

    pub fn tab_title(&self, id: TabId) -> Option<String> {
        self.tabs
            .iter()
            .find(|tab| tab.id == id)
            .map(|tab| tab.title.clone())
    }

    pub fn tab_launch_target(&self, id: TabId) -> Option<LaunchTarget> {
        self.tabs
            .iter()
            .find(|tab| tab.id == id)
            .map(|tab| tab.launch_target.clone())
    }

    pub fn tab_index(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    pub fn write_active(&mut self, data: &[u8]) -> io::Result<()> {
        let pane = self.active_pane_mut();
        pane.scroll_to_bottom()?;
        pane.write(data)
    }

    pub fn write_active_input_cursor_target(&mut self, offset: usize) -> io::Result<bool> {
        let pane = self.active_pane_mut();
        pane.scroll_to_bottom()?;
        pane.write_input_cursor_target(offset)
    }

    pub fn write_active_input_replace_range(
        &mut self,
        range: std::ops::Range<usize>,
        replacement: &str,
    ) -> io::Result<bool> {
        let pane = self.active_pane_mut();
        pane.scroll_to_bottom()?;
        pane.write_input_replace_range(range, replacement)
    }

    pub fn active_bracketed_paste_mode(&mut self) -> bool {
        self.active_pane_mut().bracketed_paste_mode()
    }

    pub fn resize_active(&mut self, size: ScreenSize) -> io::Result<()> {
        self.active_pane_mut().resize(size)
    }

    pub fn resize_active_tab(&mut self, size: ScreenSize) -> io::Result<()> {
        let tab = self.active_tab_mut();
        let cols = pane_columns_for_panes(&tab.panes, usize::from(size.cols));
        for (pane, cols) in tab.panes.iter_mut().zip(cols) {
            pane.backend.resize(ScreenSize::new(cols, size.rows)?)?;
        }
        Ok(())
    }

    pub fn active_tab_pane_columns(&self, total_cols: u16) -> Vec<u16> {
        pane_columns_for_panes(&self.active_tab().panes, usize::from(total_cols))
    }

    pub fn resize_active_tab_split(
        &mut self,
        boundary_index: usize,
        delta_cols: i16,
        size: ScreenSize,
    ) -> io::Result<bool> {
        let tab = self.active_tab_mut();
        if boundary_index + 1 >= tab.panes.len() || delta_cols == 0 {
            return Ok(false);
        }
        let mut cols = pane_columns_for_panes(&tab.panes, usize::from(size.cols));
        let min_cols = minimum_pane_columns(usize::from(size.cols), tab.panes.len()) as i32;
        let left = i32::from(cols[boundary_index]);
        let right = i32::from(cols[boundary_index + 1]);
        let delta = i32::from(delta_cols).clamp(min_cols - left, right - min_cols);
        if delta == 0 {
            return Ok(false);
        }
        cols[boundary_index] = u16::try_from(left + delta).expect("adjusted left pane cols");
        cols[boundary_index + 1] = u16::try_from(right - delta).expect("adjusted right pane cols");
        for (pane, cols) in tab.panes.iter_mut().zip(&cols) {
            pane.width_weight = u32::from(*cols).max(DEFAULT_PANE_WEIGHT);
        }
        self.resize_active_tab(size)?;
        Ok(true)
    }

    pub fn scroll_active(&mut self, lines: i32) -> io::Result<()> {
        self.active_pane_mut().scroll_display(lines)
    }

    pub fn scroll_active_changed(&mut self, lines: i32) -> io::Result<bool> {
        self.active_pane_mut().scroll_display_changed(lines)
    }

    pub fn can_scroll_active(&self, lines: i32) -> io::Result<bool> {
        self.active_pane().can_scroll_display(lines)
    }

    pub fn available_active_scroll_lines(&self, lines: i32) -> io::Result<usize> {
        self.active_pane().available_scroll_lines(lines)
    }

    pub fn snapshot_active_renderable(&mut self) -> Option<RenderableContentOwned> {
        self.active_pane_mut().snapshot_renderable()
    }

    pub fn snapshot_active_renderable_if_dirty(&mut self) -> Option<RenderableContentOwned> {
        self.active_pane_mut().snapshot_renderable_if_dirty()
    }

    pub fn active_tab_has_dirty_renderable(&mut self) -> bool {
        let mut dirty = false;
        for pane in &mut self.active_tab_mut().panes {
            dirty = pane.backend.refresh_dirty() || dirty;
        }
        dirty
    }

    pub fn snapshot_active_tab_renderables(&mut self) -> Vec<PaneRenderable> {
        let tab = self.active_tab_mut();
        let active_pane = tab.active_pane;
        tab.panes
            .iter_mut()
            .enumerate()
            .filter_map(|(index, pane)| {
                Some(PaneRenderable {
                    id: pane.id,
                    active: pane.id == active_pane,
                    index,
                    content: pane.backend.snapshot_renderable()?,
                })
            })
            .collect()
    }

    fn active_tab(&self) -> &TerminalTab {
        let active = self.active_tab;
        self.tabs
            .iter()
            .find(|tab| tab.id == active)
            .expect("active tab must exist")
    }

    fn active_tab_mut(&mut self) -> &mut TerminalTab {
        let active = self.active_tab;
        self.tabs
            .iter_mut()
            .find(|tab| tab.id == active)
            .expect("active tab must exist")
    }

    fn active_pane(&self) -> &TerminalBackend {
        let tab = self.active_tab();
        let active_pane = tab.active_pane;
        tab.panes
            .iter()
            .find(|pane| pane.id == active_pane)
            .map(|pane| &pane.backend)
            .expect("active pane must exist")
    }

    fn active_pane_mut(&mut self) -> &mut TerminalBackend {
        let tab = self.active_tab_mut();
        let active_pane = tab.active_pane;
        tab.panes
            .iter_mut()
            .find(|pane| pane.id == active_pane)
            .map(|pane| &mut pane.backend)
            .expect("active pane must exist")
    }

    fn activate_relative(&mut self, delta: isize) {
        if self.tabs.is_empty() {
            return;
        }
        let current = self
            .tabs
            .iter()
            .position(|tab| tab.id == self.active_tab)
            .expect("active tab must exist");
        let count = self.tabs.len() as isize;
        let next = (current as isize + delta).rem_euclid(count) as usize;
        self.active_tab = self.tabs[next].id;
    }

    fn activate_relative_pane(&mut self, delta: isize) {
        let tab = self.active_tab_mut();
        if tab.panes.is_empty() {
            return;
        }
        let current = tab
            .panes
            .iter()
            .position(|pane| pane.id == tab.active_pane)
            .expect("active pane must exist");
        let count = tab.panes.len() as isize;
        let next = (current as isize + delta).rem_euclid(count) as usize;
        tab.active_pane = tab.panes[next].id;
    }
}

fn spawn_backend(command: CommandBuilder, size: Option<ScreenSize>) -> io::Result<TerminalBackend> {
    match size {
        Some(size) => TerminalBackend::spawn_with_size(command, size),
        None => TerminalBackend::spawn(command),
    }
}

fn pane_columns_for_panes(panes: &[TerminalPane], total_cols: usize) -> Vec<u16> {
    pane_columns_for_weight_values(panes.iter().map(|pane| pane.width_weight), total_cols)
}

fn pane_columns_for_weight_values<I>(weights: I, total_cols: usize) -> Vec<u16>
where
    I: Clone + ExactSizeIterator<Item = u32>,
{
    if weights.len() == 0 {
        return Vec::new();
    }
    let total_weight = weights
        .clone()
        .map(|weight| usize::try_from(weight).expect("pane weight"))
        .sum::<usize>()
        .max(1);
    let mut columns = weights
        .clone()
        .map(|weight| total_cols * usize::try_from(weight).expect("pane weight") / total_weight)
        .collect::<Vec<_>>();
    let assigned = columns.iter().sum::<usize>();
    let mut remaining = total_cols.saturating_sub(assigned);
    let mut remainders = weights
        .enumerate()
        .map(|(index, weight)| {
            (
                index,
                total_cols * usize::try_from(weight).expect("pane weight") % total_weight,
            )
        })
        .collect::<Vec<_>>();
    remainders.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    for (index, _) in remainders {
        if remaining == 0 {
            break;
        }
        columns[index] += 1;
        remaining -= 1;
    }
    columns
        .into_iter()
        .map(|cols| u16::try_from(cols.max(1)).unwrap_or(u16::MAX))
        .collect()
}

fn minimum_pane_columns(total_cols: usize, pane_count: usize) -> usize {
    if pane_count == 0 {
        return 0;
    }
    if total_cols >= pane_count * COMFORTABLE_MIN_PANE_COLS {
        COMFORTABLE_MIN_PANE_COLS
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell_text::lines_to_text;
    use std::thread::sleep;
    use std::time::{Duration, Instant};

    fn shell_command() -> CommandBuilder {
        let mut command = CommandBuilder::new("/bin/sh");
        command.arg("-i");
        command.env("PS1", "");
        command.env("ENV", "");
        command.env("BASH_ENV", "");
        command
    }

    fn wait_for_active_text(workspace: &mut TerminalWorkspace, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if let Some(snapshot) = workspace.snapshot_active_renderable() {
                let text = lines_to_text(&snapshot.lines);
                if text.contains(needle) {
                    return text;
                }
            }
            sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for active tab text {needle}");
    }

    #[test]
    fn workspace_tracks_active_tab_identity() {
        let mut workspace = TerminalWorkspace::spawn_with(shell_command()).expect("spawn tab");
        let first = workspace.active_tab_id();
        let second = workspace.add_tab_with(shell_command()).expect("spawn tab");
        assert_eq!(workspace.tab_count(), 2);
        assert_ne!(first, second);
        assert!(workspace.activate(second));
        assert_eq!(workspace.active_tab_id(), second);
        assert!(workspace.activate(first));
        assert_eq!(workspace.active_tab_id(), first);
        assert!(!workspace.activate(TabId(999)));
        let _ = workspace.write_active(b"exit\n");
    }

    #[test]
    fn workspace_cycles_active_tabs() {
        let mut workspace = TerminalWorkspace::spawn_with(shell_command()).expect("spawn tab");
        let first = workspace.active_tab_id();
        let second = workspace.add_tab_with(shell_command()).expect("spawn tab");
        let third = workspace.add_tab_with(shell_command()).expect("spawn tab");

        workspace.activate_next();
        assert_eq!(workspace.active_tab_id(), second);
        workspace.activate_next();
        assert_eq!(workspace.active_tab_id(), third);
        workspace.activate_next();
        assert_eq!(workspace.active_tab_id(), first);
        workspace.activate_previous();
        assert_eq!(workspace.active_tab_id(), third);
        assert_eq!(
            workspace.tabs(),
            vec![
                TabInfo {
                    id: first,
                    active: false,
                    index: 0,
                    pane_count: 1,
                },
                TabInfo {
                    id: second,
                    active: false,
                    index: 1,
                    pane_count: 1,
                },
                TabInfo {
                    id: third,
                    active: true,
                    index: 2,
                    pane_count: 1,
                },
            ]
        );

        let _ = workspace.write_active(b"exit\n");
    }

    #[test]
    fn workspace_keeps_each_tab_terminal_state_isolated() {
        let mut workspace = TerminalWorkspace::spawn_with(shell_command()).expect("spawn tab");
        let first = workspace.active_tab_id();
        let second = workspace.add_tab_with(shell_command()).expect("spawn tab");

        workspace
            .write_active(b"printf 'FIRST_TAB_ONLY\\n'\n")
            .expect("write first tab");
        let first_text = wait_for_active_text(&mut workspace, "FIRST_TAB_ONLY");
        assert!(!first_text.contains("SECOND_TAB_ONLY"));

        assert!(workspace.activate(second));
        workspace
            .write_active(b"printf 'SECOND_TAB_ONLY\\n'\n")
            .expect("write second tab");
        let second_text = wait_for_active_text(&mut workspace, "SECOND_TAB_ONLY");
        assert!(!second_text.contains("FIRST_TAB_ONLY"));

        assert!(workspace.activate(first));
        let first_text = wait_for_active_text(&mut workspace, "FIRST_TAB_ONLY");
        assert!(!first_text.contains("SECOND_TAB_ONLY"));

        let _ = workspace.write_active(b"exit\n");
        assert!(workspace.activate(second));
        let _ = workspace.write_active(b"exit\n");
    }

    #[test]
    fn workspace_renames_reorders_and_closes_tabs() {
        let mut workspace = TerminalWorkspace::spawn_with(shell_command()).expect("spawn tab");
        let first = workspace.active_tab_id();
        assert!(workspace.rename(first, "Host"));
        assert_eq!(workspace.tab_title(first).as_deref(), Some("Host"));

        let second = workspace
            .add_titled_tab_with("Toolbox", shell_command())
            .expect("spawn tab");
        let third = workspace
            .add_titled_tab_with("Container", shell_command())
            .expect("spawn tab");

        assert!(workspace.move_left(third));
        assert_eq!(
            workspace
                .tabs()
                .into_iter()
                .map(|tab| tab.id)
                .collect::<Vec<_>>(),
            vec![first, third, second]
        );
        assert!(workspace.reorder(second, 0));
        assert_eq!(
            workspace
                .tabs()
                .into_iter()
                .map(|tab| tab.id)
                .collect::<Vec<_>>(),
            vec![second, first, third]
        );
        assert!(workspace.close_others(third));
        assert_eq!(workspace.tab_count(), 1);
        assert_eq!(workspace.active_tab_id(), third);
        assert!(!workspace.close(third));
    }

    #[test]
    fn workspace_tracks_launch_target_for_tabs() {
        let target = LaunchTarget::Toolbox {
            name: "fedora-toolbox-latest".to_string(),
        };
        let mut workspace = TerminalWorkspace::spawn_titled_with_target(
            "fedora-toolbox-latest",
            shell_command(),
            target.clone(),
        )
        .expect("spawn tab");
        let first = workspace.active_tab_id();
        assert_eq!(
            workspace.tab_title(first).as_deref(),
            Some("fedora-toolbox-latest")
        );
        assert_eq!(workspace.tab_launch_target(first), Some(target));

        let second_target = LaunchTarget::Podman {
            name: "postgres".to_string(),
        };
        let second = workspace
            .add_titled_tab_with_target("postgres", shell_command(), second_target.clone())
            .expect("spawn second tab");
        assert_eq!(workspace.tab_title(second).as_deref(), Some("postgres"));
        assert_eq!(workspace.tab_launch_target(second), Some(second_target));

        let _ = workspace.write_active(b"exit\n");
        assert!(workspace.activate(first));
        let _ = workspace.write_active(b"exit\n");
    }

    #[test]
    fn workspace_splits_active_tab_into_isolated_panes() {
        let mut workspace = TerminalWorkspace::spawn_with(shell_command()).expect("spawn tab");
        let first_pane = workspace.active_pane_id();

        workspace
            .write_active(b"printf 'FIRST_SPLIT_ONLY\\n'\n")
            .expect("write first pane");
        let first_text = wait_for_active_text(&mut workspace, "FIRST_SPLIT_ONLY");
        assert!(!first_text.contains("SECOND_SPLIT_ONLY"));

        let second_pane = workspace
            .split_active_with(shell_command())
            .expect("spawn split pane");
        assert_ne!(first_pane, second_pane);
        assert_eq!(workspace.tab_count(), 1);
        assert_eq!(
            workspace.active_tab_panes(),
            vec![
                PaneInfo {
                    id: first_pane,
                    active: false,
                    index: 0,
                },
                PaneInfo {
                    id: second_pane,
                    active: true,
                    index: 1,
                },
            ]
        );
        assert_eq!(workspace.active_tab_pane_count(), 2);

        workspace
            .write_active(b"printf 'SECOND_SPLIT_ONLY\\n'\n")
            .expect("write second pane");
        let second_text = wait_for_active_text(&mut workspace, "SECOND_SPLIT_ONLY");
        assert!(!second_text.contains("FIRST_SPLIT_ONLY"));

        assert!(workspace.activate_pane(first_pane));
        let first_text = wait_for_active_text(&mut workspace, "FIRST_SPLIT_ONLY");
        assert!(!first_text.contains("SECOND_SPLIT_ONLY"));

        workspace.activate_next_pane();
        assert_eq!(workspace.active_pane_id(), second_pane);
        workspace.activate_previous_pane();
        assert_eq!(workspace.active_pane_id(), first_pane);
        assert!(!workspace.activate_pane(PaneId(999)));

        let _ = workspace.write_active(b"exit\n");
        assert!(workspace.activate_pane(second_pane));
        let _ = workspace.write_active(b"exit\n");
    }

    #[test]
    fn workspace_snapshots_all_panes_in_active_tab() {
        let mut workspace = TerminalWorkspace::spawn_with(shell_command()).expect("spawn tab");
        let first_pane = workspace.active_pane_id();
        workspace
            .write_active(b"printf 'FIRST_VISIBLE_SPLIT\\n'\n")
            .expect("write first pane");
        wait_for_active_text(&mut workspace, "FIRST_VISIBLE_SPLIT");

        let second_pane = workspace
            .split_active_with(shell_command())
            .expect("spawn split pane");
        workspace
            .write_active(b"printf 'SECOND_VISIBLE_SPLIT\\n'\n")
            .expect("write second pane");
        wait_for_active_text(&mut workspace, "SECOND_VISIBLE_SPLIT");

        let renderables = workspace.snapshot_active_tab_renderables();
        assert_eq!(renderables.len(), 2);
        assert_eq!(renderables[0].id, first_pane);
        assert_eq!(renderables[0].index, 0);
        assert!(!renderables[0].active);
        assert!(lines_to_text(&renderables[0].content.lines).contains("FIRST_VISIBLE_SPLIT"));
        assert!(!lines_to_text(&renderables[0].content.lines).contains("SECOND_VISIBLE_SPLIT"));
        assert_eq!(renderables[1].id, second_pane);
        assert_eq!(renderables[1].index, 1);
        assert!(renderables[1].active);
        assert!(lines_to_text(&renderables[1].content.lines).contains("SECOND_VISIBLE_SPLIT"));
        assert!(!lines_to_text(&renderables[1].content.lines).contains("FIRST_VISIBLE_SPLIT"));

        assert!(workspace.activate_pane(first_pane));
        let renderables = workspace.snapshot_active_tab_renderables();
        assert!(renderables[0].active);
        assert!(!renderables[1].active);
        assert!(
            !workspace.active_tab_has_dirty_renderable(),
            "workspace renderables should be clean after a full split snapshot"
        );
        workspace
            .resize_active_tab(ScreenSize::new(82, 12).expect("valid size"))
            .expect("resize split tab");
        assert!(
            workspace.active_tab_has_dirty_renderable(),
            "split resize should mark at least one pane renderable dirty"
        );

        let _ = workspace.write_active(b"exit\n");
        assert!(workspace.activate_pane(second_pane));
        let _ = workspace.write_active(b"exit\n");
    }

    #[test]
    fn workspace_resizes_active_tab_panes_evenly() {
        let mut workspace = TerminalWorkspace::spawn_with(shell_command()).expect("spawn tab");
        let first_pane = workspace.active_pane_id();
        let second_pane = workspace
            .split_active_with(shell_command())
            .expect("spawn split pane");

        workspace
            .resize_active_tab(ScreenSize::new(81, 12).expect("valid size"))
            .expect("resize panes");
        assert!(workspace.activate_pane(first_pane));
        let first = workspace
            .snapshot_active_renderable()
            .expect("first snapshot");
        assert_eq!(first.lines.len(), 12);
        assert_eq!(first.lines[0].len(), 41);
        assert!(workspace.activate_pane(second_pane));
        let second = workspace
            .snapshot_active_renderable()
            .expect("second snapshot");
        assert_eq!(second.lines.len(), 12);
        assert_eq!(second.lines[0].len(), 40);

        let _ = workspace.write_active(b"exit\n");
        assert!(workspace.activate_pane(first_pane));
        let _ = workspace.write_active(b"exit\n");
    }

    #[test]
    fn workspace_persists_adjusted_split_columns_across_resizes() {
        let mut workspace = TerminalWorkspace::spawn_with(shell_command()).expect("spawn tab");
        let first_pane = workspace.active_pane_id();
        let second_pane = workspace
            .split_active_with(shell_command())
            .expect("spawn split pane");
        let size = ScreenSize::new(81, 12).expect("valid size");

        assert_eq!(workspace.active_tab_pane_columns(size.cols), vec![41, 40]);
        assert!(
            workspace
                .resize_active_tab_split(0, 20, size)
                .expect("resize split")
        );
        assert_eq!(workspace.active_tab_pane_columns(size.cols), vec![61, 20]);
        workspace.resize_active_tab(size).expect("resize panes");

        assert!(workspace.activate_pane(first_pane));
        let first = workspace
            .snapshot_active_renderable()
            .expect("first snapshot");
        assert_eq!(first.lines.len(), 12);
        assert_eq!(first.lines[0].len(), 61);

        assert!(workspace.activate_pane(second_pane));
        let second = workspace
            .snapshot_active_renderable()
            .expect("second snapshot");
        assert_eq!(second.lines.len(), 12);
        assert_eq!(second.lines[0].len(), 20);

        let resized = ScreenSize::new(101, 12).expect("valid size");
        assert_eq!(
            workspace.active_tab_pane_columns(resized.cols),
            vec![76, 25]
        );

        let _ = workspace.write_active(b"exit\n");
        assert!(workspace.activate_pane(first_pane));
        let _ = workspace.write_active(b"exit\n");
    }

    #[test]
    fn workspace_split_resize_keeps_adjacent_panes_above_minimum() {
        let mut workspace = TerminalWorkspace::spawn_with(shell_command()).expect("spawn tab");
        let first_pane = workspace.active_pane_id();
        workspace
            .split_active_with(shell_command())
            .expect("spawn split pane");
        let size = ScreenSize::new(30, 8).expect("valid size");

        assert!(
            workspace
                .resize_active_tab_split(0, 100, size)
                .expect("resize split")
        );
        assert_eq!(workspace.active_tab_pane_columns(size.cols), vec![22, 8]);

        let _ = workspace.write_active(b"exit\n");
        assert!(workspace.activate_pane(first_pane));
        let _ = workspace.write_active(b"exit\n");
    }
}
