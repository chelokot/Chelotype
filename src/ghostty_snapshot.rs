use crate::terminal_grid::{
    MouseMode, TerminalCell, TerminalColors, TerminalContent, TerminalLineMetadata,
    TerminalSemanticPrompt,
};
use crate::terminal_palette::default_terminal_palette;
use libghostty_vt::render::{CellIterator, RowIterator};
use libghostty_vt::screen::{CellWide, RowSemanticPrompt};
use libghostty_vt::style::{RgbColor, Underline};
use libghostty_vt::terminal::Mode;
use libghostty_vt::{RenderState, Terminal};
use std::borrow::Cow;

pub struct GhosttySnapshotter {
    render_state: RenderState<'static>,
    row_iter: RowIterator<'static>,
    cell_iter: CellIterator<'static>,
}

impl GhosttySnapshotter {
    pub fn new() -> libghostty_vt::error::Result<Self> {
        Ok(Self {
            render_state: RenderState::new()?,
            row_iter: RowIterator::new()?,
            cell_iter: CellIterator::new()?,
        })
    }

    pub fn invalidate(&mut self) {}

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
        let mut lines = Vec::with_capacity(rows);
        let mut line_metadata = Vec::with_capacity(rows);

        {
            let mut row_iter = self.row_iter.update(&snapshot)?;
            while let Some(row) = row_iter.next() {
                let metadata = line_metadata_from_ghostty(row.raw_row()?)?;
                let mut line = Vec::with_capacity(cols);
                let mut cell_iter = self.cell_iter.update(row)?;
                while let Some(cell) = cell_iter.next() {
                    line.push(terminal_cell_from_ghostty(cell, &colors)?);
                }
                lines.push(line);
                line_metadata.push(metadata);
            }
        }

        Ok(TerminalContent {
            lines,
            line_metadata,
            cursor_line,
            cursor_col,
            cursor_visible,
            display_offset: scrollbar
                .total
                .saturating_sub(scrollbar.len)
                .saturating_sub(scrollbar.offset) as usize,
            colors: TerminalColors {
                foreground: rgb_to_terminal_foreground(colors.foreground),
                background: rgb_to_terminal_background(colors.background),
                cursor: colors
                    .cursor
                    .map(rgb_to_hex)
                    .or_else(|| Some(default_terminal_palette().cursor.to_string())),
            },
            mouse: mouse_mode(terminal),
        })
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
        libghostty_vt::style::StyleColor::Palette(index) => colors
            .palette
            .get(usize::from(index.0))
            .copied()
            .map(rgb_to_hex),
        libghostty_vt::style::StyleColor::Rgb(rgb) => Some(rgb_to_hex(rgb)),
    }
}

fn rgb_to_hex(color: RgbColor) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
}

fn rgb_to_terminal_foreground(color: RgbColor) -> String {
    if color.r == 0xff && color.g == 0xff && color.b == 0xff {
        return default_terminal_palette().foreground.to_string();
    }
    rgb_to_hex(color)
}

fn rgb_to_terminal_background(color: RgbColor) -> String {
    if color.r == 0 && color.g == 0 && color.b == 0 {
        return default_terminal_palette().background.to_string();
    }
    rgb_to_hex(color)
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
        let terminal = Terminal::new(TerminalOptions {
            cols: 4,
            rows: 3,
            max_scrollback: 100,
        })
        .expect("terminal");
        let mut snapshotter = GhosttySnapshotter::new().expect("snapshotter");
        let snapshot = snapshotter.snapshot(&terminal).expect("snapshot");
        assert_eq!(
            snapshot.colors.background,
            default_terminal_palette().background
        );
        assert_eq!(snapshot.colors.cursor.as_deref(), Some("#ffffff"));
    }
}
