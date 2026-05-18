use crate::cell_text::cells_to_text;
use crate::ghostty_snapshot::GhosttySnapshotter;
pub use crate::terminal_grid::{MouseMode, TerminalContent as RenderableContentOwned};
use libghostty_vt::terminal::ScrollViewport;
use libghostty_vt::{Terminal, TerminalOptions};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::cell::RefCell;
use std::io::{Read, Write};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, channel};
use std::thread::JoinHandle;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScreenSize {
    pub cols: u16,
    pub rows: u16,
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
    terminal: Terminal<'static, 'static>,
    snapshotter: GhosttySnapshotter,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    _child: Box<dyn Child + Send>,
    _reader: JoinHandle<()>,
    pty_rx: Receiver<Vec<u8>>,
    pty_responses: Rc<RefCell<Vec<Vec<u8>>>>,
    dirty: bool,
}

impl TerminalBackend {
    pub fn spawn_shell() -> std::io::Result<Self> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        Self::spawn(CommandBuilder::new(shell))
    }

    pub fn spawn_headless_shell() -> std::io::Result<Self> {
        let mut command = CommandBuilder::new("/usr/bin/zsh");
        command.arg("-f");
        command.arg("-i");
        command.env("TERM", "xterm-256color");
        command.env("PS1", "");
        command.env("PROMPT_COMMAND", "");
        command.env("HISTFILE", "/dev/null");
        command.env("ENV", "");
        command.env("BASH_ENV", "");
        Self::spawn(command)
    }

    pub fn spawn(cmd: CommandBuilder) -> std::io::Result<Self> {
        let pty_system = native_pty_system();
        let size = ScreenSize::default();
        let pair = pty_system
            .openpty(size.pty_size())
            .map_err(|error| std::io::Error::other(error.to_string()))?;

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| std::io::Error::other(error.to_string()))?;

        let (pty_tx, pty_rx) = channel();
        let handle = std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(len) => {
                        if pty_tx.send(buf[..len].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        let pty_responses = Rc::new(RefCell::new(Vec::<Vec<u8>>::new()));
        let mut terminal = Terminal::new(TerminalOptions {
            cols: size.cols,
            rows: size.rows,
            max_scrollback: 10000,
        })
        .map_err(|error| std::io::Error::other(error.to_string()))?;
        terminal
            .on_pty_write({
                let pty_responses = pty_responses.clone();
                move |_terminal, data| {
                    pty_responses.borrow_mut().push(data.to_vec());
                }
            })
            .map_err(|error| std::io::Error::other(error.to_string()))?;

        Ok(Self {
            terminal,
            snapshotter: GhosttySnapshotter::new()
                .map_err(|error| std::io::Error::other(error.to_string()))?,
            master: pair.master,
            writer: Box::new(writer),
            _child: child,
            _reader: handle,
            pty_rx,
            pty_responses,
            dirty: true,
        })
    }

    pub fn write(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(data)?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn resize(&mut self, size: ScreenSize) -> std::io::Result<()> {
        self.master
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
            .get_size()
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        ScreenSize::new(size.cols, size.rows)
    }

    pub fn scroll_display(&mut self, lines: i32) -> std::io::Result<()> {
        self.terminal
            .scroll_viewport(ScrollViewport::Delta(-(lines as isize)));
        self.snapshotter.invalidate();
        self.dirty = true;
        Ok(())
    }

    pub fn scroll_to_bottom(&mut self) -> std::io::Result<()> {
        self.terminal.scroll_viewport(ScrollViewport::Bottom);
        self.snapshotter.invalidate();
        self.dirty = true;
        Ok(())
    }

    pub fn snapshot_renderable(&mut self) -> Option<RenderableContentOwned> {
        if self.process_pending().ok()? {
            self.dirty = true;
        }
        let snapshot = self.snapshotter.snapshot(&self.terminal).ok()?;
        self.dirty = false;
        Some(snapshot)
    }

    pub fn snapshot_renderable_if_dirty(&mut self) -> Option<RenderableContentOwned> {
        if self.process_pending().ok()? {
            self.dirty = true;
        }
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        self.snapshotter.snapshot(&self.terminal).ok()
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
        while let Ok(data) = self.pty_rx.try_recv() {
            processed = true;
            self.terminal.vt_write(&data);
            self.snapshotter.invalidate();
            let responses = std::mem::take(&mut *self.pty_responses.borrow_mut());
            for response in responses {
                self.write(&response)?;
            }
        }
        Ok(processed)
    }
}
