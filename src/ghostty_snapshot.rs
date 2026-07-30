use crate::terminal_grid::{
    MouseMode, TerminalCell, TerminalColors, TerminalContent, TerminalLine, TerminalLineMetadata,
    TerminalSemanticPrompt,
};
use crate::terminal_palette::{TerminalPalette, default_terminal_palette};
use libghostty_vt::render::{CellIterator, Dirty, RowIterator};
use libghostty_vt::screen::{CellWide, RowSemanticPrompt};
use libghostty_vt::style::{RgbColor, Underline};
use libghostty_vt::terminal::Mode;
use libghostty_vt::{RenderState, Terminal};
use std::borrow::Cow;

pub struct GhosttySnapshotter {
    render_state: RenderState<'static>,
    row_iter: RowIterator<'static>,
    cell_iter: CellIterator<'static>,
    lines: Vec<TerminalLine>,
    line_metadata: Vec<TerminalLineMetadata>,
    converted_rows: usize,
    palette_id: Option<&'static str>,
}

impl GhosttySnapshotter {
    pub fn new() -> libghostty_vt::error::Result<Self> {
        Ok(Self {
            render_state: RenderState::new()?,
            row_iter: RowIterator::new()?,
            cell_iter: CellIterator::new()?,
            lines: Vec::new(),
            line_metadata: Vec::new(),
            converted_rows: 0,
            palette_id: None,
        })
    }

    pub fn invalidate(&mut self) {
        self.lines.clear();
        self.line_metadata.clear();
    }

    pub fn snapshot(
        &mut self,
        terminal: &Terminal<'static, 'static>,
    ) -> libghostty_vt::error::Result<TerminalContent> {
        let scrollbar = terminal.scrollbar()?;
        let snapshot = self.render_state.update(terminal)?;
        let colors = snapshot.colors()?;
        let cursor = snapshot.cursor_viewport()?;
        let cursor_visible = snapshot.cursor_visible()? && cursor.is_some();
        let cursor_line = cursor.map(|cursor| i32::from(cursor.y)).unwrap_or(-1);
        let cursor_col = cursor.map(|cursor| i32::from(cursor.x)).unwrap_or(-1);
        let rows = usize::from(snapshot.rows()?);
        let cols = usize::from(snapshot.cols()?);
        let dirty = snapshot.dirty()?;
        let palette = default_terminal_palette();
        let rebuild_all = dirty == Dirty::Full
            || self.lines.len() != rows
            || self.lines.iter().any(|line| line.len() != cols)
            || self.palette_id != Some(palette.id);
        self.converted_rows = 0;
        if dirty != Dirty::Clean || rebuild_all {
            if rebuild_all {
                self.lines.clear();
                self.line_metadata.clear();
                self.lines.reserve(rows);
                self.line_metadata.reserve(rows);
            }
            let mut row_iter = self.row_iter.update(&snapshot)?;
            let mut row_index = 0;
            while let Some(row) = row_iter.next() {
                if rebuild_all || row.dirty()? {
                    let metadata = line_metadata_from_ghostty(row.raw_row()?)?;
                    let mut line = Vec::with_capacity(cols);
                    let mut cell_iter = self.cell_iter.update(row)?;
                    while let Some(cell) = cell_iter.next() {
                        line.push(terminal_cell_from_ghostty(cell, &colors)?);
                    }
                    if rebuild_all {
                        self.lines.push(line.into());
                        self.line_metadata.push(metadata);
                    } else {
                        self.lines[row_index] = line.into();
                        self.line_metadata[row_index] = metadata;
                    }
                    self.converted_rows += 1;
                }
                row.set_dirty(false)?;
                row_index += 1;
            }
            snapshot.set_dirty(Dirty::Clean)?;
        }
        self.palette_id = Some(palette.id);

        Ok(TerminalContent {
            lines: self.lines.clone(),
            line_metadata: self.line_metadata.clone(),
            cursor_line,
            cursor_col,
            cursor_visible,
            display_offset: scrollbar
                .total
                .saturating_sub(scrollbar.len)
                .saturating_sub(scrollbar.offset) as usize,
            colors: TerminalColors {
                foreground: rgb_to_hex(colors.foreground),
                background: rgb_to_hex(colors.background),
                cursor: Some(
                    colors
                        .cursor
                        .map(rgb_to_hex)
                        .unwrap_or_else(|| palette.cursor.to_string()),
                ),
            },
            mouse: mouse_mode(terminal),
        })
    }

    pub fn last_converted_rows(&self) -> usize {
        self.converted_rows
    }
}

pub fn mouse_mode(terminal: &Terminal<'static, 'static>) -> MouseMode {
    MouseMode {
        click: terminal.mode(Mode::NORMAL_MOUSE).unwrap_or(false)
            || terminal.mode(Mode::X10_MOUSE).unwrap_or(false),
        drag: terminal.mode(Mode::BUTTON_MOUSE).unwrap_or(false),
        motion: terminal.mode(Mode::ANY_MOUSE).unwrap_or(false),
        sgr: terminal.mode(Mode::SGR_MOUSE).unwrap_or(false),
        utf8: terminal.mode(Mode::UTF8_MOUSE).unwrap_or(false),
    }
}

fn line_metadata_from_ghostty(
    row: libghostty_vt::screen::Row,
) -> libghostty_vt::error::Result<TerminalLineMetadata> {
    Ok(TerminalLineMetadata {
        wrapped: row.is_wrapped()?,
        wrap_continuation: row.is_wrap_continuation()?,
        semantic_prompt: match row.semantic_prompt()? {
            RowSemanticPrompt::None => TerminalSemanticPrompt::None,
            RowSemanticPrompt::Prompt => TerminalSemanticPrompt::Prompt,
            RowSemanticPrompt::Continuation => TerminalSemanticPrompt::Continuation,
        },
    })
}

fn terminal_cell_from_ghostty(
    cell: &libghostty_vt::render::CellIteration<'_, '_>,
    colors: &libghostty_vt::render::Colors,
) -> libghostty_vt::error::Result<TerminalCell> {
    let raw = cell.raw_cell()?;
    let wide = raw.wide()?;
    if !raw.has_text()? && !raw.has_styling()? {
        let mut cell = TerminalCell::blank();
        cell.wide_spacer = matches!(wide, CellWide::SpacerTail | CellWide::SpacerHead);
        return Ok(cell);
    }
    let text = match cell.graphemes()?.into_iter().collect::<String>() {
        text if text.is_empty() => Cow::Borrowed(" "),
        text => Cow::Owned(text),
    };
    let style = cell.style()?;
    Ok(TerminalCell {
        text,
        fg: cell
            .fg_color()?
            .map(rgb_to_hex)
            .or_else(|| style_color_to_hex(style.fg_color, colors)),
        bg: cell
            .bg_color()?
            .map(rgb_to_hex)
            .or_else(|| style_color_to_hex(style.bg_color, colors)),
        bold: style.bold,
        italic: style.italic,
        underline: style.underline != Underline::None,
        inverse: style.inverse,
        strikeout: style.strikethrough,
        wide: wide == CellWide::Wide,
        wide_spacer: matches!(wide, CellWide::SpacerTail | CellWide::SpacerHead),
    })
}

fn style_color_to_hex(
    color: libghostty_vt::style::StyleColor,
    colors: &libghostty_vt::render::Colors,
) -> Option<String> {
    match color {
        libghostty_vt::style::StyleColor::None => None,
        libghostty_vt::style::StyleColor::Palette(index) => {
            let index = usize::from(index.0);
            colors.palette.get(index).copied().map(rgb_to_hex)
        }
        libghostty_vt::style::StyleColor::Rgb(rgb) => Some(rgb_to_hex(rgb)),
    }
}

fn rgb_to_hex(color: RgbColor) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
}

pub fn apply_terminal_palette(
    terminal: &mut Terminal<'static, 'static>,
    palette: &TerminalPalette,
) -> libghostty_vt::error::Result<()> {
    let mut indexed = terminal.default_color_palette()?;
    for (target, color) in indexed.iter_mut().zip(palette.indexed) {
        *target = terminal_rgb(color);
    }
    terminal
        .set_default_fg_color(Some(terminal_rgb(palette.foreground)))?
        .set_default_bg_color(Some(terminal_rgb(palette.background)))?
        .set_default_cursor_color(Some(terminal_rgb(palette.cursor)))?
        .set_default_color_palette(Some(indexed))?;
    Ok(())
}

fn terminal_rgb(color: &str) -> RgbColor {
    let color = crate::terminal_palette::TerminalRgb::from_hex(color);
    RgbColor {
        r: color.red,
        g: color.green,
        b: color.blue,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libghostty_vt::TerminalOptions;

    #[test]
    fn snapshot_exports_soft_wrap_metadata() {
        let mut terminal = Terminal::new(TerminalOptions {
            cols: 4,
            rows: 3,
            max_scrollback: 100,
        })
        .expect("terminal");
        terminal.vt_write(b"abcde");
        let mut snapshotter = GhosttySnapshotter::new().expect("snapshotter");
        let snapshot = snapshotter.snapshot(&terminal).expect("snapshot");
        assert!(snapshot.line_metadata[0].wrapped);
        assert!(snapshot.line_metadata[1].wrap_continuation);
    }

    #[test]
    fn snapshot_uses_app_palette_for_raw_vt_default_colors() {
        let mut terminal = Terminal::new(TerminalOptions {
            cols: 4,
            rows: 3,
            max_scrollback: 100,
        })
        .expect("terminal");
        apply_terminal_palette(&mut terminal, default_terminal_palette()).expect("palette");
        let mut snapshotter = GhosttySnapshotter::new().expect("snapshotter");
        let snapshot = snapshotter.snapshot(&terminal).expect("snapshot");
        assert_eq!(
            snapshot.colors.background,
            default_terminal_palette().background
        );
        assert_eq!(
            snapshot.colors.cursor.as_deref(),
            Some(default_terminal_palette().cursor)
        );
    }

    #[test]
    fn snapshot_preserves_osc_palette_overrides() {
        let mut terminal = Terminal::new(TerminalOptions {
            cols: 4,
            rows: 3,
            max_scrollback: 100,
        })
        .expect("terminal");
        apply_terminal_palette(&mut terminal, default_terminal_palette()).expect("palette");
        terminal.vt_write(b"\x1b]4;1;rgb:12/34/56\x07\x1b[31mR");
        let mut snapshotter = GhosttySnapshotter::new().expect("snapshotter");
        let snapshot = snapshotter.snapshot(&terminal).expect("snapshot");

        assert_eq!(snapshot.lines[0][0].fg.as_deref(), Some("#123456"));
    }

    #[test]
    fn snapshot_resolves_ansi_colors_from_the_selected_palette() {
        let mut terminal = Terminal::new(TerminalOptions {
            cols: 4,
            rows: 3,
            max_scrollback: 100,
        })
        .expect("terminal");
        apply_terminal_palette(&mut terminal, default_terminal_palette()).expect("palette");
        terminal.vt_write(b"\x1b[31mR");
        let mut snapshotter = GhosttySnapshotter::new().expect("snapshotter");
        let snapshot = snapshotter.snapshot(&terminal).expect("snapshot");

        assert_eq!(
            snapshot.lines[0][0].fg.as_deref(),
            Some(default_terminal_palette().indexed[1])
        );
    }

    #[test]
    fn snapshot_rebuilds_only_dirty_rows() {
        let mut terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 55,
            max_scrollback: 100,
        })
        .expect("terminal");
        let mut snapshotter = GhosttySnapshotter::new().expect("snapshotter");

        snapshotter.snapshot(&terminal).expect("initial snapshot");
        assert_eq!(snapshotter.last_converted_rows(), 55);
        snapshotter.snapshot(&terminal).expect("clean snapshot");
        assert_eq!(snapshotter.last_converted_rows(), 0);

        terminal.vt_write(b"x");
        let snapshot = snapshotter.snapshot(&terminal).expect("partial snapshot");
        assert_eq!(snapshotter.last_converted_rows(), 1);
        assert_eq!(snapshot.lines[0][0].text, "x");

        snapshotter.palette_id = Some("different-palette");
        snapshotter.snapshot(&terminal).expect("palette snapshot");
        assert_eq!(snapshotter.last_converted_rows(), 55);
    }
}
