use crate::backend::RenderableContentOwned;
use crate::cell_text::lines_to_text;
use crate::terminal_grid::{TerminalCell, TerminalLineMetadata, TerminalSemanticPrompt};
use serde::Serialize;
use std::fs::{File, create_dir_all};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize)]
struct CellJson {
    text: String,
    fg: String,
    bg: String,
    bold: bool,
    underline: bool,
    italic: bool,
    inverse: bool,
    wide: bool,
    wide_spacer: bool,
}

#[derive(Serialize)]
struct SnapshotJson {
    rows: usize,
    cols: usize,
    cursor_line: i32,
    cursor_col: i32,
    cursor_visible: bool,
    display_offset: usize,
    mouse: MouseJson,
    text: String,
    lines: Vec<LineJson>,
}

#[derive(Serialize)]
struct LineJson {
    wrapped: bool,
    wrap_continuation: bool,
    semantic_prompt: &'static str,
    cells: Vec<CellJson>,
}

#[derive(Serialize)]
struct MouseJson {
    click: bool,
    drag: bool,
    motion: bool,
    sgr: bool,
    utf8: bool,
}

pub fn write_snapshot(snapshot: RenderableContentOwned, label: &str) -> Option<PathBuf> {
    let dir = std::env::var("CHELOTYPE_SNAPSHOT_DIR")
        .unwrap_or_else(|_| "/tmp/chelotype_snapshots".to_string());
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let base = Path::new(&dir).join(format!("{label}_{ts}"));
    if let Err(err) = create_dir_all(base.parent().unwrap_or(Path::new("/"))) {
        eprintln!("snapshot dir error: {err}");
        return None;
    }
    let lines_json = snapshot
        .lines
        .iter()
        .enumerate()
        .map(|(line_idx, line)| {
            let metadata = snapshot
                .line_metadata
                .get(line_idx)
                .copied()
                .unwrap_or_default();
            LineJson {
                wrapped: metadata.wrapped,
                wrap_continuation: metadata.wrap_continuation,
                semantic_prompt: semantic_prompt_name(metadata),
                cells: line
                    .iter()
                    .map(|cell| CellJson {
                        text: cell.text.clone(),
                        fg: cell
                            .fg
                            .clone()
                            .unwrap_or_else(|| snapshot.colors.foreground.clone()),
                        bg: cell
                            .bg
                            .clone()
                            .unwrap_or_else(|| snapshot.colors.background.clone()),
                        bold: cell.bold,
                        underline: cell.underline,
                        italic: cell.italic,
                        inverse: cell.inverse,
                        wide: cell.wide,
                        wide_spacer: cell.wide_spacer,
                    })
                    .collect(),
            }
        })
        .collect::<Vec<LineJson>>();
    let json = SnapshotJson {
        rows: snapshot.lines.len(),
        cols: snapshot.lines.iter().map(Vec::len).max().unwrap_or(0),
        cursor_line: snapshot.cursor_line,
        cursor_col: snapshot.cursor_col,
        cursor_visible: snapshot.cursor_visible,
        display_offset: snapshot.display_offset,
        mouse: MouseJson {
            click: snapshot.mouse.click,
            drag: snapshot.mouse.drag,
            motion: snapshot.mouse.motion,
            sgr: snapshot.mouse.sgr,
            utf8: snapshot.mouse.utf8,
        },
        text: snapshot_plain_from_lines(&snapshot.lines),
        lines: lines_json,
    };
    let html = snapshot_to_html(&json);
    let txt = snapshot_to_plain(&json);
    let _ = write_file(
        base.with_extension("json"),
        serde_json::to_vec_pretty(&json).unwrap_or_default(),
    );
    let _ = write_file(base.with_extension("html"), html.into_bytes());
    let _ = write_file(base.with_extension("txt"), txt.into_bytes());
    Some(base)
}

fn snapshot_to_plain(snapshot: &SnapshotJson) -> String {
    snapshot.text.clone()
}

fn snapshot_plain_from_lines(lines: &[Vec<TerminalCell>]) -> String {
    lines_to_text(lines)
}

fn semantic_prompt_name(metadata: TerminalLineMetadata) -> &'static str {
    match metadata.semantic_prompt {
        TerminalSemanticPrompt::None => "none",
        TerminalSemanticPrompt::Prompt => "prompt",
        TerminalSemanticPrompt::Continuation => "continuation",
    }
}

fn snapshot_to_html(snapshot: &SnapshotJson) -> String {
    let mut out = String::from(
        "<html><body style=\"background:#0f1115;color:#e5e7eb;font-family:JetBrains Mono,monospace;font-size:13px;white-space:pre;\">",
    );
    for (line_idx, line) in snapshot.lines.iter().enumerate() {
        for (col_idx, cell) in line.cells.iter().enumerate() {
            if cell.wide_spacer {
                continue;
            }
            let mut span = String::from("<span style=\"");
            span.push_str(&format!("color:{};", cell.fg));
            span.push_str(&format!("background-color:{};", cell.bg));
            if cell.bold {
                span.push_str("font-weight:bold;");
            }
            if cell.italic {
                span.push_str("font-style:italic;");
            }
            if cell.underline {
                span.push_str("text-decoration:underline;");
            }
            span.push_str("\">");
            for ch in cell.text.chars() {
                span.push_str(&html_escape(ch));
            }
            span.push_str("</span>");
            if snapshot.cursor_visible
                && snapshot.cursor_line == line_idx as i32
                && snapshot.cursor_col == col_idx as i32
            {
                span.push_str("<span style=\"color:#7dd3fc\">|</span>");
            }
            out.push_str(&span);
        }
        if line_idx + 1 != snapshot.lines.len() {
            out.push('\n');
        }
    }
    out.push_str("</body></html>");
    out
}

fn html_escape(ch: char) -> String {
    match ch {
        '&' => "&amp;".to_string(),
        '<' => "&lt;".to_string(),
        '>' => "&gt;".to_string(),
        _ => ch.to_string(),
    }
}

fn write_file(path: PathBuf, bytes: Vec<u8>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        create_dir_all(parent)?;
    }
    let mut file = File::create(path)?;
    file.write_all(&bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(text: &str) -> TerminalCell {
        TerminalCell {
            text: text.to_string(),
            ..TerminalCell::blank()
        }
    }

    #[test]
    fn plain_snapshot_preserves_combining_marks() {
        let composed = cell("e\u{0301}");
        let lines = vec![vec![cell("a"), composed, cell("b")]];
        assert_eq!(snapshot_plain_from_lines(&lines), "ae\u{0301}b");
    }

    #[test]
    fn plain_snapshot_skips_wide_spacer_cells() {
        let mut wide = cell("中");
        wide.wide = true;
        let mut spacer = TerminalCell::blank();
        spacer.wide_spacer = true;
        let lines = vec![vec![cell("a"), wide, spacer, cell("b")]];
        assert_eq!(snapshot_plain_from_lines(&lines), "a中b");
    }
}
