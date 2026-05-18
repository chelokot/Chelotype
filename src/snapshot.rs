use crate::backend::RenderableContentOwned;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color, NamedColor};
use serde::Serialize;
use std::fs::{File, create_dir_all};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize)]
struct CellJson {
    ch: char,
    fg: String,
    bg: String,
    bold: bool,
    underline: bool,
    italic: bool,
    inverse: bool,
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
    lines: Vec<Vec<CellJson>>,
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
        .map(|line| {
            line.iter()
                .map(|cell| CellJson {
                    ch: cell.c,
                    fg: hex_for(cell.fg, &snapshot.colors),
                    bg: hex_for(cell.bg, &snapshot.colors),
                    bold: cell.flags.contains(Flags::BOLD),
                    underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
                    italic: cell.flags.contains(Flags::ITALIC),
                    inverse: cell.flags.contains(Flags::INVERSE),
                })
                .collect()
        })
        .collect::<Vec<Vec<CellJson>>>();
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

fn snapshot_plain_from_lines(lines: &[Vec<alacritty_terminal::term::cell::Cell>]) -> String {
    let mut out = String::new();
    for (idx, line) in lines.iter().enumerate() {
        for cell in line {
            out.push(cell.c);
        }
        if idx + 1 != lines.len() {
            out.push('\n');
        }
    }
    out
}

fn snapshot_to_html(snapshot: &SnapshotJson) -> String {
    let mut out = String::from(
        "<html><body style=\"background:#0f1115;color:#e5e7eb;font-family:JetBrains Mono,monospace;font-size:13px;white-space:pre;\">",
    );
    for (line_idx, line) in snapshot.lines.iter().enumerate() {
        for (col_idx, cell) in line.iter().enumerate() {
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
            span.push_str(&html_escape(cell.ch));
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

fn hex_for(color: Color, palette: &alacritty_terminal::term::color::Colors) -> String {
    match color {
        Color::Named(named) => palette[named]
            .map(|rgb| format!("#{:02x}{:02x}{:02x}", rgb.r, rgb.g, rgb.b))
            .unwrap_or_else(|| default_named(named)),
        Color::Indexed(idx) => palette[idx as usize]
            .map(|rgb| format!("#{:02x}{:02x}{:02x}", rgb.r, rgb.g, rgb.b))
            .unwrap_or_else(|| default_indexed(idx)),
        Color::Spec(rgb) => format!("#{:02x}{:02x}{:02x}", rgb.r, rgb.g, rgb.b),
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

fn default_named(color: NamedColor) -> String {
    match color {
        NamedColor::Background => "#0f1115".to_string(),
        NamedColor::Foreground => "#e5e7eb".to_string(),
        NamedColor::Black => "#000000".to_string(),
        NamedColor::Red => "#ff5f5f".to_string(),
        NamedColor::Green => "#5fff87".to_string(),
        NamedColor::Yellow => "#f1fa8c".to_string(),
        NamedColor::Blue => "#5fafff".to_string(),
        NamedColor::Magenta => "#ff7bff".to_string(),
        NamedColor::Cyan => "#5fffff".to_string(),
        NamedColor::White => "#e5e7eb".to_string(),
        NamedColor::BrightBlack => "#555555".to_string(),
        NamedColor::BrightRed => "#ff7b7b".to_string(),
        NamedColor::BrightGreen => "#7bffaf".to_string(),
        NamedColor::BrightYellow => "#ffffb3".to_string(),
        NamedColor::BrightBlue => "#7bb7ff".to_string(),
        NamedColor::BrightMagenta => "#ff9dff".to_string(),
        NamedColor::BrightCyan => "#7bffff".to_string(),
        NamedColor::BrightWhite => "#ffffff".to_string(),
        _ => "#e5e7eb".to_string(),
    }
}

fn default_indexed(idx: u8) -> String {
    match idx {
        0 => "#000000".to_string(),
        1 => "#ff5f5f".to_string(),
        2 => "#5fff87".to_string(),
        3 => "#f1fa8c".to_string(),
        4 => "#5fafff".to_string(),
        5 => "#ff7bff".to_string(),
        6 => "#5fffff".to_string(),
        7 => "#e5e7eb".to_string(),
        8 => "#555555".to_string(),
        9 => "#ff7b7b".to_string(),
        10 => "#7bffaf".to_string(),
        11 => "#ffffb3".to_string(),
        12 => "#7bb7ff".to_string(),
        13 => "#ff9dff".to_string(),
        14 => "#7bffff".to_string(),
        15 => "#ffffff".to_string(),
        _ => "#e5e7eb".to_string(),
    }
}
