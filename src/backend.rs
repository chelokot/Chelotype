use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{RenderableContent, Term, TermMode};
use alacritty_terminal::vte::ansi::Processor;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ScreenSize {
    pub cols: u16,
    pub rows: u16,
}

impl Default for ScreenSize {
    fn default() -> Self {
        Self {
            cols: 240,
            rows: 80,
        }
    }
}

impl Dimensions for ScreenSize {
    fn total_lines(&self) -> usize {
        self.rows as usize
    }

    fn screen_lines(&self) -> usize {
        self.rows as usize
    }

    fn columns(&self) -> usize {
        self.cols as usize
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

#[derive(Clone, Default)]
pub struct NullListener;

impl EventListener for NullListener {
    fn send_event(&self, _: Event) {}
}

pub struct TerminalBackend {
    term: Arc<Mutex<Term<NullListener>>>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    _child: Box<dyn Child + Send>,
    _reader: JoinHandle<()>,
}

impl TerminalBackend {
    pub fn spawn_shell() -> std::io::Result<Self> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        Self::spawn(CommandBuilder::new(shell))
    }

    pub fn spawn(cmd: CommandBuilder) -> std::io::Result<Self> {
        let pty_system = native_pty_system();
        let size = ScreenSize::default();
        let pair = pty_system
            .openpty(size.pty_size())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

        let term = Term::new(
            alacritty_terminal::term::Config::default(),
            &size,
            NullListener,
        );
        let term = Arc::new(Mutex::new(term));
        let parser = Arc::new(Mutex::new(Processor::new()));

        let term_reader = Arc::clone(&term);
        let parser_reader: Arc<Mutex<Processor>> = Arc::clone(&parser);
        let handle = std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if let Ok(mut term_guard) = term_reader.lock() {
                            if let Ok(mut parser_guard) = parser_reader.lock() {
                                parser_guard.advance(&mut *term_guard, &buf[..n]);
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            term,
            master: pair.master,
            writer: Box::new(writer),
            _child: child,
            _reader: handle,
        })
    }

    pub fn write(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(data)?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn resize(&self, size: ScreenSize) -> std::io::Result<()> {
        self.master
            .resize(size.pty_size())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        let mut term = self.term.lock().map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::Other, "terminal lock poisoned")
        })?;
        term.resize(size);
        Ok(())
    }

    pub fn pty_size(&self) -> std::io::Result<ScreenSize> {
        let size = self
            .master
            .get_size()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        ScreenSize::new(size.cols, size.rows)
    }

    pub fn scroll_display(&self, lines: i32) -> std::io::Result<()> {
        let mut term = self.term.lock().map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::Other, "terminal lock poisoned")
        })?;
        term.scroll_display(Scroll::Delta(lines));
        Ok(())
    }

    pub fn scroll_to_bottom(&self) -> std::io::Result<()> {
        let mut term = self.term.lock().map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::Other, "terminal lock poisoned")
        })?;
        term.scroll_display(Scroll::Bottom);
        Ok(())
    }

    pub fn snapshot_renderable(&self) -> Option<RenderableContentOwned> {
        self.term
            .lock()
            .ok()
            .map(|t| RenderableContentOwned::from_renderable(t.renderable_content()))
    }

    pub fn snapshot_plain_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Ok(term) = self.term.lock() {
            let content: RenderableContent<'_> = term.renderable_content();
            for indexed in content.display_iter {
                let line_idx = indexed.point.line.0.max(0) as usize;
                if line_idx >= lines.len() {
                    lines.resize(line_idx + 1, String::new());
                }
                lines[line_idx].push(indexed.cell.c);
            }
        }
        lines
    }
}

#[derive(Clone)]
pub struct RenderableContentOwned {
    pub lines: Vec<Vec<Cell>>,
    pub cursor_line: i32,
    pub cursor_col: i32,
    pub cursor_visible: bool,
    pub display_offset: usize,
    pub colors: Colors,
    pub mouse: MouseMode,
}

impl RenderableContentOwned {
    pub fn from_renderable(content: RenderableContent<'_>) -> Self {
        let mut lines: Vec<Vec<Cell>> = Vec::new();
        for indexed in content.display_iter {
            let line_idx = indexed.point.line.0.max(0) as usize;
            if line_idx >= lines.len() {
                lines.resize_with(line_idx + 1, Vec::new);
            }
            lines[line_idx].push(indexed.cell.clone());
        }
        Self {
            lines,
            cursor_line: content.cursor.point.line.0,
            cursor_col: content.cursor.point.column.0 as i32,
            cursor_visible: !matches!(
                content.cursor.shape,
                alacritty_terminal::vte::ansi::CursorShape::Hidden
            ),
            display_offset: content.display_offset,
            colors: *content.colors,
            mouse: MouseMode::from_term_mode(content.mode),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MouseMode {
    pub click: bool,
    pub drag: bool,
    pub motion: bool,
    pub sgr: bool,
    pub utf8: bool,
}

impl MouseMode {
    fn from_term_mode(mode: TermMode) -> Self {
        Self {
            click: mode.contains(TermMode::MOUSE_REPORT_CLICK),
            drag: mode.contains(TermMode::MOUSE_DRAG),
            motion: mode.contains(TermMode::MOUSE_MOTION),
            sgr: mode.contains(TermMode::SGR_MOUSE),
            utf8: mode.contains(TermMode::UTF8_MOUSE),
        }
    }

    pub fn sends_press_release(self) -> bool {
        (self.click || self.drag || self.motion) && self.sgr
    }

    pub fn sends_drag(self) -> bool {
        (self.drag || self.motion) && self.sgr
    }
}
