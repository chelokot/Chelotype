use crate::backend::{RenderableContentOwned, ScreenSize, TerminalBackend};
use portable_pty::CommandBuilder;
use std::io;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneId(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneInfo {
    pub id: PaneId,
    pub active: bool,
    pub index: usize,
}

pub struct TerminalWorkspace {
    panes: Vec<TerminalPane>,
    active: PaneId,
    next_id: u64,
}

struct TerminalPane {
    id: PaneId,
    title: String,
    backend: TerminalBackend,
}

impl TerminalWorkspace {
    pub fn spawn_shell() -> io::Result<Self> {
        Self::spawn_with(CommandBuilder::new(
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string()),
        ))
    }

    pub fn spawn_with(command: CommandBuilder) -> io::Result<Self> {
        let first_id = PaneId(1);
        Ok(Self {
            panes: vec![TerminalPane {
                id: first_id,
                title: "Chelotype".to_string(),
                backend: TerminalBackend::spawn(command)?,
            }],
            active: first_id,
            next_id: 2,
        })
    }

    pub fn pane_count(&self) -> usize {
        self.panes.len()
    }

    pub fn active_pane_id(&self) -> PaneId {
        self.active
    }

    pub fn add_pane_with(&mut self, command: CommandBuilder) -> io::Result<PaneId> {
        self.add_titled_pane_with("Chelotype", command)
    }

    pub fn add_titled_pane_with(
        &mut self,
        title: impl Into<String>,
        command: CommandBuilder,
    ) -> io::Result<PaneId> {
        let id = PaneId(self.next_id);
        self.next_id += 1;
        self.panes.push(TerminalPane {
            id,
            title: title.into(),
            backend: TerminalBackend::spawn(command)?,
        });
        Ok(id)
    }

    pub fn add_shell_pane(&mut self) -> io::Result<PaneId> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        self.add_pane_with(CommandBuilder::new(shell))
    }

    pub fn close(&mut self, id: PaneId) -> bool {
        if self.panes.len() <= 1 {
            return false;
        }
        let Some(index) = self.panes.iter().position(|pane| pane.id == id) else {
            return false;
        };
        self.panes.remove(index);
        if self.active == id {
            let next = index.min(self.panes.len() - 1);
            self.active = self.panes[next].id;
        }
        true
    }

    pub fn close_others(&mut self, id: PaneId) -> bool {
        if !self.panes.iter().any(|pane| pane.id == id) {
            return false;
        }
        self.panes.retain(|pane| pane.id == id);
        self.active = id;
        true
    }

    pub fn rename(&mut self, id: PaneId, title: impl Into<String>) -> bool {
        let Some(pane) = self.panes.iter_mut().find(|pane| pane.id == id) else {
            return false;
        };
        let title = title.into();
        if title.trim().is_empty() {
            return false;
        }
        pane.title = title;
        true
    }

    pub fn move_left(&mut self, id: PaneId) -> bool {
        let Some(index) = self.panes.iter().position(|pane| pane.id == id) else {
            return false;
        };
        if index == 0 {
            return false;
        }
        self.panes.swap(index - 1, index);
        true
    }

    pub fn move_right(&mut self, id: PaneId) -> bool {
        let Some(index) = self.panes.iter().position(|pane| pane.id == id) else {
            return false;
        };
        if index + 1 >= self.panes.len() {
            return false;
        }
        self.panes.swap(index, index + 1);
        true
    }

    pub fn reorder(&mut self, id: PaneId, position: usize) -> bool {
        let Some(index) = self.panes.iter().position(|pane| pane.id == id) else {
            return false;
        };
        let position = position.min(self.panes.len() - 1);
        if index == position {
            return false;
        }
        let pane = self.panes.remove(index);
        self.panes.insert(position, pane);
        true
    }

    pub fn activate(&mut self, id: PaneId) -> bool {
        if self.panes.iter().any(|pane| pane.id == id) {
            self.active = id;
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

    pub fn panes(&self) -> Vec<PaneInfo> {
        self.panes
            .iter()
            .enumerate()
            .map(|(index, pane)| PaneInfo {
                id: pane.id,
                active: pane.id == self.active,
                index,
            })
            .collect()
    }

    pub fn pane_title(&self, id: PaneId) -> Option<String> {
        self.panes
            .iter()
            .find(|pane| pane.id == id)
            .map(|pane| pane.title.clone())
    }

    pub fn pane_index(&self, id: PaneId) -> Option<usize> {
        self.panes.iter().position(|pane| pane.id == id)
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

    fn active_pane_mut(&mut self) -> &mut TerminalBackend {
        let active = self.active;
        self.panes
            .iter_mut()
            .find(|pane| pane.id == active)
            .map(|pane| &mut pane.backend)
            .expect("active pane must exist")
    }

    fn activate_relative(&mut self, delta: isize) {
        if self.panes.is_empty() {
            return;
        }
        let current = self
            .panes
            .iter()
            .position(|pane| pane.id == self.active)
            .expect("active pane must exist");
        let count = self.panes.len() as isize;
        let next = (current as isize + delta).rem_euclid(count) as usize;
        self.active = self.panes[next].id;
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
        panic!("timed out waiting for active pane text {needle}");
    }

    #[test]
    fn workspace_tracks_active_pane_identity() {
        let mut workspace = TerminalWorkspace::spawn_with(shell_command()).expect("spawn pane");
        let first = workspace.active_pane_id();
        let second = workspace
            .add_pane_with(shell_command())
            .expect("spawn pane");
        assert_eq!(workspace.pane_count(), 2);
        assert_ne!(first, second);
        assert!(workspace.activate(second));
        assert_eq!(workspace.active_pane_id(), second);
        assert!(workspace.activate(first));
        assert_eq!(workspace.active_pane_id(), first);
        assert!(!workspace.activate(PaneId(999)));
        let _ = workspace.write_active(b"exit\n");
    }

    #[test]
    fn workspace_cycles_active_panes() {
        let mut workspace = TerminalWorkspace::spawn_with(shell_command()).expect("spawn pane");
        let first = workspace.active_pane_id();
        let second = workspace
            .add_pane_with(shell_command())
            .expect("spawn pane");
        let third = workspace
            .add_pane_with(shell_command())
            .expect("spawn pane");

        workspace.activate_next();
        assert_eq!(workspace.active_pane_id(), second);
        workspace.activate_next();
        assert_eq!(workspace.active_pane_id(), third);
        workspace.activate_next();
        assert_eq!(workspace.active_pane_id(), first);
        workspace.activate_previous();
        assert_eq!(workspace.active_pane_id(), third);
        assert_eq!(
            workspace.panes(),
            vec![
                PaneInfo {
                    id: first,
                    active: false,
                    index: 0,
                },
                PaneInfo {
                    id: second,
                    active: false,
                    index: 1,
                },
                PaneInfo {
                    id: third,
                    active: true,
                    index: 2,
                },
            ]
        );

        let _ = workspace.write_active(b"exit\n");
    }

    #[test]
    fn workspace_keeps_each_pane_terminal_state_isolated() {
        let mut workspace = TerminalWorkspace::spawn_with(shell_command()).expect("spawn pane");
        let first = workspace.active_pane_id();
        let second = workspace
            .add_pane_with(shell_command())
            .expect("spawn pane");

        workspace
            .write_active(b"printf 'FIRST_PANE_ONLY\\n'\n")
            .expect("write first pane");
        let first_text = wait_for_active_text(&mut workspace, "FIRST_PANE_ONLY");
        assert!(!first_text.contains("SECOND_PANE_ONLY"));

        assert!(workspace.activate(second));
        workspace
            .write_active(b"printf 'SECOND_PANE_ONLY\\n'\n")
            .expect("write second pane");
        let second_text = wait_for_active_text(&mut workspace, "SECOND_PANE_ONLY");
        assert!(!second_text.contains("FIRST_PANE_ONLY"));

        assert!(workspace.activate(first));
        let first_text = wait_for_active_text(&mut workspace, "FIRST_PANE_ONLY");
        assert!(!first_text.contains("SECOND_PANE_ONLY"));

        let _ = workspace.write_active(b"exit\n");
        assert!(workspace.activate(second));
        let _ = workspace.write_active(b"exit\n");
    }

    #[test]
    fn workspace_renames_reorders_and_closes_panes() {
        let mut workspace = TerminalWorkspace::spawn_with(shell_command()).expect("spawn pane");
        let first = workspace.active_pane_id();
        assert!(workspace.rename(first, "Host"));
        assert_eq!(workspace.pane_title(first).as_deref(), Some("Host"));

        let second = workspace
            .add_titled_pane_with("Toolbox", shell_command())
            .expect("spawn pane");
        let third = workspace
            .add_titled_pane_with("Container", shell_command())
            .expect("spawn pane");

        assert!(workspace.move_left(third));
        assert_eq!(
            workspace
                .panes()
                .into_iter()
                .map(|pane| pane.id)
                .collect::<Vec<_>>(),
            vec![first, third, second]
        );
        assert!(workspace.reorder(second, 0));
        assert_eq!(
            workspace
                .panes()
                .into_iter()
                .map(|pane| pane.id)
                .collect::<Vec<_>>(),
            vec![second, first, third]
        );
        assert!(workspace.close_others(third));
        assert_eq!(workspace.pane_count(), 1);
        assert_eq!(workspace.active_pane_id(), third);
        assert!(!workspace.close(third));
    }
}
