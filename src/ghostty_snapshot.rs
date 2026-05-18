use crate::terminal_grid::{
    MouseMode, TerminalCell, TerminalColors, TerminalContent, TerminalLineMetadata,
    TerminalSemanticPrompt,
};
use libghostty_vt::render::{CellIterator, Dirty, RowIterator};
use libghostty_vt::screen::{CellWide, RowSemanticPrompt};
use libghostty_vt::style::{RgbColor, Underline};
use libghostty_vt::terminal::Mode;
use libghostty_vt::{RenderState, Terminal};

pub struct GhosttySnapshotter {
    render_state: RenderState<'static>,
    row_iter: RowIterator<'static>,
    cell_iter: CellIterator<'static>,
    cached: Option<TerminalContent>,
}

impl GhosttySnapshotter {
    pub fn new() -> libghostty_vt::error::Result<Self> {
        Ok(Self {
            render_state: RenderState::new()?,
            row_iter: RowIterator::new()?,
            cell_iter: CellIterator::new()?,
            cached: None,
        })
    }

    pub fn invalidate(&mut self) {
        self.cached = None;
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
        let full_dirty = self.cached.as_ref().is_none_or(|cached| {
            cached.lines.len() != rows || cached.lines.iter().any(|line| line.len() != cols)
        }) || dirty == Dirty::Full;
        let previous = self.cached.as_ref();
        let mut lines = Vec::with_capacity(rows);
        let mut line_metadata = Vec::with_capacity(rows);

        {
            let mut row_iter = self.row_iter.update(&snapshot)?;
            let mut row_index = 0usize;
            while let Some(row) = row_iter.next() {
                let metadata = line_metadata_from_ghostty(row.raw_row()?)?;
                if !full_dirty
                    && !row.dirty()?
                    && let Some(line) = previous.and_then(|content| content.lines.get(row_index))
                {
                    lines.push(line.clone());
                    line_metadata.push(metadata);
                    row_index += 1;
                    continue;
                }

                let mut line = Vec::with_capacity(cols);
                let mut cell_iter = self.cell_iter.update(row)?;
                while let Some(cell) = cell_iter.next() {
                    line.push(terminal_cell_from_ghostty(cell, &colors)?);
                }
                lines.push(line);
                line_metadata.push(metadata);
                row_index += 1;
            }
        }

        let content = TerminalContent {
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
                foreground: rgb_to_hex(colors.foreground),
                background: rgb_to_hex(colors.background),
                cursor: colors.cursor.map(rgb_to_hex),
            },
            mouse: mouse_mode(terminal),
        };
        self.cached = Some(content.clone());
        Ok(content)
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
    let text = match cell.graphemes()?.into_iter().collect::<String>() {
        text if text.is_empty() => " ".to_string(),
        text => text,
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
}
