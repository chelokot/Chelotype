use crate::backend::TerminalBackend;
use crate::render::{RenderLine, Renderer};
use crate::selection::{GridPoint, SelectionRange, selected_text};
use crate::snapshot::write_snapshot;
use gtk::glib;
use serde::Serialize;
use std::fs::File;
use std::fs::create_dir_all;
use std::io::Write;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::{Duration, Instant};

pub fn run_headless() -> glib::ExitCode {
    match run_headless_scenario() {
        Ok(_) => glib::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("headless error: {err}");
            glib::ExitCode::from(1)
        }
    }
}

pub fn run_headless_scenario() -> std::io::Result<PathBuf> {
    let mut backend = TerminalBackend::spawn_shell()?;
    let scenario = HeadlessScenario::from_env();
    for action in &scenario.actions {
        backend.write(action.as_bytes())?;
        sleep(Duration::from_millis(scenario.step_delay_ms));
    }
    let content = wait_for_headless_content(&backend, &scenario.expected)?;
    let rendered = Renderer::render_frame_with_selection(content.clone(), scenario.selection);
    let path = write_snapshot(content.clone(), "headless")
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "snapshot write failed"))?;
    write_render_dump(
        path.clone(),
        &rendered.history_markup,
        &rendered.input_markup,
        &rendered.lines,
        scenario
            .selection
            .map(|selection| selected_text(&content.lines, selection)),
        scenario.selection,
    )?;
    Ok(path)
}

struct HeadlessScenario {
    actions: Vec<String>,
    expected: Vec<String>,
    step_delay_ms: u64,
    selection: Option<SelectionRange>,
}

impl HeadlessScenario {
    fn from_env() -> Self {
        let actions = std::env::var("CHELOTYPE_HEADLESS_ACTIONS")
            .ok()
            .map(|value| value.split('|').map(decode_action).collect())
            .unwrap_or_else(default_actions);
        let expected = std::env::var("CHELOTYPE_HEADLESS_EXPECT")
            .ok()
            .map(|value| value.split('|').map(ToOwned::to_owned).collect())
            .unwrap_or_else(default_expected);
        let step_delay_ms = std::env::var("CHELOTYPE_HEADLESS_STEP_MS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(120);
        let selection = std::env::var("CHELOTYPE_HEADLESS_SELECTION")
            .ok()
            .and_then(|value| parse_selection_range(&value));
        Self {
            actions,
            expected,
            step_delay_ms,
            selection,
        }
    }
}

fn default_actions() -> Vec<String> {
    [
        "printf '\\nchelotype headless snapshot\\n'\n",
        "echo prompt>$PS1\n",
        "echo color-test && printf '\\e[31mred\\e[0m \\e[32mgreen\\e[0m \\e[34mblue\\e[0m\\n'\n",
    ]
    .into_iter()
    .map(ToOwned::to_owned)
    .collect()
}

fn default_expected() -> Vec<String> {
    [
        "chelotype headless snapshot",
        "color-test",
        "red green blue",
    ]
    .into_iter()
    .map(ToOwned::to_owned)
    .collect()
}

fn decode_action(input: &str) -> String {
    let mut output = String::new();
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => output.push('\n'),
                Some('r') => output.push('\r'),
                Some('t') => output.push('\t'),
                Some('e') => output.push('\x1b'),
                Some('x') => {
                    let first = chars.next();
                    let second = chars.next();
                    match (first, second) {
                        (Some(first), Some(second)) => {
                            let value = [first, second].iter().collect::<String>();
                            if let Ok(byte) = u8::from_str_radix(&value, 16) {
                                output.push(byte as char);
                            } else {
                                output.push_str("\\x");
                                output.push(first);
                                output.push(second);
                            }
                        }
                        (Some(first), None) => {
                            output.push_str("\\x");
                            output.push(first);
                        }
                        _ => output.push_str("\\x"),
                    }
                }
                Some('\\') => output.push('\\'),
                Some(other) => {
                    output.push('\\');
                    output.push(other);
                }
                None => output.push('\\'),
            }
        } else {
            output.push(ch);
        }
    }
    output
}

fn wait_for_headless_content(
    backend: &TerminalBackend,
    expected: &[String],
) -> std::io::Result<crate::backend::RenderableContentOwned> {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if let Some(content) = backend.snapshot_renderable() {
            let text = content
                .lines
                .iter()
                .flat_map(|line| line.iter().map(|cell| cell.c))
                .collect::<String>();
            if expected.iter().all(|needle| text.contains(needle)) {
                return Ok(content);
            }
        }
        sleep(Duration::from_millis(20));
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "headless content did not appear",
    ))
}

fn write_render_dump(
    base: PathBuf,
    history: &str,
    input: &str,
    lines: &[RenderLine],
    selected_text: Option<String>,
    selection: Option<SelectionRange>,
) -> std::io::Result<()> {
    let dump = RenderDump {
        history_markup: history,
        input_markup: input,
        lines,
        selected_text: selected_text.as_deref(),
        selection,
    };
    let json = serde_json::to_vec_pretty(&dump)?;
    let html = format!(
        "<html><body style=\"background:#0f1115;color:#e5e7eb;font-family:JetBrains Mono,monospace;font-size:13px;white-space:pre;\">{}</body></html>",
        format!(
            "{}{}",
            history,
            if input.is_empty() {
                "".to_string()
            } else {
                format!("\n{input}")
            }
        )
    );
    if let Some(parent) = base.parent() {
        create_dir_all(parent)?;
    }
    let mut json_file = File::create(base.with_extension("render.json"))?;
    json_file.write_all(&json)?;
    let mut file = File::create(base.with_extension("markup.html"))?;
    file.write_all(html.as_bytes())
}

#[derive(Serialize)]
struct RenderDump<'a> {
    history_markup: &'a str,
    input_markup: &'a str,
    lines: &'a [RenderLine],
    selected_text: Option<&'a str>,
    selection: Option<SelectionRange>,
}

fn parse_selection_range(input: &str) -> Option<SelectionRange> {
    let (start, end) = input.split_once(':')?;
    Some(SelectionRange::between_cells(
        parse_grid_point(start)?,
        parse_grid_point(end)?,
    ))
}

fn parse_grid_point(input: &str) -> Option<GridPoint> {
    let (row, column) = input.split_once(',')?;
    Some(GridPoint {
        row: row.parse().ok()?,
        column: column.parse().ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_headless_selection_as_inclusive_cell_drag() {
        assert_eq!(
            parse_selection_range("2,3:2,5"),
            Some(SelectionRange::new(
                GridPoint { row: 2, column: 3 },
                GridPoint { row: 2, column: 6 }
            ))
        );
    }

    #[test]
    fn rejects_invalid_headless_selection() {
        assert_eq!(parse_selection_range("2,3"), None);
        assert_eq!(parse_selection_range("x,3:2,5"), None);
    }
}
