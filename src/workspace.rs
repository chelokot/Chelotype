use crate::backend::{RenderableContentOwned, ScreenSize, TerminalBackend};
use portable_pty::CommandBuilder;
use std::io;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TabId(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneId(u64);

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

pub struct TerminalWorkspace {
    tabs: Vec<TerminalTab>,
    active_tab: TabId,
    next_tab_id: u64,
    next_pane_id: u64,
}

struct TerminalTab {
    id: TabId,
    title: String,
    panes: Vec<TerminalPane>,
    active_pane: PaneId,
}

struct TerminalPane {
    id: PaneId,
    backend: TerminalBackend,
}

impl TerminalWorkspace {
    pub fn spawn_shell() -> io::Result<Self> {
        Self::spawn_with(CommandBuilder::new(
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string()),
        ))
    }

    pub fn spawn_with(command: CommandBuilder) -> io::Result<Self> {
        let first_tab_id = TabId(1);
        let first_pane_id = PaneId(1);
        Ok(Self {
            tabs: vec![TerminalTab {
                id: first_tab_id,
                title: "Chelotype".to_string(),
                panes: vec![TerminalPane {
                    id: first_pane_id,
                    backend: TerminalBackend::spawn(command)?,
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
        let id = TabId(self.next_tab_id);
        self.next_tab_id += 1;
        let pane_id = PaneId(self.next_pane_id);
        self.next_pane_id += 1;
        self.tabs.push(TerminalTab {
            id,
            title: title.into(),
            panes: vec![TerminalPane {
                id: pane_id,
                backend: TerminalBackend::spawn(command)?,
            }],
            active_pane: pane_id,
        });
        Ok(id)
    }

    pub fn split_active_with(&mut self, command: CommandBuilder) -> io::Result<PaneId> {
        let id = PaneId(self.next_pane_id);
        self.next_pane_id += 1;
        let tab = self.active_tab_mut();
        tab.panes.push(TerminalPane {
            id,
            backend: TerminalBackend::spawn(command)?,
        });
        tab.active_pane = id;
        Ok(id)
    }

    pub fn add_shell_tab(&mut self) -> io::Result<TabId> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        self.add_tab_with(CommandBuilder::new(shell))
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

    pub fn tab_title(&self, id: TabId) -> Option<String> {
        self.tabs
            .iter()
            .find(|tab| tab.id == id)
            .map(|tab| tab.title.clone())
    }

    pub fn tab_index(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    pub fn write_active(&mut self, data: &[u8]) -> io::Result<()> {
        self.active_pane_mut().write(data)
    }

    pub fn resize_active(&mut self, size: ScreenSize) -> io::Result<()> {
        self.active_pane_mut().resize(size)
    }

    pub fn scroll_active(&mut self, lines: i32) -> io::Result<()> {
        self.active_pane_mut().scroll_display(lines)
    }

    pub fn snapshot_active_renderable(&mut self) -> Option<RenderableContentOwned> {
        self.active_pane_mut().snapshot_renderable()
    }

    pub fn snapshot_active_renderable_if_dirty(&mut self) -> Option<RenderableContentOwned> {
        self.active_pane_mut().snapshot_renderable_if_dirty()
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
}
