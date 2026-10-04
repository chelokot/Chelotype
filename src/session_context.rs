use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{collections::HashSet, collections::VecDeque};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SessionContext {
    Normal,
    Root,
    Ssh,
    RootSsh,
    #[default]
    Unknown,
}

impl SessionContext {
    pub const fn from_process_flags(root: bool, ssh: bool) -> Self {
        match (root, ssh) {
            (false, false) => Self::Normal,
            (true, false) => Self::Root,
            (false, true) => Self::Ssh,
            (true, true) => Self::RootSsh,
        }
    }

    pub const fn is_known(self) -> bool {
        !matches!(self, Self::Unknown)
    }

    pub const fn is_root(self) -> bool {
        matches!(self, Self::Root | Self::RootSsh)
    }

    pub const fn is_ssh(self) -> bool {
        matches!(self, Self::Ssh | Self::RootSsh)
    }

    pub const fn header_css_class(self) -> &'static str {
        match self {
            Self::Normal => "session-context-normal",
            Self::Root => "session-context-root",
            Self::Ssh => "session-context-ssh",
            Self::RootSsh => "session-context-root-ssh",
            Self::Unknown => "session-context-unknown",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessSnapshot {
    pub pid: i32,
    pub parent_pid: i32,
    pub process_group_id: i32,
    pub session_id: i32,
    pub effective_uid: u32,
    pub executable: Option<PathBuf>,
}

impl ProcessSnapshot {
    pub fn new(
        pid: i32,
        parent_pid: i32,
        process_group_id: i32,
        session_id: i32,
        effective_uid: u32,
        executable: impl Into<PathBuf>,
    ) -> Self {
        Self {
            pid,
            parent_pid,
            process_group_id,
            session_id,
            effective_uid,
            executable: Some(executable.into()),
        }
    }

    fn is_ssh(&self) -> bool {
        self.executable
            .as_deref()
            .and_then(Path::file_name)
            .is_some_and(|name| name == "ssh")
    }
}

pub struct ProcessContextCache {
    foreground_process_group: i32,
    context: SessionContext,
    last_full_scan: Instant,
}

const PROCESS_TREE_SCAN_INTERVAL: Duration = Duration::from_millis(500);
const PROCESS_TREE_LIMIT: usize = 256;

pub fn classify_process_group(
    foreground_process_group: i32,
    processes: &[ProcessSnapshot],
) -> SessionContext {
    let foreground = processes
        .iter()
        .filter(|process| process.process_group_id == foreground_process_group)
        .collect::<Vec<_>>();
    if foreground.is_empty() {
        return SessionContext::Unknown;
    }

    let session_id = foreground[0].session_id;
    let mut root = false;
    let mut ssh = false;
    for process in foreground {
        if process.session_id != session_id {
            continue;
        }
        let mut current_pid = process.pid;
        let mut visited = Vec::new();
        while let Some(current) = processes
            .iter()
            .find(|candidate| candidate.pid == current_pid)
        {
            if current.session_id != session_id || visited.contains(&current.pid) {
                break;
            }
            visited.push(current.pid);
            root |= current.effective_uid == 0;
            ssh |= current.is_ssh();
            if current.parent_pid <= 1 || current.parent_pid == current.pid {
                break;
            }
            current_pid = current.parent_pid;
        }
    }
    SessionContext::from_process_flags(root, ssh)
}

#[cfg(unix)]
pub fn detect_from_pty(fd: std::os::fd::RawFd) -> SessionContext {
    let mut cache = None;
    detect_from_pty_cached(fd, &mut cache)
}

#[cfg(unix)]
pub fn detect_from_pty_cached(
    fd: std::os::fd::RawFd,
    cache: &mut Option<ProcessContextCache>,
) -> SessionContext {
    if fd < 0 {
        return SessionContext::Unknown;
    }
    let foreground_process_group = unsafe { libc::tcgetpgrp(fd) };
    if foreground_process_group <= 0 {
        return SessionContext::Unknown;
    }

    if let Some(process) = read_process_snapshot(foreground_process_group)
        && process.process_group_id == foreground_process_group
    {
        let mut processes = Vec::new();
        let mut current = process;
        loop {
            let parent_pid = current.parent_pid;
            processes.push(current);
            if parent_pid <= 1 {
                break;
            }
            let Some(parent) = read_process_snapshot(parent_pid) else {
                break;
            };
            if parent.session_id != processes[0].session_id {
                break;
            }
            if processes.iter().any(|process| process.pid == parent.pid) {
                break;
            }
            current = parent;
        }
        let fast_context = classify_process_group(foreground_process_group, &processes);
        if fast_context.is_root() || fast_context.is_ssh() {
            return fast_context;
        }
        if let Some(cached) = cache.as_ref().filter(|cached| {
            cached.foreground_process_group == foreground_process_group
                && cached.last_full_scan.elapsed() < PROCESS_TREE_SCAN_INTERVAL
        }) {
            return cached.context;
        }
    }

    let Some(process) = read_process_snapshot(foreground_process_group) else {
        return SessionContext::Unknown;
    };
    if process.process_group_id != foreground_process_group {
        return SessionContext::Unknown;
    }
    let Some(processes) = read_session_process_tree(&process) else {
        return SessionContext::Unknown;
    };
    let context = classify_process_group(foreground_process_group, &processes);
    *cache = Some(ProcessContextCache {
        foreground_process_group,
        context,
        last_full_scan: Instant::now(),
    });
    context
}

#[cfg(not(unix))]
pub fn detect_from_pty(_fd: i32) -> SessionContext {
    SessionContext::Unknown
}

#[cfg(unix)]
fn read_process_snapshot(pid: i32) -> Option<ProcessSnapshot> {
    if pid <= 0 {
        return None;
    }
    let process_path = Path::new("/proc").join(pid.to_string());
    let stat = std::fs::read_to_string(process_path.join("stat")).ok()?;
    let closing_parenthesis = stat.rfind(')')?;
    let mut fields = stat[closing_parenthesis + 1..].split_whitespace();
    fields.next()?;
    let parent_pid = fields.next()?.parse().ok()?;
    let process_group_id = fields.next()?.parse().ok()?;
    let session_id = fields.next()?.parse().ok()?;
    let status = std::fs::read_to_string(process_path.join("status")).ok()?;
    let effective_uid = status.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        (fields.next() == Some("Uid:")).then(|| fields.nth(1)?.parse().ok())?
    })?;
    let executable = std::fs::read_link(process_path.join("exe"))
        .ok()
        .or_else(|| {
            std::fs::read_to_string(process_path.join("comm"))
                .ok()
                .map(|name| PathBuf::from(name.trim()))
        });
    Some(ProcessSnapshot {
        pid,
        parent_pid,
        process_group_id,
        session_id,
        effective_uid,
        executable,
    })
}

#[cfg(unix)]
fn read_session_process_tree(foreground: &ProcessSnapshot) -> Option<Vec<ProcessSnapshot>> {
    let session_leader = read_process_snapshot(foreground.session_id)?;
    let mut snapshots = vec![session_leader.clone()];
    let mut queue = VecDeque::from([session_leader.pid]);
    let mut visited = HashSet::from([session_leader.pid]);
    while let Some(pid) = queue.pop_front() {
        if visited.len() >= PROCESS_TREE_LIMIT {
            break;
        }
        for child_pid in read_process_children(pid)? {
            if !visited.insert(child_pid) {
                continue;
            }
            let Some(child) = read_process_snapshot(child_pid) else {
                continue;
            };
            if child.session_id != foreground.session_id {
                continue;
            }
            queue.push_back(child_pid);
            snapshots.push(child);
            if visited.len() >= PROCESS_TREE_LIMIT {
                break;
            }
        }
    }
    snapshots.push(foreground.clone());
    for ancestor in read_process_ancestry(foreground.clone()) {
        if !snapshots.iter().any(|process| process.pid == ancestor.pid) {
            snapshots.push(ancestor);
        }
    }
    Some(snapshots)
}

#[cfg(unix)]
fn read_process_ancestry(process: ProcessSnapshot) -> Vec<ProcessSnapshot> {
    let session_id = process.session_id;
    let mut ancestors = Vec::new();
    let mut current = process;
    loop {
        let parent_pid = current.parent_pid;
        if parent_pid <= 1 {
            break;
        }
        let Some(parent) = read_process_snapshot(parent_pid) else {
            break;
        };
        if parent.session_id != session_id {
            break;
        }
        if ancestors
            .iter()
            .any(|ancestor: &ProcessSnapshot| ancestor.pid == parent.pid)
        {
            break;
        }
        current = parent.clone();
        ancestors.push(parent);
    }
    ancestors
}

#[cfg(unix)]
fn read_process_children(pid: i32) -> Option<Vec<i32>> {
    let children = std::fs::read_to_string(
        Path::new("/proc")
            .join(pid.to_string())
            .join("task")
            .join(pid.to_string())
            .join("children"),
    )
    .ok()?;
    Some(
        children
            .split_whitespace()
            .filter_map(|child| child.parse().ok())
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::{ProcessSnapshot, SessionContext, classify_process_group};

    #[test]
    fn classifies_root_and_ssh_from_foreground_ancestry() {
        let processes = vec![
            ProcessSnapshot::new(10, 1, 10, 10, 1000, "/usr/bin/bash"),
            ProcessSnapshot::new(20, 10, 20, 10, 1000, "/usr/bin/ssh"),
            ProcessSnapshot::new(30, 20, 20, 10, 0, "/usr/bin/bash"),
        ];
        assert_eq!(
            classify_process_group(20, &processes),
            SessionContext::RootSsh
        );
    }

    #[test]
    fn ignores_similar_executable_names_and_other_sessions() {
        let processes = vec![
            ProcessSnapshot::new(10, 1, 10, 10, 1000, "/usr/bin/bash"),
            ProcessSnapshot::new(20, 10, 20, 10, 1000, "/tmp/ssh-wrapper"),
            ProcessSnapshot::new(30, 1, 30, 30, 0, "/usr/bin/ssh"),
        ];
        assert_eq!(
            classify_process_group(20, &processes),
            SessionContext::Normal
        );
    }
}
