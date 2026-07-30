use crate::cell_text::cells_to_text;
use crate::ghostty_snapshot::GhosttySnapshotter;
pub use crate::terminal_grid::{MouseMode, TerminalContent as RenderableContentOwned};
use libghostty_vt::terminal::{Mode, ScrollViewport};
use libghostty_vt::{Terminal, TerminalOptions};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::cell::RefCell;
use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TrySendError, sync_channel};
use std::thread::JoinHandle;
#[cfg(unix)]
use std::{os::fd::AsRawFd, os::unix::net::UnixStream};

static INPUT_CURSOR_TARGET_COUNTER: AtomicU64 = AtomicU64::new(1);
const PTY_CHANNEL_CAPACITY: usize = 64;
const PTY_PROCESS_BUDGET_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScreenSize {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DisplayLimits {
    offset: usize,
    max_offset: usize,
}

impl Default for ScreenSize {
    fn default() -> Self {
        Self {
            cols: 120,
            rows: 36,
        }
    }
}

impl ScreenSize {
    pub fn new(cols: u16, rows: u16) -> std::io::Result<Self> {
        if cols == 0 || rows == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "terminal size must be non-zero",
            ));
        }
        Ok(Self { cols, rows })
    }

    fn pty_size(self) -> PtySize {
        PtySize {
            rows: self.rows,
            cols: self.cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

pub struct TerminalBackend {
    terminal: Box<Terminal<'static, 'static>>,
    snapshotter: GhosttySnapshotter,
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Box<dyn Write + Send>>,
    child: Box<dyn Child + Send>,
    #[cfg(unix)]
    process_group: Option<libc::pid_t>,
    reader: Option<JoinHandle<()>>,
    reader_shutdown_requested: Arc<AtomicBool>,
    #[cfg(unix)]
    reader_shutdown_signal: Option<UnixStream>,
    pty_rx: Receiver<Vec<u8>>,
    pty_responses: Rc<RefCell<Vec<Vec<u8>>>>,
    input_cursor_target_file: Option<PathBuf>,
    input_cursor_bridge_ready: bool,
    input_cursor_operation_counter: u64,
    input_replace_operation_counter: u64,
    terminal_palette_id: &'static str,
    dirty: bool,
}

impl TerminalBackend {
    pub fn spawn_shell() -> std::io::Result<Self> {
        Self::spawn(crate::shell::default_shell_command())
    }

    pub fn spawn_headless_shell() -> std::io::Result<Self> {
        Self::spawn(headless_shell_command())
    }

    pub fn spawn(cmd: CommandBuilder) -> std::io::Result<Self> {
        Self::spawn_with_size(cmd, ScreenSize::default())
    }

    pub fn spawn_with_size(mut cmd: CommandBuilder, size: ScreenSize) -> std::io::Result<Self> {
        let input_cursor_target_file = if cmd
            .get_env(crate::shell::INPUT_CURSOR_BRIDGE_ENV)
            .is_some_and(|value| value == OsStr::new(crate::shell::INPUT_CURSOR_BRIDGE_FISH))
        {
            Some(create_input_cursor_target_file()?)
        } else {
            None
        };
        if let Some(path) = &input_cursor_target_file {
            cmd.env(crate::shell::INPUT_CURSOR_TARGET_FILE_ENV, path.as_os_str());
            inject_input_cursor_target_file(&mut cmd, path);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("COLUMNS", size.cols.to_string());
        cmd.env("LINES", size.rows.to_string());
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(size.pty_size())
            .map_err(|error| std::io::Error::other(error.to_string()))?;

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        #[cfg(unix)]
        let process_group = child
            .process_id()
            .map(|process_id| libc::pid_t::try_from(process_id).expect("child process id"));
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| std::io::Error::other(error.to_string()))?;

        let (pty_tx, pty_rx) = sync_channel(PTY_CHANNEL_CAPACITY);
        let reader_shutdown_requested = Arc::new(AtomicBool::new(false));
        let thread_shutdown_requested = reader_shutdown_requested.clone();
        #[cfg(unix)]
        let (reader_shutdown_signal, reader_shutdown_receiver) = UnixStream::pair()?;
        #[cfg(unix)]
        let reader_pty_fd = pair.master.as_raw_fd().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "PTY master does not expose a file descriptor",
            )
        })?;
        let handle = std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                #[cfg(unix)]
                {
                    let mut poll_fds = [
                        libc::pollfd {
                            fd: reader_pty_fd,
                            events: libc::POLLIN,
                            revents: 0,
                        },
                        libc::pollfd {
                            fd: reader_shutdown_receiver.as_raw_fd(),
                            events: libc::POLLIN,
                            revents: 0,
                        },
                    ];
                    let poll_result = unsafe {
                        libc::poll(poll_fds.as_mut_ptr(), poll_fds.len() as libc::nfds_t, -1)
                    };
                    if poll_result < 0 {
                        if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
                        {
                            continue;
                        }
                        break;
                    }
                    if poll_fds[1].revents != 0 {
                        break;
                    }
                    if poll_fds[0].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) == 0 {
                        continue;
                    }
                }
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(len) => {
                        let mut chunk = buf[..len].to_vec();
                        loop {
                            if thread_shutdown_requested.load(Ordering::Acquire) {
                                return;
                            }
                            match pty_tx.try_send(chunk) {
                                Ok(()) => break,
                                Err(TrySendError::Disconnected(_)) => return,
                                Err(TrySendError::Full(returned)) => {
                                    chunk = returned;
                                    std::thread::sleep(std::time::Duration::from_millis(1));
                                }
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        let pty_responses = Rc::new(RefCell::new(Vec::<Vec<u8>>::new()));
        let mut terminal = Box::new(
            Terminal::new(TerminalOptions {
                cols: size.cols,
                rows: size.rows,
                max_scrollback: 10000,
            })
            .map_err(|error| std::io::Error::other(error.to_string()))?,
        );
        terminal
            .on_pty_write({
                let pty_responses = pty_responses.clone();
                move |_terminal, data| {
                    pty_responses.borrow_mut().push(data.to_vec());
                }
            })
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let terminal_palette = crate::terminal_palette::default_terminal_palette();
        crate::ghostty_snapshot::apply_terminal_palette(&mut terminal, terminal_palette)
            .map_err(|error| std::io::Error::other(error.to_string()))?;

        Ok(Self {
            terminal,
            snapshotter: GhosttySnapshotter::new()
                .map_err(|error| std::io::Error::other(error.to_string()))?,
            master: Some(pair.master),
            writer: Some(Box::new(writer)),
            child,
            #[cfg(unix)]
            process_group,
            reader: Some(handle),
            reader_shutdown_requested,
            #[cfg(unix)]
            reader_shutdown_signal: Some(reader_shutdown_signal),
            pty_rx,
            pty_responses,
            input_cursor_target_file,
            input_cursor_bridge_ready: false,
            input_cursor_operation_counter: 0,
            input_replace_operation_counter: 0,
            terminal_palette_id: terminal_palette.id,
            dirty: true,
        })
    }

    pub fn write(&mut self, data: &[u8]) -> std::io::Result<()> {
        let writer = self.writer.as_mut().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "terminal is shut down")
        })?;
        writer.write_all(data)?;
        writer.flush()?;
        Ok(())
    }

    pub fn shutdown(&mut self) {
        let mut child_running = !matches!(self.child.try_wait(), Ok(Some(_)));
        if child_running {
            #[cfg(unix)]
            {
                let foreground_group = self
                    .master
                    .as_ref()
                    .and_then(|master| master.process_group_leader());
                for process_group in [foreground_group, self.process_group]
                    .into_iter()
                    .flatten()
                    .collect::<std::collections::BTreeSet<_>>()
                {
                    unsafe {
                        libc::kill(-process_group, libc::SIGHUP);
                    }
                }
                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(100);
                while child_running && std::time::Instant::now() < deadline {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    child_running = !matches!(self.child.try_wait(), Ok(Some(_)));
                }
            }
            if child_running {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
        self.reader_shutdown_requested
            .store(true, Ordering::Release);
        #[cfg(unix)]
        if let Some(mut signal) = self.reader_shutdown_signal.take() {
            let _ = signal.write_all(&[1]);
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        self.writer.take();
        self.master.take();
        if let Some(path) = &self.input_cursor_target_file {
            let _ = std::fs::remove_file(path);
            let _ = std::fs::remove_file(input_bridge_sidecar_path(path, ".cursor"));
            if let (Some(directory), Some(file_name)) = (path.parent(), path.file_name())
                && let Ok(entries) = std::fs::read_dir(directory)
            {
                let file_name = file_name.to_string_lossy();
                let operation_prefixes = [
                    format!("{file_name}.cursor-"),
                    format!("{file_name}.replace-"),
                ];
                for entry in entries.flatten() {
                    let entry_name = entry.file_name();
                    let entry_name = entry_name.to_string_lossy();
                    if operation_prefixes
                        .iter()
                        .any(|prefix| entry_name.starts_with(prefix))
                    {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
        }
    }

    pub fn write_input_cursor_target(&mut self, offset: usize) -> std::io::Result<bool> {
        if !self.input_cursor_bridge_ready()? {
            return Ok(false);
        }
        let Some(path) = &self.input_cursor_target_file else {
            return Ok(false);
        };
        write_input_bridge_operation(
            path,
            "cursor",
            self.input_cursor_operation_counter,
            &offset.to_string(),
        )?;
        self.input_cursor_operation_counter += 1;
        self.write(crate::shell::INPUT_CURSOR_TARGET_SEQUENCE)?;
        self.snapshotter.invalidate();
        self.dirty = true;
        Ok(true)
    }

    pub fn write_input_replace_range(
        &mut self,
        range: std::ops::Range<usize>,
        replacement: &str,
    ) -> std::io::Result<bool> {
        if !self.input_cursor_bridge_ready()? {
            return Ok(false);
        }
        let Some(path) = &self.input_cursor_target_file else {
            return Ok(false);
        };
        let payload = format!(
            "{}\t{}\t{}",
            range.start,
            range.end,
            fish_var_encode(replacement)
        );
        write_input_bridge_operation(
            path,
            "replace",
            self.input_replace_operation_counter,
            &payload,
        )?;
        self.input_replace_operation_counter += 1;
        self.write(crate::shell::INPUT_REPLACE_RANGE_SEQUENCE)?;
        self.snapshotter.invalidate();
        self.dirty = true;
        Ok(true)
    }

    pub fn input_cursor_bridge_ready(&mut self) -> std::io::Result<bool> {
        if self.input_cursor_bridge_ready {
            return Ok(true);
        }
        let Some(path) = &self.input_cursor_target_file else {
            return Ok(false);
        };
        self.input_cursor_bridge_ready = std::fs::read_to_string(path)? == "ready";
        Ok(self.input_cursor_bridge_ready)
    }

    pub fn shell_input_ready(&mut self) -> std::io::Result<bool> {
        if self.input_cursor_target_file.is_none() {
            return Ok(true);
        }
        self.input_cursor_bridge_ready()
    }

    pub fn bracketed_paste_mode(&mut self) -> bool {
        let _ = self.process_pending();
        self.terminal.mode(Mode::BRACKETED_PASTE).unwrap_or(false)
    }

    pub fn resize(&mut self, size: ScreenSize) -> std::io::Result<()> {
        self.master
            .as_ref()
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "terminal is shut down")
            })?
            .resize(size.pty_size())
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        self.terminal
            .resize(size.cols, size.rows, 8, 18)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        self.snapshotter.invalidate();
        self.dirty = true;
        Ok(())
    }

    pub fn pty_size(&self) -> std::io::Result<ScreenSize> {
        let size = self
            .master
            .as_ref()
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "terminal is shut down")
            })?
            .get_size()
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        ScreenSize::new(size.cols, size.rows)
    }

    pub fn scroll_display(&mut self, lines: i32) -> std::io::Result<()> {
        self.scroll_display_changed(lines).map(|_| ())
    }

    pub fn scroll_display_changed(&mut self, lines: i32) -> std::io::Result<bool> {
        let before = self.display_offset()?;
        self.terminal
            .scroll_viewport(ScrollViewport::Delta(-(lines as isize)));
        let after = self.display_offset()?;
        let changed = before != after;
        self.snapshotter.invalidate();
        self.dirty = changed || self.dirty;
        Ok(changed)
    }

    pub fn can_scroll_display(&self, lines: i32) -> std::io::Result<bool> {
        Ok(self.available_scroll_lines(lines)? > 0)
    }

    pub fn available_scroll_lines(&self, lines: i32) -> std::io::Result<usize> {
        let limits = self.display_limits()?;
        let available = if lines > 0 {
            limits.max_offset.saturating_sub(limits.offset)
        } else if lines < 0 {
            limits.offset
        } else {
            0
        };
        Ok(available)
    }

    pub fn scroll_to_bottom(&mut self) -> std::io::Result<()> {
        let before = self.display_offset()?;
        self.terminal.scroll_viewport(ScrollViewport::Bottom);
        if self.display_offset()? != before {
            self.snapshotter.invalidate();
            self.dirty = true;
        }
        Ok(())
    }

    fn display_offset(&self) -> std::io::Result<usize> {
        Ok(self.display_limits()?.offset)
    }

    fn display_limits(&self) -> std::io::Result<DisplayLimits> {
        let scrollbar = self
            .terminal
            .scrollbar()
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let max_offset = scrollbar.total.saturating_sub(scrollbar.len) as usize;
        let offset = scrollbar
            .total
            .saturating_sub(scrollbar.len)
            .saturating_sub(scrollbar.offset) as usize;
        Ok(DisplayLimits { offset, max_offset })
    }

    pub fn snapshot_renderable(&mut self) -> Option<RenderableContentOwned> {
        self.sync_terminal_palette().ok()?;
        if self.process_pending().ok()? {
            self.dirty = true;
        }
        let snapshot = self.snapshotter.snapshot(&self.terminal).ok()?;
        crate::perf_trace::record_counter(
            "ghostty_snapshot_converted_rows",
            self.snapshotter.last_converted_rows() as u64,
        );
        self.dirty = false;
        Some(snapshot)
    }

    pub fn refresh_dirty(&mut self) -> bool {
        if self.sync_terminal_palette().is_err() {
            return self.dirty;
        }
        if self.process_pending().unwrap_or(false) {
            self.dirty = true;
        }
        self.dirty
    }

    pub fn snapshot_renderable_if_dirty(&mut self) -> Option<RenderableContentOwned> {
        self.sync_terminal_palette().ok()?;
        if self.process_pending().ok()? {
            self.dirty = true;
        }
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        let snapshot = self.snapshotter.snapshot(&self.terminal).ok()?;
        crate::perf_trace::record_counter(
            "ghostty_snapshot_converted_rows",
            self.snapshotter.last_converted_rows() as u64,
        );
        Some(snapshot)
    }

    pub fn snapshot_plain_lines(&mut self) -> Vec<String> {
        self.snapshot_renderable()
            .map(|content| {
                content
                    .lines
                    .iter()
                    .map(|line| cells_to_text(line))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn process_pending(&mut self) -> std::io::Result<bool> {
        let mut processed = false;
        let mut processed_bytes = 0;
        while processed_bytes < PTY_PROCESS_BUDGET_BYTES
            && let Ok(data) = self.pty_rx.try_recv()
        {
            processed = true;
            processed_bytes += data.len();
            self.terminal.vt_write(&data);
            let responses = std::mem::take(&mut *self.pty_responses.borrow_mut());
            for response in responses {
                self.write(&response)?;
            }
        }
        crate::perf_trace::record_counter("pty_processed_bytes", processed_bytes as u64);
        Ok(processed)
    }

    fn sync_terminal_palette(&mut self) -> std::io::Result<()> {
        let palette = crate::terminal_palette::default_terminal_palette();
        if self.terminal_palette_id == palette.id {
            return Ok(());
        }
        crate::ghostty_snapshot::apply_terminal_palette(&mut self.terminal, palette)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        self.terminal_palette_id = palette.id;
        self.snapshotter.invalidate();
        self.dirty = true;
        Ok(())
    }
}

fn fish_var_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len() + 1);
    encoded.push('x');
    let mut escaping = false;
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() {
            if escaping {
                escaping = false;
            }
            encoded.push(char::from(byte));
        } else if byte == b'_' {
            if escaping {
                escaping = false;
            }
            encoded.push_str("__");
        } else {
            use std::fmt::Write;
            if !escaping {
                encoded.push('_');
                escaping = true;
            }
            write!(encoded, "{byte:02X}_").expect("write to string");
        }
    }
    encoded
}

fn input_bridge_sidecar_path(base: &std::path::Path, suffix: &str) -> PathBuf {
    let mut path = base.as_os_str().to_os_string();
    path.push(suffix);
    PathBuf::from(path)
}

fn write_input_bridge_operation(
    base: &std::path::Path,
    kind: &str,
    counter: u64,
    payload: &str,
) -> std::io::Result<()> {
    let operation_path = input_bridge_sidecar_path(base, &format!(".{kind}-{counter:020}"));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(operation_path)?;
    file.write_all(payload.as_bytes())?;
    file.flush()
}

pub fn headless_shell_command() -> CommandBuilder {
    let mut command = CommandBuilder::new("/usr/bin/zsh");
    command.arg("-f");
    command.arg("-i");
    command.env("TERM", "xterm-256color");
    command.env("PS1", "");
    command.env("PROMPT_COMMAND", "");
    command.env("HISTFILE", "/dev/null");
    command.env("ENV", "");
    command.env("BASH_ENV", "");
    command
}

fn create_input_cursor_target_file() -> std::io::Result<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    let directory = base.join("chelotype");
    std::fs::create_dir_all(&directory)?;
    for _ in 0..16 {
        let counter = INPUT_CURSOR_TARGET_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!(
            "input-cursor-target-{}-{counter}",
            std::process::id()
        ));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(_) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "cursor target file name collision",
    ))
}

fn inject_input_cursor_target_file(command: &mut CommandBuilder, path: &std::path::Path) {
    let placeholder = crate::shell::INPUT_CURSOR_TARGET_FILE_PLACEHOLDER;
    let path = path.to_string_lossy();
    for argument in command.get_argv_mut() {
        let value = argument.to_string_lossy();
        if value.contains(placeholder) {
            *argument = value.replace(placeholder, &path).into();
        }
    }
}

impl Drop for TerminalBackend {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::fish_var_encode;

    #[test]
    fn fish_var_encoding_preserves_empty_unicode_and_control_bytes() {
        assert_eq!(fish_var_encode(""), "x");
        assert_eq!(fish_var_encode("_"), "x__");
        assert_eq!(fish_var_encode("\n\n"), "x_0A_0A_");
        assert_eq!(fish_var_encode("好😀"), "x_E5_A5_BD_F0_9F_98_80_");
    }
}
