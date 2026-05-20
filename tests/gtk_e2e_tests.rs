use serial_test::serial;
use std::fs::{read_dir, read_to_string};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn has_command(name: &str) -> bool {
    Command::new("bash")
        .args([
            "--noprofile",
            "--norc",
            "-lc",
            &format!("command -v {name}"),
        ])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn snapshot_paths(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut paths = read_dir(dir)
        .expect("snapshot files")
        .map(|entry| entry.expect("snapshot entry").path())
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn assert_clean_gtk_stderr(stderr: &str) {
    assert!(!stderr.contains("Gtk-WARNING"), "{stderr}");
    assert!(!stderr.contains("panic"), "{stderr}");
    assert!(!stderr.contains("error:"), "{stderr}");
}

fn write_fake_toolbox(bin_dir: &std::path::Path, log_path: &std::path::Path, container_name: &str) {
    let fake_toolbox = bin_dir.join("toolbox");
    std::fs::write(
        &fake_toolbox,
        format!(
            r#"#!/usr/bin/env bash
set -euo pipefail
log={log:?}
container_name={container_name:?}
if [ "${{1:-}}" = "list" ] && [ "${{2:-}}" = "--containers" ]; then
    printf 'CONTAINER ID  CONTAINER NAME           CREATED       STATUS   IMAGE NAME\n'
    printf 'abc123        %s    today         running  image\n' "$container_name"
    exit 0
fi
if [ "${{1:-}}" = "enter" ] && [ "${{2:-}}" = "--container" ]; then
    container="$3"
    shift 3
    printf 'enter\t%s\t%s\n' "$container" "$*" >> "$log"
    exec "$@"
fi
printf 'unexpected\t%s\n' "$*" >> "$log"
exit 1
"#,
            log = log_path.to_string_lossy(),
            container_name = container_name,
        ),
    )
    .expect("fake toolbox script");
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&fake_toolbox)
            .expect("fake toolbox metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&fake_toolbox, permissions).expect("fake toolbox executable");
    }
}

fn json_snapshots(dir: &std::path::Path) -> Vec<serde_json::Value> {
    snapshot_paths(dir)
        .into_iter()
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .map(|path| read_to_string(path).expect("read json snapshot"))
        .map(|json| serde_json::from_str::<serde_json::Value>(&json).expect("valid snapshot json"))
        .collect()
}

fn snapshot_has_colored_text(snapshot: &serde_json::Value, needle: &str) -> bool {
    let Some(lines) = snapshot["lines"].as_array() else {
        return false;
    };
    lines.iter().any(|line| {
        let Some(cells) = line["cells"].as_array() else {
            return false;
        };
        let line_text = cells
            .iter()
            .filter_map(|cell| cell["text"].as_str())
            .collect::<String>();
        line_text.trim_end() == needle
            && cells.iter().any(|cell| {
                cell["text"].as_str().is_some_and(|text| !text.is_empty())
                    && cell["fg"].as_str().is_some_and(|color| color != "#e5e7eb")
            })
    })
}

fn red_pixel_count(image: &std::path::Path) -> usize {
    let output = Command::new("convert")
        .args([image.to_str().expect("image path utf8"), "txt:-"])
        .output()
        .expect("convert screenshot to pixels");
    assert!(
        output.status.success(),
        "convert failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| {
            parse_srgb(line).is_some_and(|[red, green, blue]| red > 180 && green < 90 && blue < 90)
        })
        .count()
}

fn cursor_pixel_count(image: &std::path::Path) -> usize {
    pixel_bounds(image, |pixel| {
        pixel.red == 125 && pixel.green == 211 && pixel.blue == 252
    })
    .map(|bounds| bounds.count)
    .unwrap_or(0)
}

fn command_block_rail_pixel_count(image: &std::path::Path) -> usize {
    pixel_bounds(image, |pixel| {
        pixel.red == 46 && pixel.green == 166 && pixel.blue == 199
    })
    .map(|bounds| bounds.count)
    .unwrap_or(0)
}

fn geometry_metric(path: &std::path::Path, name: &str) -> f64 {
    let geometry = read_to_string(path).expect("read geometry trace");
    geometry
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once('=')?;
            (key == name).then(|| value.parse::<f64>().expect("numeric geometry metric"))
        })
        .unwrap_or_else(|| panic!("missing geometry metric {name}: {geometry}"))
}

fn perf_samples(path: &std::path::Path, event: &str) -> Vec<Duration> {
    read_to_string(path)
        .expect("read perf trace")
        .lines()
        .filter_map(|line| {
            let (kind, micros) = line.split_once('\t')?;
            if kind != event {
                return None;
            }
            Some(Duration::from_micros(micros.parse().expect("perf micros")))
        })
        .collect()
}

fn percentile_duration(mut samples: Vec<Duration>, percentile: usize) -> Duration {
    samples.sort_unstable();
    let index = ((samples.len() - 1) * percentile) / 100;
    samples[index]
}

fn perf_counters(path: &std::path::Path, event: &str) -> Vec<u64> {
    read_to_string(path)
        .expect("read perf trace")
        .lines()
        .filter_map(|line| {
            let (kind, value) = line.split_once('\t')?;
            if kind != event {
                return None;
            }
            Some(value.parse().expect("perf counter"))
        })
        .collect()
}

fn percentile_counter(mut samples: Vec<u64>, percentile: usize) -> u64 {
    samples.sort_unstable();
    let index = ((samples.len() - 1) * percentile) / 100;
    samples[index]
}

#[derive(Clone, Copy)]
struct ImagePixel {
    x: usize,
    y: usize,
    red: u16,
    green: u16,
    blue: u16,
}

#[derive(Debug)]
struct PixelBounds {
    min_x: usize,
    max_x: usize,
    min_y: usize,
    max_y: usize,
    count: usize,
}

impl PixelBounds {
    fn width(&self) -> usize {
        self.max_x - self.min_x + 1
    }

    fn height(&self) -> usize {
        self.max_y - self.min_y + 1
    }
}

fn pixel_bounds(
    image: &std::path::Path,
    matches: impl Fn(ImagePixel) -> bool,
) -> Option<PixelBounds> {
    let output = Command::new("convert")
        .args([image.to_str().expect("image path utf8"), "txt:-"])
        .output()
        .expect("convert screenshot to pixels");
    assert!(
        output.status.success(),
        "convert failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut bounds: Option<PixelBounds> = None;
    for pixel in String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_image_pixel)
        .filter(|pixel| matches(*pixel))
    {
        bounds = Some(match bounds {
            Some(bounds) => PixelBounds {
                min_x: bounds.min_x.min(pixel.x),
                max_x: bounds.max_x.max(pixel.x),
                min_y: bounds.min_y.min(pixel.y),
                max_y: bounds.max_y.max(pixel.y),
                count: bounds.count + 1,
            },
            None => PixelBounds {
                min_x: pixel.x,
                max_x: pixel.x,
                min_y: pixel.y,
                max_y: pixel.y,
                count: 1,
            },
        });
    }
    bounds
}

fn parse_srgb(line: &str) -> Option<[u16; 3]> {
    let start = line.find("srgb(")? + "srgb(".len();
    let end = line[start..].find(')')? + start;
    let mut parts = line[start..end]
        .split(',')
        .map(|part| part.trim().parse::<u16>().ok());
    Some([parts.next()??, parts.next()??, parts.next()??])
}

fn parse_image_pixel(line: &str) -> Option<ImagePixel> {
    let (position, rest) = line.split_once(':')?;
    let (x, y) = position.split_once(',')?;
    let [red, green, blue] = parse_srgb(rest)?;
    Some(ImagePixel {
        x: x.trim().parse().ok()?,
        y: y.trim().parse().ok()?,
        red,
        green,
        blue,
    })
}

#[test]
#[serial]
fn gtk_e2e_renders_real_window_to_snapshot_under_xvfb() {
    if !has_command("xvfb-run") {
        eprintln!("skipping gtk e2e because xvfb-run is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");

    let output = Command::new("xvfb-run")
        .args(["-a", env!("CARGO_BIN_EXE_chelotype")])
        .env("GDK_BACKEND", "x11")
        .env("GSETTINGS_BACKEND", "memory")
        .env("NO_AT_BRIDGE", "1")
        .env("CHELOTYPE_UI_E2E", "1")
        .env("CHELOTYPE_SNAPSHOT_DIR", &dir)
        .env("CHELOTYPE_UI_E2E_INPUT", "printf 'GTK_E2E_OK\\n'\n")
        .env("CHELOTYPE_UI_E2E_EXPECT", "GTK_E2E_OK")
        .output()
        .expect("run gtk e2e binary under xvfb");

    assert!(
        output.status.success(),
        "gtk e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let json_snapshot = snapshot_paths(&dir)
        .into_iter()
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .expect("json snapshot");
    let json = read_to_string(json_snapshot).expect("read json snapshot");
    assert!(json.contains("GTK_E2E_OK"));
    assert!(json.contains("\"lines\""));
    assert!(json.contains("\"cells\""));
    assert!(json.contains("\"cursor_visible\""));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_exports_colored_cells_under_xvfb() {
    if !has_command("xvfb-run") {
        eprintln!("skipping gtk color e2e because xvfb-run is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-color-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");

    let output = Command::new("xvfb-run")
        .args(["-a", env!("CARGO_BIN_EXE_chelotype")])
        .env("GDK_BACKEND", "x11")
        .env("GSETTINGS_BACKEND", "memory")
        .env("NO_AT_BRIDGE", "1")
        .env("CHELOTYPE_UI_E2E", "1")
        .env("CHELOTYPE_SNAPSHOT_DIR", &dir)
        .env(
            "CHELOTYPE_UI_E2E_INPUT",
            "printf '\\033[31mGTK_RED_%s\\033[0m\\n' STYLE; printf 'GTK_COLOR_%s\\n' DONE\n",
        )
        .env("CHELOTYPE_UI_E2E_EXPECT", "GTK_COLOR_DONE")
        .output()
        .expect("run gtk color e2e binary under xvfb");

    assert!(
        output.status.success(),
        "gtk color e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let snapshots = json_snapshots(&dir);
    assert!(
        snapshots
            .iter()
            .any(|snapshot| snapshot_has_colored_text(snapshot, "GTK_RED_STYLE")),
        "colored GTK_RED_STYLE output was not preserved in JSON snapshots"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_exports_unicode_grapheme_and_width_cells_under_xvfb() {
    if !has_command("xvfb-run") {
        eprintln!("skipping gtk unicode e2e because xvfb-run is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-unicode-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");

    let combining = "e\u{0301}";
    let wide = "\u{4e2d}";
    let zwj_emoji = "\u{1f469}\u{200d}\u{1f4bb}";
    let middle_dot = "\u{00b7}";
    let omega = "\u{03a9}";
    let expected =
        format!("GTK_UNICODE {combining} WIDE {wide} EMOJI {zwj_emoji} AMBIG {middle_dot} {omega}");
    let input = format!("printf '{expected}\\n'\n");

    let output = Command::new("xvfb-run")
        .args(["-a", env!("CARGO_BIN_EXE_chelotype")])
        .env("GDK_BACKEND", "x11")
        .env("GSETTINGS_BACKEND", "memory")
        .env("NO_AT_BRIDGE", "1")
        .env("CHELOTYPE_UI_E2E", "1")
        .env("CHELOTYPE_SNAPSHOT_DIR", &dir)
        .env("CHELOTYPE_UI_E2E_INPUT", input)
        .env(
            "CHELOTYPE_UI_E2E_EXPECT",
            format!("{combining}|{wide}|{zwj_emoji}|{middle_dot}|{omega}"),
        )
        .env("CHELOTYPE_UI_E2E_TIMEOUT_MS", "6000")
        .output()
        .expect("run gtk unicode e2e binary under xvfb");

    assert!(
        output.status.success(),
        "gtk unicode e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let snapshots = json_snapshots(&dir);
    let snapshot = snapshots
        .iter()
        .find(|snapshot| {
            snapshot["text"]
                .as_str()
                .is_some_and(|text| text.contains(&expected))
        })
        .unwrap_or_else(|| {
            panic!("gtk unicode snapshot did not contain {expected}: {snapshots:?}")
        });
    let cells = snapshot["lines"]
        .as_array()
        .expect("snapshot lines")
        .iter()
        .flat_map(|line| line["cells"].as_array().expect("line cells"))
        .collect::<Vec<_>>();

    assert!(
        cells
            .iter()
            .any(|cell| cell["text"].as_str() == Some(combining)
                && cell["wide"].as_bool() == Some(false)
                && cell["wide_spacer"].as_bool() == Some(false)),
        "combining grapheme was not preserved as one normal cell: {snapshot}"
    );
    assert!(
        cells.iter().any(|cell| cell["text"].as_str() == Some(wide)
            && cell["wide"].as_bool() == Some(true)),
        "CJK wide character was not exported as wide: {snapshot}"
    );
    assert!(
        cells
            .iter()
            .any(|cell| cell["wide_spacer"].as_bool() == Some(true)),
        "wide spacer metadata was not exported: {snapshot}"
    );
    assert!(
        cells.iter().any(|cell| cell["text"]
            .as_str()
            .is_some_and(|text| text.contains('\u{200d}'))
            && cell["wide"].as_bool() == Some(true)),
        "ZWJ emoji was not preserved as wide grapheme state: {snapshot}"
    );
    for text in [middle_dot, omega] {
        assert!(
            cells.iter().any(|cell| cell["text"].as_str() == Some(text)
                && cell["wide"].as_bool() == Some(false)
                && cell["wide_spacer"].as_bool() == Some(false)),
            "ambiguous-width {text} was not exported as one normal cell: {snapshot}"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_captures_nonblank_window_screenshot_under_xvfb() {
    if !has_command("xvfb-run")
        || !has_command("xdotool")
        || !has_command("import")
        || !has_command("identify")
    {
        eprintln!(
            "skipping gtk screenshot e2e because xvfb-run, xdotool, import, or identify is not installed"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-screenshot-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let screenshot = dir.join("window.png");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
screenshot="$3"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "printf 'SCREENSHOT_OK\n'"
xdotool key --window "$window_id" Return
for _ in {1..100}; do
    if grep -R 'SCREENSHOT_OK' "$snapshot_dir" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if ! grep -R 'SCREENSHOT_OK' "$snapshot_dir" >/dev/null 2>&1; then
    echo "screenshot marker never appeared" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
import -window "$window_id" "$screenshot"
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-screenshot-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            screenshot.to_str().expect("screenshot path utf8"),
        ])
        .output()
        .expect("run gtk screenshot e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk screenshot e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let identify = Command::new("identify")
        .args([
            "-format",
            "%w %h %k",
            screenshot.to_str().expect("screenshot path utf8"),
        ])
        .output()
        .expect("identify screenshot");
    assert!(
        identify.status.success(),
        "identify failed: {}",
        String::from_utf8_lossy(&identify.stderr)
    );
    let metrics = String::from_utf8_lossy(&identify.stdout);
    let values = metrics
        .split_whitespace()
        .map(|value| value.parse::<usize>().expect("numeric identify metric"))
        .collect::<Vec<_>>();
    assert_eq!(values.len(), 3, "unexpected identify metrics: {metrics}");
    assert!(values[0] > 100, "screenshot width too small: {metrics}");
    assert!(values[1] > 100, "screenshot height too small: {metrics}");
    assert!(values[2] > 1, "screenshot appears blank: {metrics}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_renders_truecolor_cells_into_window_pixels_under_xvfb() {
    if !has_command("xvfb-run")
        || !has_command("xdotool")
        || !has_command("import")
        || !has_command("identify")
        || !has_command("convert")
    {
        eprintln!(
            "skipping gtk pixel color e2e because xvfb-run, xdotool, import, identify, or convert is not installed"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-pixel-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let screenshot = dir.join("window.png");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
screenshot="$3"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "printf '\033[38;2;255;0;0mPIXEL_RED_OK\033[0m\n'; printf 'PIXEL_DONE\n'"
xdotool key --window "$window_id" Return
for _ in {1..100}; do
    if grep -R 'PIXEL_DONE' "$snapshot_dir" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if ! grep -R 'PIXEL_DONE' "$snapshot_dir" >/dev/null 2>&1; then
    echo "pixel marker never appeared" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
import -window "$window_id" "$screenshot"
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-pixel-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            screenshot.to_str().expect("screenshot path utf8"),
        ])
        .output()
        .expect("run gtk pixel color e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk pixel color e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let count = red_pixel_count(&screenshot);
    assert!(count > 20, "expected red terminal pixels, found {count}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_renders_osc133_command_block_rail_under_xvfb() {
    if !has_command("xvfb-run")
        || !has_command("xdotool")
        || !has_command("import")
        || !has_command("convert")
    {
        eprintln!(
            "skipping gtk command block e2e because xvfb-run, xdotool, import, or convert is not installed"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-command-block-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let screenshot = dir.join("window.png");
    let clipboard_trace = dir.join("clipboard.tsv");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
screenshot="$3"
clipboard_trace="$4"
geometry_trace="$5"
rm -f /tmp/chelotype.log
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_DEBUG=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_RENDER_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_CLIPBOARD_TRACE="$clipboard_trace" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "printf '\033]133;A\007CB_PROMPT\033]133;B\007\n\033]133;C\007CB_OUTPUT\nCB_DONE\n\033]133;D;0\007\033]133;A\007NEXT_PROMPT\n'"
xdotool key --window "$window_id" Return
for _ in {1..120}; do
    if grep -R 'CB_DONE' "$snapshot_dir" >/dev/null 2>&1 && grep -R '"command_blocks": \\[' "$snapshot_dir"/*.render.json >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if ! grep -R 'CB_DONE' "$snapshot_dir" >/dev/null 2>&1; then
    echo "command block output never appeared" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
if ! grep -R '"prompt_start_row"' "$snapshot_dir"/*.render.json >/dev/null 2>&1; then
    echo "command block render metadata never appeared" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
xdotool key --window "$window_id" ctrl+shift+Up
for _ in {1..100}; do
    if grep -F 'primary	CB_OUTPUT\nCB_DONE' "$clipboard_trace" >/dev/null 2>&1 && grep -R '"selected_text": "CB_OUTPUT\nCB_DONE"' "$snapshot_dir" >/dev/null 2>&1; then
        break
    fi
    sleep 0.05
done
if ! grep -F 'primary	CB_OUTPUT\nCB_DONE' "$clipboard_trace" >/dev/null 2>&1; then
    echo "command block keyboard shortcut did not select previous output" >&2
    cat "$clipboard_trace" >&2 || true
    grep -R '"selected_text"' "$snapshot_dir" >&2 || true
    exit 1
fi
xdotool key --window "$window_id" ctrl+shift+c
for _ in {1..100}; do
    if grep -F 'clipboard	CB_OUTPUT\nCB_DONE' "$clipboard_trace" >/dev/null 2>&1; then
        break
    fi
    sleep 0.05
done
if ! grep -F 'clipboard	CB_OUTPUT\nCB_DONE' "$clipboard_trace" >/dev/null 2>&1; then
    echo "command block keyboard-selected output did not copy" >&2
    cat "$clipboard_trace" >&2 || true
    exit 1
fi
: > "$clipboard_trace"
latest_txt="$(ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1)"
marker_row="$(grep -n '^CB_OUTPUT' "$latest_txt" | tail -n 1 | cut -d: -f1)"
marker_row="$((marker_row - 1))"
if [ -z "$marker_row" ] || [ "$marker_row" -lt 0 ] || [ ! -f "$geometry_trace" ]; then
    echo "could not locate command block output row" >&2
    [ -n "$latest_txt" ] && cat "$latest_txt" >&2
    exit 1
fi
canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
rail_x="$(awk -v canvas_x="$canvas_x" 'BEGIN { printf "%d", canvas_x + 6 }')"
rail_y="$(awk -v canvas_y="$canvas_y" -v row="$marker_row" -v line="$line_height" 'BEGIN { printf "%d", canvas_y + ((row + 0.5) * line) }')"
echo "command block rail click window=$window_id row=$marker_row at=$rail_x,$rail_y" >&2
xdotool windowfocus "$window_id" || true
eval "$(xdotool getwindowgeometry --shell "$window_id")"
rail_root_x="$(awk -v left="$X" -v x="$rail_x" 'BEGIN { printf "%d", left + x }')"
rail_root_y="$(awk -v top="$Y" -v y="$rail_y" 'BEGIN { printf "%d", top + y }')"
xdotool mousemove "$rail_root_x" "$rail_root_y"
xdotool click 3
sleep 0.2
copy_block_x="$(awk -v x="$rail_root_x" 'BEGIN { printf "%d", x + 72 }')"
copy_block_y="$(awk -v y="$rail_root_y" 'BEGIN { printf "%d", y + 24 }')"
xdotool mousemove "$copy_block_x" "$copy_block_y"
xdotool click 1
for _ in {1..100}; do
    if grep -F 'clipboard	CB_OUTPUT\nCB_DONE' "$clipboard_trace" >/dev/null 2>&1; then
        break
    fi
    sleep 0.05
done
if ! grep -F 'clipboard	CB_OUTPUT\nCB_DONE' "$clipboard_trace" >/dev/null 2>&1; then
    echo "command block context menu did not copy output" >&2
    echo "rail=$rail_root_x,$rail_root_y copy=$copy_block_x,$copy_block_y" >&2
    cat "$clipboard_trace" >&2 || true
    cat /tmp/chelotype.log >&2 || true
    exit 1
fi
xdotool mousemove --sync --window "$window_id" "$rail_x" "$rail_y"
sleep 0.08
xdotool mousedown 1
sleep 0.08
xdotool mouseup 1
for _ in {1..100}; do
    if grep -F 'primary	CB_OUTPUT\nCB_DONE' "$clipboard_trace" >/dev/null 2>&1 && grep -R '"selected_text": "CB_OUTPUT\nCB_DONE"' "$snapshot_dir" >/dev/null 2>&1; then
        break
    fi
    sleep 0.05
done
if ! grep -F 'primary	CB_OUTPUT\nCB_DONE' "$clipboard_trace" >/dev/null 2>&1; then
    echo "command block rail click did not select output" >&2
    cat "$clipboard_trace" >&2 || true
    grep -R '"selected_text"' "$snapshot_dir" >&2 || true
    cat /tmp/chelotype.log >&2 || true
    exit 1
fi
import -window "$window_id" "$screenshot"
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-command-block-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            screenshot.to_str().expect("screenshot path utf8"),
            clipboard_trace.to_str().expect("clipboard trace path utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk command block e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk command block e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let rail_pixels = command_block_rail_pixel_count(&screenshot);
    assert!(
        rail_pixels > 20,
        "expected visible command block rail pixels, found {rail_pixels}"
    );
    let trace = read_to_string(&clipboard_trace).expect("read clipboard trace");
    assert!(
        trace
            .lines()
            .any(|line| line == "primary\tCB_OUTPUT\\nCB_DONE")
    );
    assert!(
        trace
            .lines()
            .any(|line| line == "clipboard\tCB_OUTPUT\\nCB_DONE")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_command_block_rail_selection_survives_resize_reflow_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") || !has_command("python3") {
        eprintln!(
            "skipping gtk command block reflow e2e because xvfb-run, xdotool, or python3 is not installed"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-command-block-reflow-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let clipboard_trace = dir.join("clipboard.tsv");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
clipboard_trace="$3"
geometry_trace="$4"
expected='CB_WRAP_0123456789_abcdefghijklmnopqrstuvwxyz_ABCDEFGHIJKLMNOPQRSTUVWXYZ_0123456789_tail'
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_DEBUG=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_RENDER_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_CLIPBOARD_TRACE="$clipboard_trace" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
latest_txt() {
    ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1
}
render_snapshot_count() {
    find "$snapshot_dir" -maxdepth 1 -name '*.render.json' 2>/dev/null | wc -l
}
wait_latest_contains_wrapped_output() {
    for _ in {1..140}; do
        latest="$(latest_txt || true)"
        if [ -n "$latest" ] && grep -F 'CB_WRAP_' "$latest" >/dev/null 2>&1 && grep -F '0123456789_tail' "$latest" >/dev/null 2>&1 && [ -f "$geometry_trace" ]; then
            return 0
        fi
        sleep 0.1
    done
    echo "latest text did not contain wrapped command block output" >&2
    latest="$(latest_txt || true)"
    [ -n "$latest" ] && cat "$latest" >&2
    return 1
}
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 1 "printf '\033]133;A\007CB_PROMPT\033]133;B\007\n\033]133;C\007CB_WRAP_0123456789_abcdefghijklmnopqrstuvwxyz_ABCDEFGHIJKLMNOPQRSTUVWXYZ_0123456789_tail\n\033]133;D;0\007\033]133;A\007NEXT_PROMPT\n'"
xdotool key --window "$window_id" Return
wait_latest_contains_wrapped_output
before_resize_render_count="$(render_snapshot_count)"
xdotool windowsize "$window_id" 520 420
for _ in {1..140}; do
    latest="$(latest_txt || true)"
    render_count="$(render_snapshot_count)"
    if [ "$render_count" -gt "$before_resize_render_count" ] && [ -n "$latest" ] && grep -F 'CB_WRAP_' "$latest" >/dev/null 2>&1 && grep -F '0123456789_tail' "$latest" >/dev/null 2>&1 && grep -R '"prompt_start_row"' "$snapshot_dir"/*.render.json >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if [ "$(render_snapshot_count)" -le "$before_resize_render_count" ]; then
    echo "resize did not produce a fresh render snapshot" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
latest="$(latest_txt || true)"
if [ -z "$latest" ] || ! grep -F 'CB_WRAP_' "$latest" >/dev/null 2>&1 || ! grep -F '0123456789_tail' "$latest" >/dev/null 2>&1; then
    echo "resized command block output never appeared" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    [ -n "$latest" ] && cat "$latest" >&2
    exit 1
fi
marker_row="$(python3 - "$snapshot_dir" <<'PY'
import glob
import json
import os
import sys

paths = glob.glob(os.path.join(sys.argv[1], "*.render.json"))
latest = max(paths, key=os.path.getmtime)
with open(latest, encoding="utf-8") as handle:
    frame = json.load(handle)
for line in frame["lines"]:
    if line["text"].startswith("CB_WRAP_"):
        print(line["row"])
        break
PY
)"
if [ -z "$marker_row" ] || [ "$marker_row" -lt 0 ]; then
    echo "could not locate resized command block output row" >&2
    cat "$latest" >&2
    exit 1
fi
canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
eval "$(xdotool getwindowgeometry --shell "$window_id")"
rail_root_x="$(awk -v left="$X" -v canvas_x="$canvas_x" 'BEGIN { printf "%d", left + canvas_x + 6 }')"
rail_root_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$marker_row" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
xdotool mousemove "$rail_root_x" "$rail_root_y"
xdotool click 1
for _ in {1..120}; do
    if grep -Fx "primary	$expected" "$clipboard_trace" >/dev/null 2>&1 && grep -R "\"selected_text\": \"$expected\"" "$snapshot_dir" >/dev/null 2>&1; then
        break
    fi
    sleep 0.05
done
if ! grep -Fx "primary	$expected" "$clipboard_trace" >/dev/null 2>&1; then
    echo "resized command block rail click did not select reflowed output" >&2
    echo "rail=$rail_root_x,$rail_root_y row=$marker_row" >&2
    cat "$clipboard_trace" >&2 || true
    grep -R '"selected_text"' "$snapshot_dir" >&2 || true
    cat "$latest" >&2 || true
    exit 1
fi
xdotool mousemove "$rail_root_x" "$rail_root_y"
xdotool click 3
sleep 0.2
copy_block_x="$(awk -v x="$rail_root_x" 'BEGIN { printf "%d", x + 72 }')"
copy_block_y="$(awk -v y="$rail_root_y" 'BEGIN { printf "%d", y + 24 }')"
xdotool mousemove "$copy_block_x" "$copy_block_y"
xdotool click 1
for _ in {1..120}; do
    if grep -Fx "clipboard	$expected" "$clipboard_trace" >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.05
done
echo "resized command block context menu did not copy reflowed output" >&2
echo "rail=$rail_root_x,$rail_root_y copy=$copy_block_x,$copy_block_y row=$marker_row" >&2
cat "$clipboard_trace" >&2 || true
cat "$latest" >&2 || true
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-command-block-reflow-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            clipboard_trace.to_str().expect("clipboard trace path utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk command block reflow e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk command block reflow e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let trace = read_to_string(&clipboard_trace).expect("read clipboard trace");
    assert!(
        trace.lines().any(|line| line
            == "primary\tCB_WRAP_0123456789_abcdefghijklmnopqrstuvwxyz_ABCDEFGHIJKLMNOPQRSTUVWXYZ_0123456789_tail"),
        "reflowed command block output was not selected: {trace}"
    );
    assert!(
        trace.lines().any(|line| line
            == "clipboard\tCB_WRAP_0123456789_abcdefghijklmnopqrstuvwxyz_ABCDEFGHIJKLMNOPQRSTUVWXYZ_0123456789_tail"),
        "reflowed command block output was not copied from context menu: {trace}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_keeps_colored_text_on_fixed_grid_across_spaces_under_xvfb() {
    if !has_command("xvfb-run")
        || !has_command("xdotool")
        || !has_command("import")
        || !has_command("convert")
    {
        eprintln!(
            "skipping gtk grid spacing e2e because xvfb-run, xdotool, import, or convert is not installed"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-grid-spacing-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let screenshot = dir.join("window.png");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
screenshot="$3"
geometry_trace="$4"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "printf '\033[38;2;255;0;0mA\033[0m\033[38;2;102;102;102m \033[38;2;0;255;0mB\033[0m\n'; printf 'GRID_SPACE_DONE\n'"
xdotool key --window "$window_id" Return
for _ in {1..100}; do
    if grep -R 'GRID_SPACE_DONE' "$snapshot_dir" >/dev/null 2>&1 && [ -f "$geometry_trace" ]; then
        break
    fi
    sleep 0.1
done
if ! grep -R 'GRID_SPACE_DONE' "$snapshot_dir" >/dev/null 2>&1; then
    echo "grid spacing marker never appeared" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
import -window "$window_id" "$screenshot"
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-grid-spacing-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            screenshot.to_str().expect("screenshot path utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk grid spacing e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk grid spacing e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let red = pixel_bounds(&screenshot, |pixel| {
        pixel.red > 180 && pixel.green < 90 && pixel.blue < 90
    })
    .expect("red A pixels");
    let green = pixel_bounds(&screenshot, |pixel| {
        pixel.y >= red.min_y.saturating_sub(2)
            && pixel.y <= red.max_y + 2
            && pixel.green > 180
            && pixel.red < 90
            && pixel.blue < 90
    })
    .expect("green B pixels");
    let expected_delta = geometry_metric(&geometry_trace, "cell_width") * 2.0;
    let actual_delta = green.min_x as f64 - red.min_x as f64;
    assert!(
        (actual_delta - expected_delta).abs() <= 2.0,
        "colored cells after a styled space drifted off grid: red={red:?} green={green:?} expected_delta={expected_delta:.2} actual_delta={actual_delta:.2}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_scroll_wheel_drains_through_smooth_steps_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk smooth scroll e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-smooth-scroll-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let scroll_trace = dir.join("scroll.tsv");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
scroll_trace="$3"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_SCROLL_TRACE="$scroll_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 1 "python3 -c \"for n in range(1, 61): print(f'SMOOTH_SCROLL_{n}')\""
xdotool key --window "$window_id" Return
for _ in {1..120}; do
    if grep -R '^SMOOTH_SCROLL_60' "$snapshot_dir"/*.txt >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if ! grep -R '^SMOOTH_SCROLL_60' "$snapshot_dir"/*.txt >/dev/null 2>&1; then
    echo "smooth scroll output never appeared" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
eval "$(xdotool getwindowgeometry --shell "$window_id")"
xdotool mousemove "$((X + WIDTH / 2))" "$((Y + HEIGHT / 2))"
xdotool click 4
for _ in {1..120}; do
    steps="$(awk -F '\t' '$1 == "step" { count++ } END { print count + 0 }' "$scroll_trace" 2>/dev/null || echo 0)"
    if [ "$steps" -ge 3 ] && grep -R '"display_offset": [1-9]' "$snapshot_dir"/*.json >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.05
done
echo "smooth scroll did not drain through visible one-line steps" >&2
cat "$scroll_trace" >&2 || true
grep -R '"display_offset"' "$snapshot_dir"/*.json >&2 || true
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-smooth-scroll-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            scroll_trace.to_str().expect("scroll trace path utf8"),
        ])
        .output()
        .expect("run gtk smooth scroll e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk smooth scroll e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let trace = read_to_string(&scroll_trace).expect("read smooth scroll trace");
    assert!(trace.lines().any(|line| line == "enqueue\t3\t3"));
    assert!(
        trace
            .lines()
            .filter(|line| line.starts_with("step\t1\t"))
            .count()
            >= 3,
        "{trace}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_renders_narrow_cursor_pixels_under_xvfb() {
    if !has_command("xvfb-run")
        || !has_command("xdotool")
        || !has_command("import")
        || !has_command("convert")
    {
        eprintln!(
            "skipping gtk cursor pixel e2e because xvfb-run, xdotool, import, or convert is not installed"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-cursor-pixel-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let screenshot = dir.join("window.png");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
screenshot="$3"
geometry_trace="$4"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "abc def"
for _ in {1..100}; do
    if grep -R 'abc def' "$snapshot_dir" >/dev/null 2>&1 && [ -f "$geometry_trace" ]; then
        break
    fi
    sleep 0.1
done
if ! grep -R 'abc def' "$snapshot_dir" >/dev/null 2>&1; then
    echo "cursor position snapshot never contained typed text" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
xdotool type --window "$window_id" --delay 2 "Z"
sleep 0.15
import -window "$window_id" "$screenshot"
for _ in {1..100}; do
    if grep -R 'abc defZ' "$snapshot_dir" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if ! grep -R 'abc defZ' "$snapshot_dir" >/dev/null 2>&1; then
    echo "cursor reset snapshot never contained final typed text" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-cursor-pixel-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            screenshot.to_str().expect("screenshot path utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk cursor pixel e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk cursor pixel e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let bounds = pixel_bounds(&screenshot, |pixel| {
        pixel.red == 125 && pixel.green == 211 && pixel.blue == 252
    })
    .expect("cursor-colored pixels in screenshot");
    assert!(
        bounds.width() <= 2,
        "cursor should be a narrow vertical caret, got {bounds:?}"
    );
    assert!(
        bounds.height() >= 16,
        "cursor should be visibly tall, got {bounds:?}"
    );
    assert!(
        bounds.count >= 16,
        "cursor should have enough visible pixels, got {bounds:?}"
    );
    let cursor_snapshot = json_snapshots(&dir)
        .into_iter()
        .find(|snapshot| {
            snapshot["text"]
                .as_str()
                .is_some_and(|text| text.contains("abc defZ"))
        })
        .expect("cursor position snapshot");
    let cursor_column = cursor_snapshot["cursor_col"]
        .as_f64()
        .expect("numeric cursor column");
    let cursor_line = cursor_snapshot["cursor_line"]
        .as_f64()
        .expect("numeric cursor line");
    let expected_x = geometry_metric(&geometry_trace, "canvas_x")
        + cursor_column * geometry_metric(&geometry_trace, "cell_width");
    let expected_y = geometry_metric(&geometry_trace, "canvas_y")
        + cursor_line * geometry_metric(&geometry_trace, "line_height");
    assert!(
        (bounds.min_x as f64 - expected_x).abs() <= 6.0,
        "cursor x should match terminal cursor column: bounds={bounds:?} expected_x={expected_x:.2}"
    );
    assert!(
        (bounds.min_y as f64 - expected_y).abs() <= 6.0,
        "cursor y should match terminal cursor row: bounds={bounds:?} expected_y={expected_y:.2}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_blinks_cursor_and_resets_after_input_under_xvfb() {
    if !has_command("xvfb-run")
        || !has_command("xdotool")
        || !has_command("import")
        || !has_command("convert")
    {
        eprintln!(
            "skipping gtk cursor blink e2e because xvfb-run, xdotool, import, or convert is not installed"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-cursor-blink-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let visible = dir.join("visible.png");
    let hidden = dir.join("hidden.png");
    let reset = dir.join("reset.png");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
visible="$3"
hidden="$4"
reset="$5"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "blink"
sleep 0.15
import -window "$window_id" "$visible"
sleep 0.65
import -window "$window_id" "$hidden"
xdotool type --window "$window_id" --delay 2 "X"
sleep 0.15
import -window "$window_id" "$reset"
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-cursor-blink-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            visible.to_str().expect("visible screenshot path utf8"),
            hidden.to_str().expect("hidden screenshot path utf8"),
            reset.to_str().expect("reset screenshot path utf8"),
        ])
        .output()
        .expect("run gtk cursor blink e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk cursor blink e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let visible_count = cursor_pixel_count(&visible);
    let hidden_count = cursor_pixel_count(&hidden);
    let reset_count = cursor_pixel_count(&reset);
    assert!(
        visible_count >= 16,
        "cursor should start visible, got {visible_count} cursor pixels"
    );
    assert_eq!(
        hidden_count, 0,
        "cursor should blink off, got {hidden_count} cursor pixels"
    );
    assert!(
        reset_count >= 16,
        "cursor should reset visible after input, got {reset_count} cursor pixels"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_accepts_real_keyboard_input_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk keyboard e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-keyboard-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 'echo XDO_E2E_OK'
xdotool key --window "$window_id" Return
for _ in {1..100}; do
    if grep -R 'XDO_E2E_OK' "$snapshot_dir" >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.1
done
echo "snapshot never contained XDO_E2E_OK" >&2
find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-keyboard-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
        ])
        .output()
        .expect("run gtk keyboard e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk keyboard e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let text = snapshot_paths(&dir)
        .into_iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "txt"))
        .map(|path| read_to_string(path).expect("read text snapshot"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("XDO_E2E_OK"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_commits_composed_input_text_through_im_context_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk IM compose e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-im-compose-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2

wait_latest_text() {
    local text="$1"
    for _ in {1..100}; do
        latest_txt="$(ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1 || true)"
        if [ -n "$latest_txt" ] && grep -F "$text" "$latest_txt" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.1
    done
    echo "latest text did not become: $text" >&2
    latest_txt="$(ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1 || true)"
    [ -n "$latest_txt" ] && cat "$latest_txt" >&2
    return 1
}

xdotool type --window "$window_id" --delay 2 "abc"
wait_latest_text '❯ abc'
xdotool key --window "$window_id" ctrl+a
sleep 0.1
xdotool key --window "$window_id" Multi_key apostrophe e
wait_latest_text '❯ é'
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-im-compose-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
        ])
        .output()
        .expect("run gtk IM compose e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk IM compose e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_renders_im_preedit_before_commit_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk IM preedit e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-im-preedit-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_RENDER_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2

latest_render() {
    ls -t "$snapshot_dir"/*.render.json 2>/dev/null | head -n 1
}
latest_txt() {
    ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1
}
wait_preedit() {
    for _ in {1..100}; do
        latest="$(latest_render || true)"
        if [ -n "$latest" ] && grep -F '"preedit": {' "$latest" >/dev/null 2>&1 && grep -F '"input_text": "❯"' "$latest" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.1
    done
    echo "preedit render state never appeared" >&2
    latest="$(latest_render || true)"
    [ -n "$latest" ] && cat "$latest" >&2
    return 1
}
wait_latest_text() {
    local text="$1"
    for _ in {1..100}; do
        latest="$(latest_txt || true)"
        if [ -n "$latest" ] && grep -F "$text" "$latest" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.1
    done
    echo "latest text did not become: $text" >&2
    latest="$(latest_txt || true)"
    [ -n "$latest" ] && cat "$latest" >&2
    return 1
}
xdotool key --window "$window_id" Multi_key
sleep 0.15
xdotool key --window "$window_id" apostrophe
wait_preedit
xdotool key --window "$window_id" e
wait_latest_text '❯ é'
for _ in {1..100}; do
    latest="$(latest_render || true)"
    if [ -n "$latest" ] && grep -F '"preedit": null' "$latest" >/dev/null 2>&1 && grep -F '"input_text": "❯ é"' "$latest" >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.1
done
echo "preedit did not clear after composed commit" >&2
latest="$(latest_render || true)"
[ -n "$latest" ] && cat "$latest" >&2
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-im-preedit-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
        ])
        .output()
        .expect("run gtk IM preedit e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk IM preedit e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let render = snapshot_paths(&dir)
        .into_iter()
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".render.json"))
        })
        .map(|path| read_to_string(path).expect("read render snapshot"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(render.contains("\"preedit\": {"));
    assert!(render.contains("\"columns\":"));
    assert!(render.contains("\"cursor_columns\":"));
    assert!(render.contains("\"preedit\": null"));
    assert!(render.contains("\"input_text\": \"❯ é\""));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_tracks_held_key_render_and_paint_latency_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk held-key perf e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-held-key-perf-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    let zdot = dir.join("zdot");
    let snapshots = dir.join("snapshots");
    std::fs::create_dir_all(&zdot).expect("zdot dir");
    std::fs::create_dir_all(&snapshots).expect("snapshot dir");
    std::fs::write(
        zdot.join(".zshrc"),
        "PS1=\"❯ \"\nHISTFILE=/dev/null\nSAVEHIST=0\n",
    )
    .expect("zshrc fixture");
    let perf_trace = dir.join("perf.tsv");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
perf_trace="$3"
zdot="$4"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_PERF_TRACE="$perf_trace" CHELOTYPE_ALLOC_TRACE=1 CHELOTYPE_SHELL=/usr/bin/zsh ZDOTDIR="$zdot" HOME="$zdot" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.35
xdotool keydown --window "$window_id" a
sleep 10
xdotool keyup --window "$window_id" a
sleep 0.5
if [ ! -s "$perf_trace" ]; then
    echo "held-key perf trace was not written" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
if ! grep -R 'aaaaaaaaaaaaaaaaaaaaaaaa' "$snapshot_dir" >/dev/null 2>&1; then
    echo "held-key snapshots did not show sustained typed input" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-held-key-perf-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            snapshots.to_str().expect("snapshot dir utf8"),
            perf_trace.to_str().expect("perf trace path utf8"),
            zdot.to_str().expect("zdot path utf8"),
        ])
        .output()
        .expect("run gtk held-key perf e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk held-key perf e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let render = perf_samples(&perf_trace, "gtk_render");
    let paint = perf_samples(&perf_trace, "gtk_paint");
    let input_to_render = perf_samples(&perf_trace, "input_to_render");
    let input_allocs = perf_counters(&perf_trace, "input_allocs_to_render");
    let input_alloc_bytes = perf_counters(&perf_trace, "input_alloc_bytes_to_render");
    let render_allocs = perf_counters(&perf_trace, "gtk_render_allocs");
    let render_alloc_bytes = perf_counters(&perf_trace, "gtk_render_alloc_bytes");
    let rss_kib = perf_counters(&perf_trace, "process_rss_kib");
    assert!(
        render.len() >= 80,
        "held-key produced too few render samples: {}",
        render.len()
    );
    assert!(
        paint.len() >= 80,
        "held-key produced too few paint samples: {}",
        paint.len()
    );
    assert!(
        input_to_render.len() >= 80,
        "held-key produced too few input-to-render samples: {}",
        input_to_render.len()
    );
    assert!(
        input_allocs.len() >= 80,
        "held-key produced too few input allocation samples: {}",
        input_allocs.len()
    );
    assert!(
        input_alloc_bytes.len() >= 80,
        "held-key produced too few input allocated-byte samples: {}",
        input_alloc_bytes.len()
    );
    assert!(
        render_allocs.len() >= 80,
        "held-key produced too few allocation samples: {}",
        render_allocs.len()
    );
    assert!(
        rss_kib.len() >= 80,
        "held-key produced too few process RSS samples: {}",
        rss_kib.len()
    );
    let render_p95 = percentile_duration(render.clone(), 95);
    let render_p99 = percentile_duration(render, 99);
    let paint_p95 = percentile_duration(paint.clone(), 95);
    let paint_p99 = percentile_duration(paint, 99);
    let input_to_render_p95 = percentile_duration(input_to_render.clone(), 95);
    let input_to_render_p99 = percentile_duration(input_to_render, 99);
    let input_allocs_p95 = percentile_counter(input_allocs.clone(), 95);
    let input_allocs_p99 = percentile_counter(input_allocs, 99);
    let input_alloc_bytes_p95 = percentile_counter(input_alloc_bytes.clone(), 95);
    let input_alloc_bytes_p99 = percentile_counter(input_alloc_bytes, 99);
    let render_allocs_p95 = percentile_counter(render_allocs.clone(), 95);
    let render_allocs_p99 = percentile_counter(render_allocs, 99);
    let render_alloc_bytes_p95 = percentile_counter(render_alloc_bytes.clone(), 95);
    let render_alloc_bytes_p99 = percentile_counter(render_alloc_bytes, 99);
    let rss_min = *rss_kib.iter().min().expect("rss samples");
    let rss_max = *rss_kib.iter().max().expect("rss samples");
    let rss_growth = rss_max.saturating_sub(rss_min);
    assert!(
        render_p95 <= Duration::from_millis(8),
        "held-key gtk_render p95 exceeded 120 Hz budget: {render_p95:?}"
    );
    assert!(
        render_p99 <= Duration::from_millis(16),
        "held-key gtk_render p99 exceeded 60 Hz budget: {render_p99:?}"
    );
    assert!(
        paint_p95 <= Duration::from_millis(16),
        "held-key gtk_paint p95 exceeded 60 Hz budget: {paint_p95:?}"
    );
    assert!(
        paint_p99 <= Duration::from_millis(33),
        "held-key gtk_paint p99 exceeded two-frame budget: {paint_p99:?}"
    );
    assert!(
        input_to_render_p95 <= Duration::from_millis(16),
        "held-key input_to_render p95 exceeded 60 Hz budget: {input_to_render_p95:?}"
    );
    assert!(
        input_to_render_p99 <= Duration::from_millis(33),
        "held-key input_to_render p99 exceeded two-frame budget: {input_to_render_p99:?}"
    );
    assert!(
        input_allocs_p95 <= 18_000,
        "held-key input allocation p95 too high: {input_allocs_p95}"
    );
    assert!(
        input_allocs_p99 <= 36_000,
        "held-key input allocation p99 too high: {input_allocs_p99}"
    );
    assert!(
        input_alloc_bytes_p95 <= 3_000_000,
        "held-key input allocated-byte p95 too high: {input_alloc_bytes_p95}"
    );
    assert!(
        input_alloc_bytes_p99 <= 6_000_000,
        "held-key input allocated-byte p99 too high: {input_alloc_bytes_p99}"
    );
    assert!(
        render_allocs_p95 <= 15_000,
        "held-key gtk_render allocation p95 too high: {render_allocs_p95}"
    );
    assert!(
        render_allocs_p99 <= 30_000,
        "held-key gtk_render allocation p99 too high: {render_allocs_p99}"
    );
    assert!(
        render_alloc_bytes_p95 <= 2_500_000,
        "held-key gtk_render allocated-byte p95 too high: {render_alloc_bytes_p95}"
    );
    assert!(
        render_alloc_bytes_p99 <= 5_000_000,
        "held-key gtk_render allocated-byte p99 too high: {render_alloc_bytes_p99}"
    );
    assert!(rss_min > 0, "held-key process RSS samples must be non-zero");
    assert!(
        rss_growth <= 96 * 1024,
        "held-key process RSS growth too high: min={rss_min} KiB max={rss_max} KiB growth={rss_growth} KiB"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_creates_and_switches_terminal_tabs_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk tab e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-tabs-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
latest_txt() {
    ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1
}
wait_latest_contains_only() {
    needle="$1"
    forbidden="$2"
    for _ in {1..120}; do
        latest="$(latest_txt || true)"
        if [ -n "$latest" ] && grep -F "$needle" "$latest" >/dev/null 2>&1 && ! grep -F "$forbidden" "$latest" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.1
    done
    echo "latest snapshot did not contain only $needle" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    latest="$(latest_txt || true)"
    [ -n "$latest" ] && sed -n '1,10p' "$latest" >&2
    return 1
}
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "printf 'TAB_ONE_ACTIVE\n'"
xdotool key --window "$window_id" Return
for _ in {1..100}; do
    if grep -R 'TAB_ONE_ACTIVE' "$snapshot_dir" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if ! grep -R 'TAB_ONE_ACTIVE' "$snapshot_dir" >/dev/null 2>&1; then
    echo "first tab content never appeared" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
xdotool key --window "$window_id" ctrl+shift+t
sleep 0.4
xdotool type --window "$window_id" --delay 2 "printf 'TAB_TWO_ACTIVE\n'"
xdotool key --window "$window_id" Return
wait_latest_contains_only 'TAB_TWO_ACTIVE' 'TAB_ONE_ACTIVE'
xdotool key --window "$window_id" ctrl+Page_Up
wait_latest_contains_only 'TAB_ONE_ACTIVE' 'TAB_TWO_ACTIVE'
xdotool key --window "$window_id" ctrl+Page_Down
wait_latest_contains_only 'TAB_TWO_ACTIVE' 'TAB_ONE_ACTIVE'
xdotool key --window "$window_id" ctrl+shift+w
wait_latest_contains_only 'TAB_ONE_ACTIVE' 'TAB_TWO_ACTIVE'
xdotool type --window "$window_id" --delay 2 "printf 'TAB_ONE_AFTER_CLOSE\n'"
xdotool key --window "$window_id" Return
wait_latest_contains_only 'TAB_ONE_AFTER_CLOSE' 'TAB_TWO_ACTIVE'
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-tabs-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
        ])
        .output()
        .expect("run gtk tab e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk tab e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let text = snapshot_paths(&dir)
        .into_iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "txt"))
        .map(|path| read_to_string(path).expect("read text snapshot"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("TAB_ONE_ACTIVE"));
    assert!(text.contains("TAB_TWO_ACTIVE"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_starts_first_tab_in_available_toolbox_container_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!(
            "skipping gtk container startup e2e because xvfb-run or xdotool is not installed"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-container-startup-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("container startup dir");
    let fake_bin = dir.join("bin");
    std::fs::create_dir_all(&fake_bin).expect("fake bin dir");
    let toolbox_log = dir.join("toolbox.tsv");
    write_fake_toolbox(&fake_bin, &toolbox_log, "fedora-toolbox-latest");

    let snapshot_dir = dir.join("snapshots");
    std::fs::create_dir_all(&snapshot_dir).expect("snapshot dir");
    let config_dir = dir.join("config");
    std::fs::create_dir_all(&config_dir).expect("config dir");
    let tab_trace = dir.join("tabs.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
tab_trace="$3"
config_dir="$4"
fake_bin="$5"
toolbox_log="$6"
PATH="$fake_bin:$PATH" GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_CONFIG_DIR="$config_dir" CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_TAB_TRACE="$tab_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..100}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ] && [ -f "$tab_trace" ] && grep -F 'tab_0_launch_target=toolbox:fedora-toolbox-latest' "$tab_trace" >/dev/null 2>&1 && grep -F 'enter	fedora-toolbox-latest' "$toolbox_log" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
if ! grep -F 'tab_0_title=fedora-toolbox-latest' "$tab_trace" >/dev/null 2>&1; then
    echo "initial tab title did not use toolbox container" >&2
    cat "$tab_trace" >&2 || true
    cat "$toolbox_log" >&2 || true
    exit 1
fi
if ! grep -F 'tab_0_launch_target=toolbox:fedora-toolbox-latest' "$tab_trace" >/dev/null 2>&1; then
    echo "initial tab launch target did not use toolbox container" >&2
    cat "$tab_trace" >&2 || true
    cat "$toolbox_log" >&2 || true
    exit 1
fi
if ! grep -F 'enter	fedora-toolbox-latest' "$toolbox_log" >/dev/null 2>&1; then
    echo "fake toolbox was not entered" >&2
    cat "$toolbox_log" >&2 || true
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "printf 'TOOLBOX_STARTUP_OK\n'"
xdotool key --window "$window_id" Return
for _ in {1..100}; do
    if grep -R '^TOOLBOX_STARTUP_OK' "$snapshot_dir"/*.txt >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.1
done
echo "startup toolbox tab did not accept shell input" >&2
cat "$tab_trace" >&2 || true
cat "$toolbox_log" >&2 || true
find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-container-startup-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            snapshot_dir.to_str().expect("snapshot dir utf8"),
            tab_trace.to_str().expect("tab trace path utf8"),
            config_dir.to_str().expect("config dir utf8"),
            fake_bin.to_str().expect("fake bin dir utf8"),
            toolbox_log.to_str().expect("toolbox log path utf8"),
        ])
        .output()
        .expect("run gtk container startup e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk container startup e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let trace = read_to_string(&tab_trace).expect("read tab trace");
    assert!(trace.contains("tab_0_title=fedora-toolbox-latest"));
    assert!(trace.contains("tab_0_launch_target=toolbox:fedora-toolbox-latest"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_opens_toolbox_container_from_launch_menu_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!(
            "skipping gtk container launch menu e2e because xvfb-run or xdotool is not installed"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-container-menu-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("container menu dir");
    let fake_bin = dir.join("bin");
    std::fs::create_dir_all(&fake_bin).expect("fake bin dir");
    let toolbox_log = dir.join("toolbox.tsv");
    write_fake_toolbox(&fake_bin, &toolbox_log, "fedora-toolbox-latest");

    let snapshot_dir = dir.join("snapshots");
    std::fs::create_dir_all(&snapshot_dir).expect("snapshot dir");
    let config_dir = dir.join("config");
    std::fs::create_dir_all(&config_dir).expect("config dir");
    std::fs::write(config_dir.join("config"), "startup_launch_target=host\n").expect("config");
    let tab_trace = dir.join("tabs.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
tab_trace="$3"
config_dir="$4"
fake_bin="$5"
toolbox_log="$6"
PATH="$fake_bin:$PATH" GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_CONFIG_DIR="$config_dir" CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_TAB_TRACE="$tab_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..100}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ] && [ -f "$tab_trace" ] && grep -F 'tab_0_launch_target=host' "$tab_trace" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
trace_value() {
    local key="$1"
    sed -n "s/^${key}=\\([0-9][0-9]*\\)$/\\1/p" "$tab_trace"
}
wait_container_tab() {
    for _ in {1..100}; do
        if grep -F 'tab_count=2' "$tab_trace" >/dev/null 2>&1 && grep -F 'selected_index=1' "$tab_trace" >/dev/null 2>&1 && grep -F 'tab_1_launch_target=toolbox:fedora-toolbox-latest' "$tab_trace" >/dev/null 2>&1 && grep -F 'enter	fedora-toolbox-latest' "$toolbox_log" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.1
    done
    return 1
}
xdotool windowfocus "$window_id" || true
sleep 0.2
eval "$(xdotool getwindowgeometry --shell "$window_id")"
launch_x="$(trace_value launch_menu_x)"
launch_y="$(trace_value launch_menu_y)"
launch_width="$(trace_value launch_menu_width)"
launch_height="$(trace_value launch_menu_height)"
button_x="$(awk -v left="$X" -v x="$launch_x" -v width="$launch_width" 'BEGIN { printf "%d", left + x + (width / 2) }')"
button_y="$(awk -v top="$Y" -v y="$launch_y" -v height="$launch_height" 'BEGIN { printf "%d", top + y + (height / 2) }')"
xdotool mousemove "$button_x" "$button_y"
xdotool click 1
sleep 0.3
command -v import >/dev/null 2>&1 && import -window root "$snapshot_dir/../menu-open.png" || true
search_x="$(awk -v left="$X" -v x="$launch_x" 'BEGIN { printf "%d", left + x + 24 }')"
search_y="$(awk -v top="$Y" -v y="$launch_y" -v height="$launch_height" 'BEGIN { printf "%d", top + y + height + 32 }')"
xdotool mousemove "$search_x" "$search_y"
xdotool click 1
xdotool type --delay 2 "fedora-toolbox-latest"
sleep 0.2
command -v import >/dev/null 2>&1 && import -window root "$snapshot_dir/../menu-filter.png" || true
row_x="$(awk -v x="$search_x" 'BEGIN { printf "%d", x + 20 }')"
row_y="$(awk -v y="$search_y" 'BEGIN { printf "%d", y + 96 }')"
xdotool mousemove "$row_x" "$row_y"
xdotool click 1
xdotool key Return
sleep 0.2
command -v import >/dev/null 2>&1 && import -window root "$snapshot_dir/../menu-after-row-click.png" || true
if ! wait_container_tab; then
    echo "launch menu did not open toolbox tab" >&2
    cat "$tab_trace" >&2 || true
    cat "$toolbox_log" >&2 || true
    exit 1
fi
xdotool type --window "$window_id" --delay 2 "printf 'TOOLBOX_MENU_OK\n'"
xdotool key --window "$window_id" Return
for _ in {1..100}; do
    latest="$(ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1 || true)"
    if [ -n "$latest" ] && grep -F 'TOOLBOX_MENU_OK' "$latest" >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.1
done
echo "toolbox menu tab did not accept input" >&2
cat "$tab_trace" >&2 || true
cat "$toolbox_log" >&2 || true
find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-container-menu-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            snapshot_dir.to_str().expect("snapshot dir utf8"),
            tab_trace.to_str().expect("tab trace path utf8"),
            config_dir.to_str().expect("config dir utf8"),
            fake_bin.to_str().expect("fake bin dir utf8"),
            toolbox_log.to_str().expect("toolbox log path utf8"),
        ])
        .output()
        .expect("run gtk container menu e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk container menu e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let trace = read_to_string(&tab_trace).expect("read tab trace");
    assert!(trace.contains("tab_count=2"));
    assert!(trace.contains("tab_1_launch_target=toolbox:fedora-toolbox-latest"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_remembers_single_toolbox_tab_after_window_close_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!(
            "skipping gtk container persistence e2e because xvfb-run or xdotool is not installed"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-container-persistence-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("container persistence dir");
    let fake_bin = dir.join("bin");
    std::fs::create_dir_all(&fake_bin).expect("fake bin dir");
    let toolbox_log = dir.join("toolbox.tsv");
    write_fake_toolbox(&fake_bin, &toolbox_log, "fedora-toolbox-latest");

    let first_snapshot_dir = dir.join("first-snapshots");
    let second_snapshot_dir = dir.join("second-snapshots");
    std::fs::create_dir_all(&first_snapshot_dir).expect("first snapshot dir");
    std::fs::create_dir_all(&second_snapshot_dir).expect("second snapshot dir");
    let config_dir = dir.join("config");
    std::fs::create_dir_all(&config_dir).expect("config dir");
    std::fs::write(config_dir.join("config"), "startup_launch_target=host\n").expect("config");
    let first_tab_trace = dir.join("first-tabs.env");
    let second_tab_trace = dir.join("second-tabs.env");

    let script = r#"
set -euo pipefail
bin="$1"
first_snapshot_dir="$2"
second_snapshot_dir="$3"
first_tab_trace="$4"
second_tab_trace="$5"
config_dir="$6"
fake_bin="$7"
toolbox_log="$8"

start_app() {
    local snapshot_dir="$1"
    local tab_trace="$2"
    PATH="$fake_bin:$PATH" GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_CONFIG_DIR="$config_dir" CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_TAB_TRACE="$tab_trace" "$bin" &
    app_pid="$!"
}

wait_window() {
    local tab_trace="$1"
    window_id=""
    for _ in {1..100}; do
        window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
        if [ -n "$window_id" ] && [ -f "$tab_trace" ]; then
            return 0
        fi
        sleep 0.1
    done
    echo "chelotype window did not appear" >&2
    return 1
}

trace_value() {
    local tab_trace="$1"
    local key="$2"
    sed -n "s/^${key}=\\([0-9][0-9]*\\)$/\\1/p" "$tab_trace"
}

wait_trace_contains() {
    local tab_trace="$1"
    shift
    for _ in {1..120}; do
        local ok=1
        for expected in "$@"; do
            if ! grep -F "$expected" "$tab_trace" >/dev/null 2>&1; then
                ok=0
                break
            fi
        done
        if [ "$ok" = 1 ]; then
            return 0
        fi
        sleep 0.1
    done
    echo "tab trace did not reach expected state" >&2
    cat "$tab_trace" >&2 || true
    return 1
}

open_toolbox_from_launcher() {
    local tab_trace="$1"
    eval "$(xdotool getwindowgeometry --shell "$window_id")"
    launch_x="$(trace_value "$tab_trace" launch_menu_x)"
    launch_y="$(trace_value "$tab_trace" launch_menu_y)"
    launch_width="$(trace_value "$tab_trace" launch_menu_width)"
    launch_height="$(trace_value "$tab_trace" launch_menu_height)"
    button_x="$(awk -v left="$X" -v x="$launch_x" -v width="$launch_width" 'BEGIN { printf "%d", left + x + (width / 2) }')"
    button_y="$(awk -v top="$Y" -v y="$launch_y" -v height="$launch_height" 'BEGIN { printf "%d", top + y + (height / 2) }')"
    xdotool mousemove "$button_x" "$button_y"
    xdotool click 1
    sleep 0.3
    search_x="$(awk -v left="$X" -v x="$launch_x" 'BEGIN { printf "%d", left + x + 24 }')"
    search_y="$(awk -v top="$Y" -v y="$launch_y" -v height="$launch_height" 'BEGIN { printf "%d", top + y + height + 32 }')"
    xdotool mousemove "$search_x" "$search_y"
    xdotool click 1
    xdotool type --delay 2 "fedora-toolbox-latest"
    sleep 0.2
    row_x="$(awk -v x="$search_x" 'BEGIN { printf "%d", x + 20 }')"
    row_y="$(awk -v y="$search_y" 'BEGIN { printf "%d", y + 96 }')"
    xdotool mousemove "$row_x" "$row_y"
    xdotool click 1
    xdotool key Return
}

start_app "$first_snapshot_dir" "$first_tab_trace"
trap 'kill "$app_pid" 2>/dev/null || true' EXIT
wait_window "$first_tab_trace"
xdotool windowfocus "$window_id" || true
sleep 0.2
wait_trace_contains "$first_tab_trace" 'tab_count=1' 'tab_0_launch_target=host'
open_toolbox_from_launcher "$first_tab_trace"
wait_trace_contains "$first_tab_trace" 'tab_count=2' 'selected_index=1' 'tab_1_launch_target=toolbox:fedora-toolbox-latest'
grep -F 'enter	fedora-toolbox-latest' "$toolbox_log" >/dev/null 2>&1

xdotool key --window "$window_id" ctrl+Page_Up
wait_trace_contains "$first_tab_trace" 'tab_count=2' 'selected_index=0'
xdotool key --window "$window_id" ctrl+shift+w
wait_trace_contains "$first_tab_trace" 'tab_count=1' 'selected_index=0' 'tab_0_launch_target=toolbox:fedora-toolbox-latest'

xdotool windowclose "$window_id"
for _ in {1..100}; do
    if ! kill -0 "$app_pid" 2>/dev/null; then
        break
    fi
    sleep 0.1
done
wait "$app_pid" 2>/dev/null || true
if kill -0 "$app_pid" 2>/dev/null; then
    echo "first app did not exit after window close" >&2
    exit 1
fi
if ! grep -F 'startup_launch_target=toolbox:fedora-toolbox-latest' "$config_dir/config" >/dev/null 2>&1; then
    echo "single toolbox tab was not remembered on window close" >&2
    cat "$config_dir/config" >&2 || true
    exit 1
fi

start_app "$second_snapshot_dir" "$second_tab_trace"
trap 'kill "$app_pid" 2>/dev/null || true' EXIT
wait_window "$second_tab_trace"
wait_trace_contains "$second_tab_trace" 'tab_count=1' 'selected_index=0' 'tab_0_launch_target=toolbox:fedora-toolbox-latest'
xdotool windowclose "$window_id"
wait "$app_pid" 2>/dev/null || true
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-container-persistence-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            first_snapshot_dir
                .to_str()
                .expect("first snapshot dir utf8"),
            second_snapshot_dir
                .to_str()
                .expect("second snapshot dir utf8"),
            first_tab_trace.to_str().expect("first tab trace utf8"),
            second_tab_trace.to_str().expect("second tab trace utf8"),
            config_dir.to_str().expect("config dir utf8"),
            fake_bin.to_str().expect("fake bin dir utf8"),
            toolbox_log.to_str().expect("toolbox log path utf8"),
        ])
        .output()
        .expect("run gtk container persistence e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk container persistence e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let config = read_to_string(config_dir.join("config")).expect("read config");
    assert!(config.contains("startup_launch_target=toolbox:fedora-toolbox-latest"));
    let trace = read_to_string(&second_tab_trace).expect("read second tab trace");
    assert!(trace.contains("tab_0_launch_target=toolbox:fedora-toolbox-latest"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_closes_terminal_tab_with_middle_click_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!(
            "skipping gtk middle-click tab close e2e because xvfb-run or xdotool is not installed"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-middle-click-tabs-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let tab_trace = dir.join("tabs.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
tab_trace="$3"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_TAB_TRACE="$tab_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ] && [ -f "$tab_trace" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ] || [ ! -f "$tab_trace" ]; then
    echo "chelotype window or tab trace did not appear" >&2
    exit 1
fi
latest_txt() {
    ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1
}
wait_latest_contains_only() {
    needle="$1"
    forbidden="$2"
    for _ in {1..120}; do
        latest="$(latest_txt || true)"
        if [ -n "$latest" ] && grep -F "$needle" "$latest" >/dev/null 2>&1 && ! grep -F "$forbidden" "$latest" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.1
    done
    echo "latest snapshot did not contain only $needle" >&2
    latest="$(latest_txt || true)"
    [ -n "$latest" ] && sed -n '1,12p' "$latest" >&2
    cat "$tab_trace" >&2 || true
    return 1
}
wait_tab_trace() {
    expected_count="$1"
    expected_selected="$2"
    for _ in {1..100}; do
        count="$(sed -n 's/^tab_count=\([0-9][0-9]*\)$/\1/p' "$tab_trace")"
        selected="$(sed -n 's/^selected_index=\([0-9][0-9]*\)$/\1/p' "$tab_trace")"
        width="$(sed -n 's/^tab_bar_width=\([0-9][0-9]*\)$/\1/p' "$tab_trace")"
        height="$(sed -n 's/^tab_bar_height=\([0-9][0-9]*\)$/\1/p' "$tab_trace")"
        if [ "$count" = "$expected_count" ] && [ "$selected" = "$expected_selected" ] && [ "${width:-0}" -gt 20 ] && [ "${height:-0}" -gt 10 ]; then
            return 0
        fi
        sleep 0.05
    done
    echo "tab trace did not reach count=$expected_count selected=$expected_selected" >&2
    cat "$tab_trace" >&2 || true
    return 1
}
middle_click_tab() {
    index="$1"
    eval "$(xdotool getwindowgeometry --shell "$window_id")"
    tab_bar_x="$(sed -n 's/^tab_bar_x=\([0-9][0-9]*\)$/\1/p' "$tab_trace")"
    tab_bar_y="$(sed -n 's/^tab_bar_y=\([0-9][0-9]*\)$/\1/p' "$tab_trace")"
    tab_bar_width="$(sed -n 's/^tab_bar_width=\([0-9][0-9]*\)$/\1/p' "$tab_trace")"
    tab_bar_height="$(sed -n 's/^tab_bar_height=\([0-9][0-9]*\)$/\1/p' "$tab_trace")"
    tab_count="$(sed -n 's/^tab_count=\([0-9][0-9]*\)$/\1/p' "$tab_trace")"
    target_x="$(awk -v left="$X" -v bar_x="$tab_bar_x" -v width="$tab_bar_width" -v count="$tab_count" -v tab_index="$index" 'BEGIN { printf "%d", left + bar_x + (((tab_index + 0.5) * width) / count) }')"
    target_y="$(awk -v top="$Y" -v bar_y="$tab_bar_y" -v height="$tab_bar_height" 'BEGIN { printf "%d", top + bar_y + (height / 2) }')"
    xdotool mousemove "$target_x" "$target_y"
    xdotool click 2
}
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "printf 'MIDDLE_TAB_ONE\n'"
xdotool key --window "$window_id" Return
for _ in {1..100}; do
    if grep -R 'MIDDLE_TAB_ONE' "$snapshot_dir" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
xdotool key --window "$window_id" ctrl+shift+t
wait_tab_trace 2 1
xdotool type --window "$window_id" --delay 2 "printf 'MIDDLE_TAB_TWO\n'"
xdotool key --window "$window_id" Return
wait_latest_contains_only 'MIDDLE_TAB_TWO' 'MIDDLE_TAB_ONE'
middle_click_tab 1
wait_tab_trace 1 0
wait_latest_contains_only 'MIDDLE_TAB_ONE' 'MIDDLE_TAB_TWO'
xdotool type --window "$window_id" --delay 2 "printf 'MIDDLE_TAB_ONE_AFTER_CLOSE\n'"
xdotool key --window "$window_id" Return
wait_latest_contains_only 'MIDDLE_TAB_ONE_AFTER_CLOSE' 'MIDDLE_TAB_TWO'
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-middle-click-tabs-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            tab_trace.to_str().expect("tab trace path utf8"),
        ])
        .output()
        .expect("run gtk middle-click tab close e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk middle-click tab close e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_keeps_live_input_isolated_between_tabs_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk live tab input e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-live-tabs-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
latest_txt() {
    ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1
}
wait_latest_contains_only() {
    needle="$1"
    forbidden="$2"
    for _ in {1..120}; do
        latest="$(latest_txt || true)"
        if [ -n "$latest" ] && grep -F "$needle" "$latest" >/dev/null 2>&1 && ! grep -F "$forbidden" "$latest" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.1
    done
    echo "latest snapshot did not isolate live tab input: wanted=$needle forbidden=$forbidden" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    latest="$(latest_txt || true)"
    [ -n "$latest" ] && sed -n '1,12p' "$latest" >&2
    return 1
}
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "tab_one_live_input"
wait_latest_contains_only '❯ tab_one_live_input' 'tab_two_live_input'
xdotool key --window "$window_id" ctrl+shift+t
sleep 0.4
xdotool type --window "$window_id" --delay 2 "tab_two_live_input"
wait_latest_contains_only '❯ tab_two_live_input' 'tab_one_live_input'
xdotool key --window "$window_id" ctrl+Page_Up
wait_latest_contains_only '❯ tab_one_live_input' 'tab_two_live_input'
xdotool key --window "$window_id" ctrl+Page_Down
wait_latest_contains_only '❯ tab_two_live_input' 'tab_one_live_input'
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-live-tabs-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
        ])
        .output()
        .expect("run gtk live tab input e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk live tab input e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let text = snapshot_paths(&dir)
        .into_iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "txt"))
        .map(|path| read_to_string(path).expect("read text snapshot"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("tab_one_live_input"));
    assert!(text.contains("tab_two_live_input"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_renders_real_split_panes_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk split e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-split-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
wait_contains() {
    needle="$1"
    for _ in {1..140}; do
        if grep -R "$needle" "$snapshot_dir" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.1
    done
    echo "snapshot never contained $needle" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    return 1
}
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "printf 'GTK_SPLIT_LEFT\n'"
xdotool key --window "$window_id" Return
wait_contains 'GTK_SPLIT_LEFT'
xdotool key --window "$window_id" ctrl+shift+e
sleep 0.5
xdotool type --window "$window_id" --delay 2 "printf 'GTK_SPLIT_RIGHT\n'"
xdotool key --window "$window_id" Return
for _ in {1..160}; do
    latest="$(ls -t "$snapshot_dir"/*.workspace.render.json 2>/dev/null | head -n 1 || true)"
    if [ -n "$latest" ] && grep -F 'GTK_SPLIT_LEFT' "$latest" >/dev/null 2>&1 && grep -F 'GTK_SPLIT_RIGHT' "$latest" >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.1
done
echo "workspace split render never contained both panes" >&2
find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
latest="$(ls -t "$snapshot_dir"/*.workspace.render.json 2>/dev/null | head -n 1 || true)"
[ -n "$latest" ] && sed -n '1,80p' "$latest" >&2
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-split-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
        ])
        .output()
        .expect("run gtk split e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk split e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let workspace_json = snapshot_paths(&dir)
        .into_iter()
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".workspace.render.json"))
        })
        .map(|path| read_to_string(path).expect("read workspace render json"))
        .find(|json| json.contains("GTK_SPLIT_LEFT") && json.contains("GTK_SPLIT_RIGHT"))
        .expect("workspace render json with both split panes");
    let dump = serde_json::from_str::<serde_json::Value>(&workspace_json)
        .expect("valid workspace render json");
    let panes = dump["panes"].as_array().expect("panes array");
    assert_eq!(panes.len(), 2, "{workspace_json}");
    assert!(
        panes[0]["frame"]["lines"]
            .to_string()
            .contains("GTK_SPLIT_LEFT")
    );
    assert!(
        panes[1]["frame"]["lines"]
            .to_string()
            .contains("GTK_SPLIT_RIGHT")
    );
    assert!(panes[0]["cols"].as_u64().expect("left cols") > 0);
    assert!(panes[1]["cols"].as_u64().expect("right cols") > 0);
    assert_ne!(panes[0]["origin_col"], panes[1]["origin_col"]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_click_activates_split_pane_before_typing_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk split click e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-split-click-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
geometry_trace="$3"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
wait_contains() {
    needle="$1"
    for _ in {1..140}; do
        if grep -R "$needle" "$snapshot_dir" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.1
    done
    echo "snapshot never contained $needle" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    return 1
}
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "printf 'GTK_SPLIT_CLICK_LEFT_BEFORE\n'"
xdotool key --window "$window_id" Return
wait_contains 'GTK_SPLIT_CLICK_LEFT_BEFORE'
xdotool key --window "$window_id" ctrl+shift+e
sleep 0.5
xdotool type --window "$window_id" --delay 2 "printf 'GTK_SPLIT_CLICK_RIGHT_BEFORE\n'"
xdotool key --window "$window_id" Return
wait_contains 'GTK_SPLIT_CLICK_RIGHT_BEFORE'
for _ in {1..80}; do
    if [ -f "$geometry_trace" ]; then
        break
    fi
    sleep 0.1
done
if [ ! -f "$geometry_trace" ]; then
    echo "geometry trace did not appear" >&2
    exit 1
fi
canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
eval "$(xdotool getwindowgeometry --shell "$window_id")"
left_pane_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (2.5 * cell) }')"
left_pane_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + (4.5 * line) }')"
echo "split click target window=$window_id X=$X Y=$Y canvas=$canvas_x,$canvas_y cell=$cell_width line=$line_height target=$left_pane_x,$left_pane_y" >&2
xdotool mousemove "$left_pane_x" "$left_pane_y"
xdotool click 1
sleep 0.2
xdotool type --window "$window_id" --delay 2 "printf 'GTK_SPLIT_LEFT_AFTER_CLICK\n'"
xdotool key --window "$window_id" Return
for _ in {1..160}; do
    latest="$(ls -t "$snapshot_dir"/*.workspace.render.json 2>/dev/null | head -n 1 || true)"
    if [ -n "$latest" ] && grep -F 'GTK_SPLIT_LEFT_AFTER_CLICK' "$latest" >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.1
done
echo "workspace split render never contained clicked-left marker" >&2
find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
latest="$(ls -t "$snapshot_dir"/*.workspace.render.json 2>/dev/null | head -n 1 || true)"
[ -n "$latest" ] && sed -n '1,120p' "$latest" >&2
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-split-click-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk split click e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk split click e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let workspace_json = snapshot_paths(&dir)
        .into_iter()
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".workspace.render.json"))
        })
        .map(|path| read_to_string(path).expect("read workspace render json"))
        .rev()
        .find(|json| json.contains("GTK_SPLIT_LEFT_AFTER_CLICK"))
        .expect("workspace render json with clicked-left marker");
    let dump = serde_json::from_str::<serde_json::Value>(&workspace_json)
        .expect("valid workspace render json");
    let panes = dump["panes"].as_array().expect("panes array");
    assert_eq!(panes.len(), 2, "{workspace_json}");
    let left_text = panes[0]["frame"]["lines"].to_string();
    let right_text = panes[1]["frame"]["lines"].to_string();
    assert!(
        left_text.contains("GTK_SPLIT_LEFT_AFTER_CLICK"),
        "{workspace_json}"
    );
    assert!(
        !right_text.contains("GTK_SPLIT_LEFT_AFTER_CLICK"),
        "{workspace_json}"
    );
    assert_eq!(panes[0]["active"], true, "{workspace_json}");
    assert_eq!(panes[1]["active"], false, "{workspace_json}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_click_moves_shell_cursor_inside_split_pane_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") || !has_command("python3") {
        eprintln!(
            "skipping gtk split cursor e2e because xvfb-run, xdotool, or python3 is not installed"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-split-cursor-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
geometry_trace="$3"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool key --window "$window_id" ctrl+shift+e
sleep 0.5
xdotool type --window "$window_id" --delay 2 "abcdef"
latest_json=""
for _ in {1..120}; do
    latest_json="$(ls -t "$snapshot_dir"/*.workspace.render.json 2>/dev/null | head -n 1 || true)"
    if [ -n "$latest_json" ] && grep -F 'abcdef' "$latest_json" >/dev/null 2>&1 && [ -f "$geometry_trace" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$latest_json" ] || [ ! -f "$geometry_trace" ]; then
    echo "split cursor setup snapshots or geometry did not appear" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
read -r origin_col cursor_line cursor_col < <(python3 - "$latest_json" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    frame = json.load(handle)
active = next(pane for pane in frame["panes"] if pane["active"])
cursor = active["frame"]["cursor"]
print(active["origin_col"], cursor["line"], cursor["column"])
PY
)
canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
eval "$(xdotool getwindowgeometry --shell "$window_id")"
target_col="$((cursor_col - 6))"
target_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v origin="$origin_col" -v col="$target_col" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + ((origin + col + 1.5) * cell) }')"
target_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$cursor_line" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
echo "split cursor click target window=$window_id X=$X Y=$Y pane_origin=$origin_col cursor=$cursor_line,$cursor_col canvas=$canvas_x,$canvas_y cell=$cell_width line=$line_height target=$target_x,$target_y" >&2
xdotool mousemove "$target_x" "$target_y"
xdotool click 1
sleep 0.2
xdotool type --window "$window_id" --delay 2 "Z"
for _ in {1..120}; do
    latest_json="$(ls -t "$snapshot_dir"/*.workspace.render.json 2>/dev/null | head -n 1 || true)"
    if [ -n "$latest_json" ] && grep -F 'aZbcdef' "$latest_json" >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.1
done
echo "click did not move shell cursor inside split pane before typing Z" >&2
find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
latest_json="$(ls -t "$snapshot_dir"/*.workspace.render.json 2>/dev/null | head -n 1 || true)"
[ -n "$latest_json" ] && sed -n '1,120p' "$latest_json" >&2
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-split-cursor-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk split cursor e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk split cursor e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let workspace_json = snapshot_paths(&dir)
        .into_iter()
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".workspace.render.json"))
        })
        .map(|path| read_to_string(path).expect("read workspace render json"))
        .rev()
        .find(|json| json.contains("aZbcdef"))
        .expect("workspace render json with split cursor edit marker");
    let dump = serde_json::from_str::<serde_json::Value>(&workspace_json)
        .expect("valid workspace render json");
    let panes = dump["panes"].as_array().expect("panes array");
    assert_eq!(panes.len(), 2, "{workspace_json}");
    let active = panes
        .iter()
        .find(|pane| pane["active"].as_bool() == Some(true))
        .expect("active split pane");
    assert!(
        active["frame"]["lines"].to_string().contains("aZbcdef"),
        "{workspace_json}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_drag_selection_stays_in_origin_split_pane_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") || !has_command("python3") {
        eprintln!(
            "skipping gtk split selection e2e because xvfb-run, xdotool, or python3 is not installed"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-split-selection-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let clipboard_trace = dir.join("clipboard.tsv");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
clipboard_trace="$3"
geometry_trace="$4"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_CLIPBOARD_TRACE="$clipboard_trace" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
wait_contains() {
    needle="$1"
    for _ in {1..140}; do
        if grep -R "$needle" "$snapshot_dir" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.1
    done
    echo "snapshot never contained $needle" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    return 1
}
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "printf 'LEFT_SPLIT_SELECTION\n'"
xdotool key --window "$window_id" Return
wait_contains 'LEFT_SPLIT_SELECTION'
xdotool key --window "$window_id" ctrl+shift+e
sleep 0.5
xdotool type --window "$window_id" --delay 2 "printf 'RIGHT_SPLIT_SELECTION\n'"
xdotool key --window "$window_id" Return
wait_contains 'RIGHT_SPLIT_SELECTION'
latest_json=""
for _ in {1..120}; do
    latest_json="$(ls -t "$snapshot_dir"/*.workspace.render.json 2>/dev/null | head -n 1 || true)"
    if [ -n "$latest_json" ] && grep -F 'LEFT_SPLIT_SELECTION' "$latest_json" >/dev/null 2>&1 && grep -F 'RIGHT_SPLIT_SELECTION' "$latest_json" >/dev/null 2>&1 && [ -f "$geometry_trace" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$latest_json" ] || [ ! -f "$geometry_trace" ]; then
    echo "split selection setup snapshots or geometry did not appear" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
read -r left_origin right_origin marker_row < <(python3 - "$latest_json" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    frame = json.load(handle)
left = frame["panes"][0]
right = frame["panes"][1]
row = next(
    line["row"]
    for line in left["frame"]["lines"]
    if line["text"].startswith("LEFT_SPLIT_SELECTION")
)
print(left["origin_col"], right["origin_col"], row)
PY
)
canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
eval "$(xdotool getwindowgeometry --shell "$window_id")"
start_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v origin="$left_origin" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + ((origin + 0.8) * cell) }')"
start_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$marker_row" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
end_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v origin="$right_origin" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + ((origin + 2.5) * cell) }')"
mid_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v origin="$left_origin" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + ((origin + 12.0) * cell) }')"
echo "split selection drag window=$window_id X=$X Y=$Y left_origin=$left_origin right_origin=$right_origin row=$marker_row start=$start_x,$start_y end=$end_x,$start_y" >&2
xdotool mousemove "$start_x" "$start_y"
xdotool mousedown 1
sleep 0.12
xdotool mousemove "$mid_x" "$start_y"
sleep 0.12
xdotool mousemove "$end_x" "$start_y"
sleep 0.12
xdotool mouseup 1
for _ in {1..120}; do
    if grep -R '"selected_text": "LEFT_SPLIT_SELECTION' "$snapshot_dir" >/dev/null 2>&1 && grep -F 'primary	LEFT_SPLIT_SELECTION' "$clipboard_trace" >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.1
done
echo "split drag selection did not stay in the origin pane" >&2
find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
grep -R '"selected_text"' "$snapshot_dir" >&2 || true
cat "$clipboard_trace" >&2 || true
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-split-selection-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            clipboard_trace.to_str().expect("clipboard trace path utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk split selection e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk split selection e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let json = snapshot_paths(&dir)
        .into_iter()
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .map(|path| read_to_string(path).expect("read json snapshot"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(json.contains("\"selected_text\": \"LEFT_SPLIT_SELECTION"));
    let trace = read_to_string(&clipboard_trace).expect("read clipboard trace");
    assert!(
        trace
            .lines()
            .any(|line| line == "primary\tLEFT_SPLIT_SELECTION"),
        "primary selection was not exported exactly: {trace}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_drag_resizes_split_panes_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") || !has_command("python3") {
        eprintln!(
            "skipping gtk split resize e2e because xvfb-run, xdotool, or python3 is not installed"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-split-resize-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let geometry_trace = dir.join("geometry.env");
    let resize_result = dir.join("resize.tsv");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
geometry_trace="$3"
resize_result="$4"
rm -f /tmp/chelotype.log
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_DEBUG=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
wait_contains() {
    needle="$1"
    for _ in {1..140}; do
        if grep -R "$needle" "$snapshot_dir" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.1
    done
    echo "snapshot never contained $needle" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    return 1
}
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "printf 'SPLIT_RESIZE_LEFT\n'"
xdotool key --window "$window_id" Return
wait_contains 'SPLIT_RESIZE_LEFT'
xdotool key --window "$window_id" ctrl+shift+e
sleep 0.5
xdotool type --window "$window_id" --delay 2 "printf 'SPLIT_RESIZE_RIGHT\n'"
xdotool key --window "$window_id" Return
wait_contains 'SPLIT_RESIZE_RIGHT'
latest_json=""
for _ in {1..120}; do
    latest_json="$(ls -t "$snapshot_dir"/*.workspace.render.json 2>/dev/null | head -n 1 || true)"
    if [ -n "$latest_json" ] && grep -F 'SPLIT_RESIZE_LEFT' "$latest_json" >/dev/null 2>&1 && grep -F 'SPLIT_RESIZE_RIGHT' "$latest_json" >/dev/null 2>&1 && [ -f "$geometry_trace" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$latest_json" ] || [ ! -f "$geometry_trace" ]; then
    echo "split resize setup snapshots or geometry did not appear" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
read -r initial_left initial_right right_origin < <(python3 - "$latest_json" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    frame = json.load(handle)
left, right = frame["panes"][:2]
print(left["cols"], right["cols"], right["origin_col"])
PY
)
canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
boundary_x="$(awk -v canvas_x="$canvas_x" -v origin="$right_origin" -v cell="$cell_width" 'BEGIN { printf "%d", canvas_x + (origin * cell) }')"
target_x="$(awk -v boundary="$boundary_x" -v cell="$cell_width" 'BEGIN { printf "%d", boundary + (10 * cell) }')"
target_y="$(awk -v canvas_y="$canvas_y" -v line="$line_height" 'BEGIN { printf "%d", canvas_y + (3 * line) }')"
echo "split resize drag window=$window_id initial=$initial_left,$initial_right boundary=$boundary_x target=$target_x,$target_y" >&2
xdotool mousemove --window "$window_id" "$boundary_x" "$target_y"
xdotool mousedown 1
sleep 0.08
xdotool mousemove --window "$window_id" "$target_x" "$target_y"
sleep 0.08
xdotool mouseup 1
for _ in {1..140}; do
    latest_json="$(ls -t "$snapshot_dir"/*.workspace.render.json 2>/dev/null | head -n 1 || true)"
    if [ -n "$latest_json" ] && python3 - "$latest_json" "$initial_left" "$initial_right" > "$resize_result" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    frame = json.load(handle)
left, right = frame["panes"][:2]
initial_left = int(sys.argv[2])
initial_right = int(sys.argv[3])
left_cols = int(left["cols"])
right_cols = int(right["cols"])
if left_cols - initial_left >= 6 and initial_right - right_cols >= 6 and int(right["origin_col"]) == left_cols:
    print(initial_left, initial_right, left_cols, right_cols, sep="\t")
else:
    raise SystemExit(1)
PY
    then
        exit 0
    fi
    sleep 0.1
done
echo "split pane drag did not resize rendered pane geometry" >&2
echo "initial left/right: $initial_left $initial_right" >&2
find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
latest_json="$(ls -t "$snapshot_dir"/*.workspace.render.json 2>/dev/null | head -n 1 || true)"
[ -n "$latest_json" ] && sed -n '1,120p' "$latest_json" >&2
cat /tmp/chelotype.log >&2 || true
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-split-resize-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
            resize_result.to_str().expect("resize result path utf8"),
        ])
        .output()
        .expect("run gtk split resize e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk split resize e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let result = read_to_string(&resize_result).expect("read split resize result");
    let values = result
        .trim()
        .split('\t')
        .map(|value| value.parse::<u64>().expect("resize metric"))
        .collect::<Vec<_>>();
    assert_eq!(values.len(), 4, "{result}");
    assert!(
        values[2] > values[0] && values[3] < values[1],
        "split resize did not grow left and shrink right pane: {result}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_selects_text_with_real_mouse_drag_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk mouse e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-mouse-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let clipboard_trace = dir.join("clipboard.tsv");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
clipboard_trace="$3"
geometry_trace="$4"
rm -f /tmp/chelotype.log
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_DEBUG=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_CLIPBOARD_TRACE="$clipboard_trace" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "printf 'MOUSE_SELECT_OK\n'"
xdotool key --window "$window_id" Return
for _ in {1..100}; do
    if grep -R '^MOUSE_SELECT_OK' "$snapshot_dir"/*.txt >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if ! grep -R '^MOUSE_SELECT_OK' "$snapshot_dir"/*.txt >/dev/null 2>&1; then
    echo "text for mouse selection never appeared" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
latest_txt="$(ls "$snapshot_dir"/*.txt 2>/dev/null | tail -n 1)"
marker_row="$(grep -n '^MOUSE_SELECT_OK' "$latest_txt" | tail -n 1 | cut -d: -f1)"
marker_row="$((marker_row - 1))"
canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
eval "$(xdotool getwindowgeometry --shell "$window_id")"
start_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (1.0 * cell) }')"
start_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$marker_row" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
end_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (15.8 * cell) }')"
end_y="$start_y"
xdotool mousemove "$start_x" "$start_y"
xdotool mousedown 1
sleep 0.05
xdotool mousemove "$end_x" "$end_y"
sleep 0.05
xdotool mouseup 1
for _ in {1..100}; do
    if grep -R '"selected_text": "MOUSE_SELECT_OK' "$snapshot_dir" >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.1
done
echo "selection snapshot never contained MOUSE_SELECT_OK" >&2
find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
grep -R '"selected_text"' "$snapshot_dir" >&2 || true
cat /tmp/chelotype.log >&2 || true
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-mouse-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            clipboard_trace.to_str().expect("clipboard trace path utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk mouse e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk mouse e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let json = snapshot_paths(&dir)
        .into_iter()
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .map(|path| read_to_string(path).expect("read json snapshot"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(json.contains("\"selected_text\": \"MOUSE_SELECT_OK"));

    let trace = read_to_string(&clipboard_trace).expect("read clipboard trace");
    assert!(
        trace.lines().any(|line| line == "primary\tMOUSE_SELECT_OK"),
        "primary selection was not exported exactly: {trace}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_selects_text_with_mouse_drag_after_terminal_idle_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk idle mouse e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-idle-mouse-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let clipboard_trace = dir.join("clipboard.tsv");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
clipboard_trace="$3"
geometry_trace="$4"
rm -f /tmp/chelotype.log
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_DEBUG=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_CLIPBOARD_TRACE="$clipboard_trace" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
eval "$(xdotool getwindowgeometry --shell "$window_id")"
xdotool mousemove "$((X + 100))" "$((Y + 100))"
xdotool click 1
sleep 0.1
xdotool type --window "$window_id" --delay 2 "printf 'idle-selection-target\n'"
xdotool key --window "$window_id" Return
for _ in {1..120}; do
    if grep -R '^idle-selection-target' "$snapshot_dir"/*.txt >/dev/null 2>&1 && [ -f "$geometry_trace" ]; then
        break
    fi
    sleep 0.1
done
if ! grep -R '^idle-selection-target' "$snapshot_dir"/*.txt >/dev/null 2>&1; then
    echo "idle selection target never appeared" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
sleep 1.3
latest_txt="$(ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1)"
marker_row="$(grep -n '^idle-selection-target' "$latest_txt" | tail -n 1 | cut -d: -f1)"
marker_row="$((marker_row - 1))"
canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
eval "$(xdotool getwindowgeometry --shell "$window_id")"
start_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (1.5 * cell) }')"
mid_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (10.5 * cell) }')"
end_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (20.8 * cell) }')"
target_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$marker_row" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
echo "idle drag window=$window_id row=$marker_row start=$start_x,$target_y mid=$mid_x,$target_y end=$end_x,$target_y" >&2
xdotool windowfocus "$window_id" || true
xdotool mousemove "$start_x" "$target_y"
xdotool mousedown 1
sleep 0.2
xdotool mousemove "$mid_x" "$target_y"
sleep 0.2
xdotool mousemove "$end_x" "$target_y"
sleep 0.2
xdotool mouseup 1
for _ in {1..100}; do
    if grep -F 'primary	idle-selection-target' "$clipboard_trace" >/dev/null 2>&1 && grep -R '"selected_text": "idle-selection-target' "$snapshot_dir" >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.05
done
echo "idle mouse drag did not render/export selected text after terminal had gone clean" >&2
cat "$clipboard_trace" >&2 || true
grep -R '"selected_text"' "$snapshot_dir" >&2 || true
cat /tmp/chelotype.log >&2 || true
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-idle-mouse-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            clipboard_trace.to_str().expect("clipboard trace path utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk idle mouse e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk idle mouse e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let trace = read_to_string(&clipboard_trace).expect("read clipboard trace");
    assert!(
        trace
            .lines()
            .any(|line| line == "primary\tidle-selection-target"),
        "primary selection was not exported exactly: {trace}"
    );

    let json = snapshot_paths(&dir)
        .into_iter()
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .map(|path| read_to_string(path).expect("read json snapshot"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(json.contains("\"selected_text\": \"idle-selection-target"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_wayland_drag_selects_output_under_nested_weston() {
    for command in ["xvfb-run", "weston", "xdotool"] {
        if !has_command(command) {
            eprintln!("skipping nested Wayland drag e2e because {command} is not installed");
            return;
        }
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-wayland-drag-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let clipboard_trace = dir.join("clipboard.tsv");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
clipboard_trace="$3"
geometry_trace="$4"
window_id="$CHELOTYPE_NESTED_WAYLAND_X_WINDOW"
rm -f /tmp/chelotype.log
ZSH_DISABLE_COMPFIX=true DISABLE_AUTO_UPDATE=true DISABLE_UPDATE_PROMPT=true GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_DEBUG=1 CHELOTYPE_TEST_SUPPRESS_MOTION_BUTTON_MASK=1 CHELOTYPE_TEST_CLICK_CANCEL_ON_DRAG_BEGIN=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_CLIPBOARD_TRACE="$clipboard_trace" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
app_pid="$!"
cleanup() {
    kill "$app_pid" 2>/dev/null || true
    wait "$app_pid" 2>/dev/null || true
}
trap cleanup EXIT
xdotool windowfocus "$window_id" || true
sleep 0.8
xdotool type --window "$window_id" --delay 2 "printf 'WAYLAND alpha beta\n'"
xdotool key --window "$window_id" Return
for _ in {1..120}; do
    if grep -R '^WAYLAND alpha beta' "$snapshot_dir"/*.txt >/dev/null 2>&1 && [ -f "$geometry_trace" ]; then
        break
    fi
    sleep 0.1
done
if ! grep -R '^WAYLAND alpha beta' "$snapshot_dir"/*.txt >/dev/null 2>&1; then
    echo "nested Wayland output target never appeared" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
latest_txt="$(ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1)"
marker_row="$(grep -n '^WAYLAND alpha beta' "$latest_txt" | tail -n 1 | cut -d: -f1)"
marker_row="$((marker_row - 1))"
canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
eval "$(xdotool getwindowgeometry --shell "$window_id")"
start_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (0.8 * cell) }')"
mid_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (8.8 * cell) }')"
end_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (7.8 * cell) }')"
target_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$marker_row" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
xdotool mousemove "$start_x" "$target_y"
xdotool mousedown 1
sleep 0.12
xdotool mousemove "$mid_x" "$target_y"
sleep 0.12
xdotool mousemove "$end_x" "$target_y"
sleep 0.12
xdotool mouseup 1
for _ in {1..100}; do
    if grep -F 'primary	WAYLAND' "$clipboard_trace" >/dev/null 2>&1; then
        break
    fi
    sleep 0.05
done
if ! grep -F 'primary	WAYLAND' "$clipboard_trace" >/dev/null 2>&1; then
    echo "nested Wayland drag did not export selected text" >&2
    echo "window=$window_id row=$marker_row start=$start_x,$target_y end=$end_x,$target_y" >&2
    cat "$clipboard_trace" >&2 || true
    cat "$latest_txt" >&2 || true
    cat /tmp/chelotype.log >&2 || true
    exit 1
fi
word_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (14.5 * cell) }')"
xdotool mousemove "$word_x" "$target_y"
xdotool click --repeat 2 --delay 60 1
for _ in {1..100}; do
    if grep -F 'primary	beta' "$clipboard_trace" >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.05
done
echo "nested Wayland double click did not select word" >&2
echo "window=$window_id row=$marker_row word=$word_x,$target_y" >&2
cat "$clipboard_trace" >&2 || true
cat "$latest_txt" >&2 || true
cat /tmp/chelotype.log >&2 || true
exit 1
"#;

    let output = Command::new("scripts/with-nested-wayland.sh")
        .args([
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-wayland-drag-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            clipboard_trace.to_str().expect("clipboard trace path utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk drag e2e under nested Wayland");

    assert!(
        output.status.success(),
        "nested Wayland drag e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let trace = read_to_string(&clipboard_trace).expect("read clipboard trace");
    assert!(
        trace
            .lines()
            .any(|line| line.starts_with("primary\tWAYLAND"))
    );
    assert!(trace.lines().any(|line| line == "primary\tbeta"));
    let debug = read_to_string("/tmp/chelotype.log").unwrap_or_default();
    assert!(
        debug.contains("mouse click gesture cancel ignored during active drag"),
        "nested Wayland e2e did not exercise click-cancel-during-drag guard:\n{debug}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_manual_like_mouse_drag_selects_output_and_input_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!(
            "skipping gtk manual-like mouse drag e2e because xvfb-run or xdotool is not installed"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-manual-mouse-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let clipboard_trace = dir.join("clipboard.tsv");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
clipboard_trace="$3"
geometry_trace="$4"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_CLIPBOARD_TRACE="$clipboard_trace" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
latest_txt() {
    ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1
}
wait_latest_text() {
    local text="$1"
    for _ in {1..100}; do
        latest="$(latest_txt || true)"
        if [ -n "$latest" ] && grep -F "$text" "$latest" >/dev/null 2>&1 && [ -f "$geometry_trace" ]; then
            return 0
        fi
        sleep 0.1
    done
    echo "latest text did not become: $text" >&2
    latest="$(latest_txt || true)"
    [ -n "$latest" ] && cat "$latest" >&2
    return 1
}
drag_row_cols() {
    local row="$1"
    local start_col="$2"
    local end_col="$3"
    canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    eval "$(xdotool getwindowgeometry --shell "$window_id")"
    start_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v col="$start_col" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (col * cell) }')"
    end_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v col="$end_col" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (col * cell) }')"
    mid_x="$(awk -v start="$start_x" -v end="$end_x" 'BEGIN { printf "%d", (start + end) / 2 }')"
    target_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$row" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
    xdotool mousemove "$start_x" "$target_y"
    xdotool mousedown 1
    sleep 0.15
    xdotool mousemove "$mid_x" "$target_y"
    sleep 0.15
    xdotool mousemove "$end_x" "$target_y"
    sleep 0.15
    xdotool mouseup 1
}
wait_primary() {
    local text="$1"
    for _ in {1..100}; do
        if grep -F "primary	$text" "$clipboard_trace" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.05
    done
    echo "primary selection did not become: $text" >&2
    cat "$clipboard_trace" >&2 || true
    return 1
}
xdotool windowfocus "$window_id" || true
sleep 0.25
xdotool type --window "$window_id" --delay 2 "printf 'MANUAL_MOUSE_OUTPUT\n'"
xdotool key --window "$window_id" Return
wait_latest_text 'MANUAL_MOUSE_OUTPUT'
latest="$(latest_txt)"
output_row="$(grep -n '^MANUAL_MOUSE_OUTPUT' "$latest" | tail -n 1 | cut -d: -f1)"
output_row="$((output_row - 1))"
drag_row_cols "$output_row" "0.6" "19.8"
wait_primary "MANUAL_MOUSE_OUTPUT"
xdotool key --window "$window_id" ctrl+shift+c
for _ in {1..80}; do
    if grep -F 'clipboard	MANUAL_MOUSE_OUTPUT' "$clipboard_trace" >/dev/null 2>&1; then
        break
    fi
    sleep 0.05
done
if ! grep -F 'clipboard	MANUAL_MOUSE_OUTPUT' "$clipboard_trace" >/dev/null 2>&1; then
    echo "manual-like output drag did not copy selected text" >&2
    cat "$clipboard_trace" >&2 || true
    exit 1
fi
xdotool type --window "$window_id" --delay 2 "manualinput"
wait_latest_text '❯ manualinput'
latest_json="$(ls -t "$snapshot_dir"/*.json 2>/dev/null | head -n 1)"
input_row="$(sed -n 's/^  "cursor_line": \([0-9][0-9]*\),/\1/p' "$latest_json" | head -n 1)"
drag_row_cols "$input_row" "2.7" "8.2"
wait_primary "manual"
xdotool type --window "$window_id" --delay 2 "X"
wait_latest_text '❯ Xinput'
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-manual-mouse-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            clipboard_trace.to_str().expect("clipboard trace path utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk manual-like mouse e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk manual-like mouse e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let trace = read_to_string(&clipboard_trace).expect("read clipboard trace");
    assert!(
        trace
            .lines()
            .any(|line| line == "primary\tMANUAL_MOUSE_OUTPUT")
    );
    assert!(
        trace
            .lines()
            .any(|line| line == "clipboard\tMANUAL_MOUSE_OUTPUT")
    );
    assert!(trace.lines().any(|line| line == "primary\tmanual"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_drag_release_keeps_selection_stable_for_copy_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk drag release e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-drag-release-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let clipboard_trace = dir.join("clipboard.tsv");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
clipboard_trace="$3"
geometry_trace="$4"
rm -f /tmp/chelotype.log
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_DEBUG=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_CLIPBOARD_TRACE="$clipboard_trace" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "printf 'DRAG_RELEASE_STABLE\n'"
xdotool key --window "$window_id" Return
for _ in {1..100}; do
    if grep -R '^DRAG_RELEASE_STABLE' "$snapshot_dir"/*.txt >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if ! grep -R '^DRAG_RELEASE_STABLE' "$snapshot_dir"/*.txt >/dev/null 2>&1; then
    echo "text for drag release copy never appeared" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
latest_txt="$(ls "$snapshot_dir"/*.txt 2>/dev/null | tail -n 1)"
marker_row="$(grep -n '^DRAG_RELEASE_STABLE' "$latest_txt" | tail -n 1 | cut -d: -f1)"
marker_row="$((marker_row - 1))"
canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
eval "$(xdotool getwindowgeometry --shell "$window_id")"
start_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (1.0 * cell) }')"
start_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$marker_row" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
end_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (19.8 * cell) }')"
xdotool mousemove "$start_x" "$start_y"
xdotool mousedown 1
sleep 0.05
xdotool mousemove "$end_x" "$start_y"
sleep 0.05
xdotool mouseup 1
for _ in {1..100}; do
    if grep -F 'primary	DRAG_RELEASE_STABLE' "$clipboard_trace" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if ! grep -F 'primary	DRAG_RELEASE_STABLE' "$clipboard_trace" >/dev/null 2>&1; then
    echo "initial drag selection did not export expected primary text" >&2
    cat "$clipboard_trace" >&2 || true
    exit 1
fi
xdotool click 3
sleep 0.1
xdotool key --window "$window_id" Escape
sleep 0.1
xdotool mousemove 620 420
sleep 0.2
xdotool key --window "$window_id" ctrl+shift+c
for _ in {1..100}; do
    if grep -F 'clipboard	DRAG_RELEASE_STABLE' "$clipboard_trace" >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.1
done
echo "released drag selection was not stable enough to copy" >&2
cat "$clipboard_trace" >&2 || true
find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-drag-release-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            clipboard_trace.to_str().expect("clipboard trace path utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk drag release e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk drag release e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let trace = read_to_string(&clipboard_trace).expect("read clipboard trace");
    assert!(
        trace
            .lines()
            .any(|line| line == "clipboard\tDRAG_RELEASE_STABLE"),
        "selection was not stable after release: {trace}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_does_not_render_selection_over_wrong_text_after_scroll_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk selection scroll e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-selection-scroll-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
geometry_trace="$3"
rm -f /tmp/chelotype.log
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_DEBUG=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 1 "python3 -c \"for i in range(1, 41): print(f'VISUAL_FILL_{i:02d}'); print('VISUAL_SCROLL_TARGET')\""
xdotool key --window "$window_id" Return
for _ in {1..120}; do
    if grep -R '^VISUAL_SCROLL_TARGET' "$snapshot_dir"/*.txt >/dev/null 2>&1 && [ -f "$geometry_trace" ]; then
        break
    fi
    sleep 0.1
done
if ! grep -R '^VISUAL_SCROLL_TARGET' "$snapshot_dir"/*.txt >/dev/null 2>&1; then
    echo "selection scroll target never appeared" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
latest_txt="$(ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1)"
marker_row="$(grep -n '^VISUAL_SCROLL_TARGET' "$latest_txt" | tail -n 1 | cut -d: -f1)"
marker_row="$((marker_row - 1))"
canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
eval "$(xdotool getwindowgeometry --shell "$window_id")"
start_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (1.0 * cell) }')"
start_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$marker_row" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
end_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (21.8 * cell) }')"
xdotool mousemove "$start_x" "$start_y"
xdotool mousedown 1
sleep 0.05
xdotool mousemove "$end_x" "$start_y"
sleep 0.05
xdotool mouseup 1
for _ in {1..100}; do
    if grep -R '"selected_text": "VISUAL_SCROLL_TARGET' "$snapshot_dir" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if ! grep -R '"selected_text": "VISUAL_SCROLL_TARGET' "$snapshot_dir" >/dev/null 2>&1; then
    echo "initial target selection was not captured" >&2
    grep -R '"selected_text"' "$snapshot_dir" >&2 || true
    exit 1
fi
before_count="$(ls "$snapshot_dir"/*.json 2>/dev/null | wc -l)"
xdotool key --window "$window_id" shift+Page_Up
for _ in {1..40}; do
    after_count="$(ls "$snapshot_dir"/*.json 2>/dev/null | wc -l)"
    if [ "$after_count" -gt "$before_count" ]; then
        break
    fi
    sleep 0.1
done
sleep 0.2
latest_json="$(ls -t "$snapshot_dir"/*.json 2>/dev/null | head -n 1)"
if grep -F '"selected_text": "VISUAL_SCROLL_TARGET' "$latest_json" >/dev/null 2>&1; then
    exit 0
fi
if grep -F '"selected_text": ' "$latest_json" >/dev/null 2>&1 && ! grep -F '"selected_text": null' "$latest_json" >/dev/null 2>&1; then
    echo "selection highlight moved onto different visible text after scroll" >&2
    grep -F '"selected_text": ' "$latest_json" >&2 || true
    exit 1
fi
before_count="$(ls "$snapshot_dir"/*.json 2>/dev/null | wc -l)"
xdotool key --window "$window_id" shift+Page_Down
for _ in {1..60}; do
    latest_json="$(ls -t "$snapshot_dir"/*.json 2>/dev/null | head -n 1)"
    after_count="$(ls "$snapshot_dir"/*.json 2>/dev/null | wc -l)"
    if [ "$after_count" -gt "$before_count" ] && grep -F '"selected_text": "VISUAL_SCROLL_TARGET' "$latest_json" >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.1
done
echo "selection did not reattach to the original text after scrolling back" >&2
grep -R '"selected_text"' "$snapshot_dir" >&2 || true
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-selection-scroll-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk selection scroll e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk selection scroll e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_keeps_selection_anchored_during_smooth_wheel_scroll_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!(
            "skipping gtk smooth-scroll selection e2e because xvfb-run or xdotool is not installed"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-selection-smooth-scroll-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let geometry_trace = dir.join("geometry.env");
    let scroll_trace = dir.join("scroll.tsv");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
geometry_trace="$3"
scroll_trace="$4"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" CHELOTYPE_SCROLL_TRACE="$scroll_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
snapshot_count() {
    find "$snapshot_dir" -maxdepth 1 -name '*.json' 2>/dev/null | wc -l
}
latest_snapshot() {
    local extension="$1"
    find "$snapshot_dir" -maxdepth 1 -name "*.$extension" -printf '%T@ %p\n' 2>/dev/null | sort -n | tail -n 1 | sed 's/^[^ ]* //'
}
xdotool type --window "$window_id" --delay 1 "python3 -c \"for i in range(1, 80): print(f'SMOOTH_FILL_{i:02d}'); print('SMOOTH_SELECTION_TARGET')\""
xdotool key --window "$window_id" Return
for _ in {1..140}; do
    if grep -R '^SMOOTH_SELECTION_TARGET' "$snapshot_dir"/*.txt >/dev/null 2>&1 && [ -f "$geometry_trace" ]; then
        break
    fi
    sleep 0.1
done
if ! grep -R '^SMOOTH_SELECTION_TARGET' "$snapshot_dir"/*.txt >/dev/null 2>&1; then
    echo "smooth-scroll selection target never appeared" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
latest_txt="$(latest_snapshot txt)"
marker_row="$(grep -n '^SMOOTH_SELECTION_TARGET' "$latest_txt" | tail -n 1 | cut -d: -f1)"
marker_row="$((marker_row - 1))"
canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
eval "$(xdotool getwindowgeometry --shell "$window_id")"
start_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (1.0 * cell) }')"
end_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (25.5 * cell) }')"
target_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$marker_row" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
xdotool mousemove "$start_x" "$target_y"
xdotool mousedown 1
sleep 0.06
xdotool mousemove "$end_x" "$target_y"
sleep 0.06
xdotool mouseup 1
for _ in {1..100}; do
    if grep -R '"selected_text": "SMOOTH_SELECTION_TARGET' "$snapshot_dir" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if ! grep -R '"selected_text": "SMOOTH_SELECTION_TARGET' "$snapshot_dir" >/dev/null 2>&1; then
    echo "initial smooth-scroll target selection was not captured" >&2
    grep -R '"selected_text"' "$snapshot_dir" >&2 || true
    exit 1
fi
mouse_x="$((X + WIDTH / 2))"
mouse_y="$((Y + HEIGHT / 2))"
xdotool mousemove "$mouse_x" "$mouse_y"
before_count="$(snapshot_count)"
for _ in {1..12}; do
    xdotool click 4 || true
done
for _ in {1..160}; do
    after_count="$(snapshot_count)"
    positive_steps="$(awk -F '\t' '$1 == "step" && $2 == "1" { count++ } END { print count + 0 }' "$scroll_trace" 2>/dev/null || echo 0)"
    if [ "$after_count" -gt "$before_count" ] && [ "$positive_steps" -ge 12 ]; then
        break
    fi
    sleep 0.05
done
latest_json="$(latest_snapshot json)"
if grep -F '"selected_text": ' "$latest_json" >/dev/null 2>&1 && ! grep -F '"selected_text": null' "$latest_json" >/dev/null 2>&1 && ! grep -F '"selected_text": "SMOOTH_SELECTION_TARGET' "$latest_json" >/dev/null 2>&1; then
    echo "smooth wheel scroll moved selection onto different visible text" >&2
    grep -F '"selected_text": ' "$latest_json" >&2 || true
    cat "$scroll_trace" >&2 || true
    exit 1
fi
before_count="$(snapshot_count)"
for _ in {1..12}; do
    xdotool click 5 || true
done
for _ in {1..180}; do
    after_count="$(snapshot_count)"
    negative_steps="$(awk -F '\t' '$1 == "step" && $2 == "-1" { count++ } END { print count + 0 }' "$scroll_trace" 2>/dev/null || echo 0)"
    if [ "$after_count" -gt "$before_count" ] && [ "$negative_steps" -ge 12 ] && grep -R '"selected_text": "SMOOTH_SELECTION_TARGET' "$snapshot_dir" >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.05
done
echo "smooth wheel scroll did not reattach selection to original text" >&2
grep -R '"selected_text"' "$snapshot_dir" >&2 || true
cat "$scroll_trace" >&2 || true
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-selection-smooth-scroll-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
            scroll_trace.to_str().expect("scroll trace path utf8"),
        ])
        .output()
        .expect("run gtk smooth-scroll selection e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk smooth-scroll selection e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let trace = read_to_string(&scroll_trace).expect("read smooth-scroll selection trace");
    assert!(
        trace
            .lines()
            .filter(|line| line.starts_with("step\t1\t"))
            .count()
            >= 12,
        "{trace}"
    );
    assert!(
        trace
            .lines()
            .filter(|line| line.starts_with("step\t-1\t"))
            .count()
            >= 12,
        "{trace}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_copies_selection_to_clipboard_with_ctrl_shift_c_under_xvfb() {
    gtk_e2e_copies_selection_to_clipboard_with_shortcut_under_xvfb(
        "ctrl+shift+c",
        "CLIPBOARD_COPY_OK",
        "Ctrl+Shift+C",
    );
}

#[test]
#[serial]
fn gtk_e2e_copies_selection_to_clipboard_with_ctrl_c_under_xvfb() {
    gtk_e2e_copies_selection_to_clipboard_with_shortcut_under_xvfb(
        "ctrl+c",
        "CTRL_C_COPY_OK",
        "Ctrl+C",
    );
}

fn gtk_e2e_copies_selection_to_clipboard_with_shortcut_under_xvfb(
    shortcut: &str,
    marker: &str,
    label: &str,
) {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk clipboard e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-clipboard-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let clipboard_trace = dir.join("clipboard.tsv");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
clipboard_trace="$3"
geometry_trace="$4"
shortcut="$5"
marker="$6"
label="$7"
rm -f /tmp/chelotype.log
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_DEBUG=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_CLIPBOARD_TRACE="$clipboard_trace" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "printf '$marker\n'"
xdotool key --window "$window_id" Return
for _ in {1..100}; do
    if grep -R "^$marker" "$snapshot_dir"/*.txt >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if ! grep -R "^$marker" "$snapshot_dir"/*.txt >/dev/null 2>&1; then
    echo "text for clipboard copy never appeared" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
latest_txt="$(ls "$snapshot_dir"/*.txt 2>/dev/null | tail -n 1)"
marker_row="$(grep -n "^$marker" "$latest_txt" | tail -n 1 | cut -d: -f1)"
marker_row="$((marker_row - 1))"
canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
eval "$(xdotool getwindowgeometry --shell "$window_id")"
start_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (1.0 * cell) }')"
start_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$marker_row" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
end_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (17.8 * cell) }')"
xdotool mousemove "$start_x" "$start_y"
xdotool mousedown 1
sleep 0.05
xdotool mousemove "$end_x" "$start_y"
sleep 0.05
xdotool mouseup 1
for _ in {1..100}; do
    if grep -F "primary	$marker" "$clipboard_trace" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
xdotool key --window "$window_id" "$shortcut"
for _ in {1..100}; do
    if grep -F "clipboard	$marker" "$clipboard_trace" >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.1
done
echo "$label did not export selected text to clipboard" >&2
cat "$clipboard_trace" >&2 || true
find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-clipboard-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            clipboard_trace.to_str().expect("clipboard trace path utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
            shortcut,
            marker,
            label,
        ])
        .output()
        .expect("run gtk clipboard e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk clipboard e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let trace = read_to_string(&clipboard_trace).expect("read clipboard trace");
    assert!(
        trace.contains(&format!("clipboard\t{marker}")),
        "clipboard was not exported: {trace}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_ctrl_c_without_selection_interrupts_running_program_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk Ctrl+C interrupt e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-ctrl-c-interrupt-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "cat"
xdotool key --window "$window_id" Return
sleep 0.3
xdotool type --window "$window_id" --delay 2 "CAT_INPUT_BEFORE_INTERRUPT"
xdotool key --window "$window_id" Return
for _ in {1..100}; do
    if grep -R '^CAT_INPUT_BEFORE_INTERRUPT' "$snapshot_dir"/*.txt >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if ! grep -R '^CAT_INPUT_BEFORE_INTERRUPT' "$snapshot_dir"/*.txt >/dev/null 2>&1; then
    echo "cat did not echo input before interrupt" >&2
    grep -R 'CAT_INPUT' "$snapshot_dir"/*.txt >&2 || true
    exit 1
fi
xdotool key --window "$window_id" ctrl+c
sleep 0.3
xdotool type --window "$window_id" --delay 2 "printf 'AFTER_CTRL_C_INTERRUPT\n'"
xdotool key --window "$window_id" Return
for _ in {1..100}; do
    if grep -R '^AFTER_CTRL_C_INTERRUPT' "$snapshot_dir"/*.txt >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.1
done
echo "Ctrl+C without selection did not interrupt cat and return to shell" >&2
grep -R 'AFTER_CTRL_C_INTERRUPT\\|printf' "$snapshot_dir"/*.txt >&2 || true
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-ctrl-c-interrupt-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
        ])
        .output()
        .expect("run gtk Ctrl+C interrupt e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk Ctrl+C interrupt e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_replaces_selected_input_text_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk input selection e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-input-selection-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
geometry_trace="$3"
rm -f /tmp/chelotype.log
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_DEBUG=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
snapshot_count() {
    find "$snapshot_dir" -maxdepth 1 -name '*.txt' 2>/dev/null | wc -l
}
latest_txt() {
    ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1
}
wait_latest_text_after() {
    local before="$1"
    local text="$2"
    for _ in {1..160}; do
        local count
        count="$(snapshot_count)"
        latest="$(latest_txt || true)"
        if [ "$count" -gt "$before" ] && [ -n "$latest" ] && grep -F "$text" "$latest" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.1
    done
    echo "latest text after snapshot $before did not become: $text" >&2
    latest="$(latest_txt || true)"
    [ -n "$latest" ] && cat "$latest" >&2
    return 1
}
xdotool type --window "$window_id" --delay 2 "abcdef"
for _ in {1..100}; do
    if grep -R '❯ abcdef' "$snapshot_dir" >/dev/null 2>&1 && [ -f "$geometry_trace" ]; then
        break
    fi
    sleep 0.1
done
latest_json="$(ls -t "$snapshot_dir"/*.json 2>/dev/null | head -n 1 || true)"
if [ -z "$latest_json" ] || [ ! -f "$geometry_trace" ]; then
    echo "input selection setup snapshots or geometry did not appear" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
cursor_line="$(sed -n 's/^  "cursor_line": \([0-9][0-9]*\),/\1/p' "$latest_json" | head -n 1)"
canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
eval "$(xdotool getwindowgeometry --shell "$window_id")"
start_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (3.0 * cell) }')"
end_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (5.8 * cell) }')"
mid_x="$(awk -v start="$start_x" -v end="$end_x" 'BEGIN { printf "%d", (start + end) / 2 }')"
target_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$cursor_line" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
xdotool mousemove "$start_x" "$target_y"
xdotool mousedown 1
sleep 0.12
xdotool mousemove "$mid_x" "$target_y"
sleep 0.12
xdotool mousemove "$end_x" "$target_y"
sleep 0.12
xdotool mouseup 1
sleep 0.2
before="$(snapshot_count)"
xdotool type --window "$window_id" --delay 2 "X"
if ! wait_latest_text_after "$before" '❯ Xef'; then
    echo "selected input text was not replaced by typed text" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    grep -R '❯ ' "$snapshot_dir"/*.txt >&2 || true
    exit 1
fi
xdotool key --window "$window_id" ctrl+a
sleep 0.1
xdotool key --window "$window_id" BackSpace
sleep 0.2
xdotool type --window "$window_id" --delay 2 "alpha beta"
for _ in {1..100}; do
    if grep -R '❯ alpha beta' "$snapshot_dir" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
word_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (9.5 * cell) }')"
xdotool mousemove "$word_x" "$target_y"
xdotool click --repeat 2 --delay 90 1
sleep 0.1
before="$(snapshot_count)"
xdotool type --window "$window_id" --delay 2 "X"
if ! wait_latest_text_after "$before" '❯ alpha X'; then
    echo "double-clicked input word was not replaced" >&2
    grep -R '❯ ' "$snapshot_dir"/*.txt >&2 || true
    exit 1
fi
xdotool key --window "$window_id" ctrl+a
sleep 0.1
xdotool key --window "$window_id" BackSpace
sleep 0.2
xdotool type --window "$window_id" --delay 2 "alpha beta"
for _ in {1..100}; do
    if grep -R '❯ alpha beta' "$snapshot_dir" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
line_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (4.5 * cell) }')"
xdotool mousemove "$line_x" "$target_y"
xdotool click --repeat 3 --delay 90 1
sleep 0.1
before="$(snapshot_count)"
xdotool type --window "$window_id" --delay 2 "Z"
wait_latest_text_after "$before" '❯ Z' && exit 0
echo "triple-clicked input line was not replaced as input-only text" >&2
grep -R '❯ ' "$snapshot_dir"/*.txt >&2 || true
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-input-selection-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk input selection e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk input selection e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let text = snapshot_paths(&dir)
        .into_iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "txt"))
        .map(|path| read_to_string(path).expect("read text snapshot"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("❯ Xef"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_copies_cuts_and_pastes_selected_input_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk input clipboard e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-input-clipboard-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let clipboard_trace = dir.join("clipboard.tsv");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
clipboard_trace="$3"
geometry_trace="$4"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_CLIPBOARD_TRACE="$clipboard_trace" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "abcdef"
for _ in {1..100}; do
    if grep -R '❯ abcdef' "$snapshot_dir" >/dev/null 2>&1 && [ -f "$geometry_trace" ]; then
        break
    fi
    sleep 0.1
done
latest_json="$(ls -t "$snapshot_dir"/*.json 2>/dev/null | head -n 1 || true)"
if [ -z "$latest_json" ] || [ ! -f "$geometry_trace" ]; then
    echo "input clipboard setup snapshots or geometry did not appear" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
cursor_line="$(sed -n 's/^  "cursor_line": \([0-9][0-9]*\),/\1/p' "$latest_json" | head -n 1)"
canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
eval "$(xdotool getwindowgeometry --shell "$window_id")"
start_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (3.0 * cell) }')"
end_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (5.8 * cell) }')"
mid_x="$(awk -v start="$start_x" -v end="$end_x" 'BEGIN { printf "%d", (start + end) / 2 }')"
target_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$cursor_line" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
xdotool mousemove "$start_x" "$target_y"
xdotool mousedown 1
sleep 0.12
xdotool mousemove "$mid_x" "$target_y"
sleep 0.12
xdotool mousemove "$end_x" "$target_y"
sleep 0.12
xdotool mouseup 1
for _ in {1..60}; do
    if grep -F 'primary	abcd' "$clipboard_trace" >/dev/null 2>&1; then
        break
    fi
    sleep 0.05
done
xdotool key --window "$window_id" ctrl+c
for _ in {1..60}; do
    if grep -F 'clipboard	abcd' "$clipboard_trace" >/dev/null 2>&1; then
        break
    fi
    sleep 0.05
done
if ! grep -F 'clipboard	abcd' "$clipboard_trace" >/dev/null 2>&1; then
    echo "Ctrl+C did not copy selected input text" >&2
    cat "$clipboard_trace" >&2 || true
    exit 1
fi
xdotool key --window "$window_id" ctrl+x
for _ in {1..100}; do
    if grep -R '❯ ef' "$snapshot_dir" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if ! grep -R '❯ ef' "$snapshot_dir" >/dev/null 2>&1; then
    echo "Ctrl+X did not cut selected input text" >&2
    grep -R '❯ ' "$snapshot_dir"/*.txt >&2 || true
    exit 1
fi
xdotool key --window "$window_id" ctrl+v
for _ in {1..100}; do
    if grep -R '❯ abcdef' "$snapshot_dir" >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.1
done
echo "Ctrl+V did not paste clipboard text back into input" >&2
cat "$clipboard_trace" >&2 || true
grep -R '❯ ' "$snapshot_dir"/*.txt >&2 || true
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-input-clipboard-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            clipboard_trace.to_str().expect("clipboard trace path utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk input clipboard e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk input clipboard e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let trace = read_to_string(&clipboard_trace).expect("read clipboard trace");
    assert!(trace.lines().any(|line| line == "clipboard\tabcd"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_copies_selection_from_right_click_context_menu_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!(
            "skipping gtk context menu copy e2e because xvfb-run or xdotool is not installed"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-context-menu-copy-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let clipboard_trace = dir.join("clipboard.tsv");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
clipboard_trace="$3"
geometry_trace="$4"
rm -f /tmp/chelotype.log
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_DEBUG=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_CLIPBOARD_TRACE="$clipboard_trace" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "printf 'CONTEXT_COPY_OK\n'"
xdotool key --window "$window_id" Return
for _ in {1..100}; do
    if grep -R '^CONTEXT_COPY_OK' "$snapshot_dir"/*.txt >/dev/null 2>&1 && [ -f "$geometry_trace" ]; then
        break
    fi
    sleep 0.1
done
if ! grep -R '^CONTEXT_COPY_OK' "$snapshot_dir"/*.txt >/dev/null 2>&1; then
    echo "context copy target never appeared" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
latest_txt="$(ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1)"
marker_row="$(grep -n '^CONTEXT_COPY_OK' "$latest_txt" | tail -n 1 | cut -d: -f1)"
marker_row="$((marker_row - 1))"
canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
eval "$(xdotool getwindowgeometry --shell "$window_id")"
start_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (0.8 * cell) }')"
end_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (15.8 * cell) }')"
target_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$marker_row" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
xdotool mousemove "$start_x" "$target_y"
xdotool mousedown 1
sleep 0.05
xdotool mousemove "$end_x" "$target_y"
sleep 0.05
xdotool mouseup 1
for _ in {1..60}; do
    if grep -F 'primary	CONTEXT_COPY_OK' "$clipboard_trace" >/dev/null 2>&1; then
        break
    fi
    sleep 0.05
done
if ! grep -F 'primary	CONTEXT_COPY_OK' "$clipboard_trace" >/dev/null 2>&1; then
    echo "selection was not ready before opening context menu" >&2
    cat "$clipboard_trace" >&2 || true
    cat /tmp/chelotype.log >&2 || true
    exit 1
fi
menu_x="$start_x"
menu_y="$target_y"
xdotool mousemove "$menu_x" "$menu_y"
xdotool click 3
sleep 0.2
copy_x="$(awk -v x="$menu_x" 'BEGIN { printf "%d", x + 32 }')"
copy_y="$(awk -v y="$menu_y" 'BEGIN { printf "%d", y + 24 }')"
xdotool mousemove "$copy_x" "$copy_y"
xdotool click 1
for _ in {1..100}; do
    if grep -F 'clipboard	CONTEXT_COPY_OK' "$clipboard_trace" >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.05
done
echo "right-click context menu Copy did not export selected text" >&2
echo "menu=$menu_x,$menu_y copy=$copy_x,$copy_y" >&2
cat "$clipboard_trace" >&2 || true
cat /tmp/chelotype.log >&2 || true
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-context-menu-copy-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            clipboard_trace.to_str().expect("clipboard trace path utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk context menu copy e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk context menu copy e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let trace = read_to_string(&clipboard_trace).expect("read clipboard trace");
    assert!(
        trace
            .lines()
            .any(|line| line == "clipboard\tCONTEXT_COPY_OK"),
        "context menu Copy did not trace clipboard export: {trace}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_cuts_and_pastes_input_from_right_click_context_menu_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!(
            "skipping gtk context menu input edit e2e because xvfb-run or xdotool is not installed"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-context-menu-input-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let clipboard_trace = dir.join("clipboard.tsv");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
clipboard_trace="$3"
geometry_trace="$4"
rm -f /tmp/chelotype.log
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_DEBUG=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_CLIPBOARD_TRACE="$clipboard_trace" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "abcdef"
for _ in {1..100}; do
    if grep -R '❯ abcdef' "$snapshot_dir" >/dev/null 2>&1 && [ -f "$geometry_trace" ]; then
        break
    fi
    sleep 0.1
done
latest_json="$(ls -t "$snapshot_dir"/*.json 2>/dev/null | head -n 1 || true)"
if [ -z "$latest_json" ] || [ ! -f "$geometry_trace" ]; then
    echo "input context setup snapshots or geometry did not appear" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
cursor_line="$(sed -n 's/^  "cursor_line": \([0-9][0-9]*\),/\1/p' "$latest_json" | head -n 1)"
canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
eval "$(xdotool getwindowgeometry --shell "$window_id")"
start_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (3.0 * cell) }')"
end_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (5.8 * cell) }')"
target_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$cursor_line" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
xdotool mousemove "$start_x" "$target_y"
xdotool mousedown 1
sleep 0.05
xdotool mousemove "$end_x" "$target_y"
sleep 0.05
xdotool mouseup 1
for _ in {1..60}; do
    if grep -F 'primary	abcd' "$clipboard_trace" >/dev/null 2>&1; then
        break
    fi
    sleep 0.05
done
if ! grep -F 'primary	abcd' "$clipboard_trace" >/dev/null 2>&1; then
    echo "input selection was not ready before context cut" >&2
    cat "$clipboard_trace" >&2 || true
    cat /tmp/chelotype.log >&2 || true
    exit 1
fi
menu_x="$start_x"
menu_y="$target_y"
xdotool mousemove "$menu_x" "$menu_y"
xdotool click 3
sleep 0.2
cut_x="$(awk -v x="$menu_x" 'BEGIN { printf "%d", x + 28 }')"
cut_y="$(awk -v y="$menu_y" 'BEGIN { printf "%d", y + 58 }')"
xdotool mousemove "$cut_x" "$cut_y"
xdotool click 1
for _ in {1..100}; do
    if grep -F 'clipboard	abcd' "$clipboard_trace" >/dev/null 2>&1 && grep -R '❯ ef' "$snapshot_dir" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if ! grep -F 'clipboard	abcd' "$clipboard_trace" >/dev/null 2>&1 || ! grep -R '❯ ef' "$snapshot_dir" >/dev/null 2>&1; then
    echo "context menu Cut did not remove selected input text" >&2
    echo "menu=$menu_x,$menu_y cut=$cut_x,$cut_y" >&2
    cat "$clipboard_trace" >&2 || true
    grep -R '❯ ' "$snapshot_dir"/*.txt >&2 || true
    cat /tmp/chelotype.log >&2 || true
    exit 1
fi
xdotool mousemove "$menu_x" "$target_y"
xdotool click 3
sleep 0.2
paste_x="$(awk -v x="$menu_x" 'BEGIN { printf "%d", x + 36 }')"
paste_y="$(awk -v y="$menu_y" 'BEGIN { printf "%d", y + 92 }')"
xdotool mousemove "$paste_x" "$paste_y"
xdotool click 1
for _ in {1..100}; do
    if grep -R '❯ abcdef' "$snapshot_dir" >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.1
done
echo "context menu Paste did not restore cut input text" >&2
echo "menu=$menu_x,$menu_y paste=$paste_x,$paste_y" >&2
cat "$clipboard_trace" >&2 || true
grep -R '❯ ' "$snapshot_dir"/*.txt >&2 || true
cat /tmp/chelotype.log >&2 || true
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-context-menu-input-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            clipboard_trace.to_str().expect("clipboard trace path utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk context menu input e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk context menu input e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let trace = read_to_string(&clipboard_trace).expect("read clipboard trace");
    assert!(
        trace.lines().any(|line| line == "clipboard\tabcd"),
        "context menu Cut did not trace clipboard export: {trace}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_zooms_terminal_with_keyboard_and_ctrl_wheel_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk zoom e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-zoom-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
geometry_trace="$2"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ] && [ -f "$geometry_trace" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ] || [ ! -f "$geometry_trace" ]; then
    echo "chelotype window or geometry did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
initial="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
xdotool key --window "$window_id" ctrl+plus
for _ in {1..80}; do
    current="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    if awk -v current="$current" -v initial="$initial" 'BEGIN { exit !(current > initial + 0.3) }'; then
        break
    fi
    sleep 0.05
done
after_key="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
if ! awk -v current="$after_key" -v initial="$initial" 'BEGIN { exit !(current > initial + 0.3) }'; then
    echo "Ctrl+plus did not increase terminal cell width: initial=$initial current=$after_key" >&2
    exit 1
fi
eval "$(xdotool getwindowgeometry --shell "$window_id")"
xdotool mousemove "$((X + WIDTH / 2))" "$((Y + HEIGHT / 2))"
xdotool keydown Control
xdotool click 5
xdotool keyup Control
for _ in {1..80}; do
    current="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    if awk -v current="$current" -v after_key="$after_key" 'BEGIN { exit !(current < after_key - 0.3) }'; then
        exit 0
    fi
    sleep 0.05
done
current="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
echo "Ctrl+wheel did not decrease terminal cell width: before=$after_key current=$current" >&2
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-zoom-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk zoom e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk zoom e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_persists_terminal_zoom_between_app_restarts_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk zoom persistence e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-zoom-persistence-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let first_geometry_trace = dir.join("first-geometry.env");
    let second_geometry_trace = dir.join("second-geometry.env");
    let config_dir = dir.join("config");
    std::fs::create_dir_all(&config_dir).expect("config dir");

    let script = r#"
set -euo pipefail
bin="$1"
config_dir="$2"
first_geometry_trace="$3"
second_geometry_trace="$4"

launch_app() {
    local geometry_trace="$1"
    rm -f "$geometry_trace"
    GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_CONFIG_DIR="$config_dir" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
    app_pid="$!"
    window_id=""
    for _ in {1..80}; do
        window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
        if [ -n "$window_id" ] && [ -f "$geometry_trace" ]; then
            return 0
        fi
        sleep 0.1
    done
    echo "chelotype window or geometry did not appear for $geometry_trace" >&2
    return 1
}

read_cell_width() {
    sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$1"
}

app_pid=""
trap 'if [ -n "${app_pid:-}" ]; then kill "$app_pid" 2>/dev/null || true; fi' EXIT
launch_app "$first_geometry_trace"
xdotool windowfocus "$window_id" || true
sleep 0.2
initial="$(read_cell_width "$first_geometry_trace")"
xdotool key --window "$window_id" ctrl+plus
xdotool key --window "$window_id" ctrl+plus
for _ in {1..80}; do
    zoomed="$(read_cell_width "$first_geometry_trace")"
    if awk -v zoomed="$zoomed" -v initial="$initial" 'BEGIN { exit !(zoomed > initial + 0.6) }'; then
        break
    fi
    sleep 0.05
done
zoomed="$(read_cell_width "$first_geometry_trace")"
if ! awk -v zoomed="$zoomed" -v initial="$initial" 'BEGIN { exit !(zoomed > initial + 0.6) }'; then
    echo "Ctrl+plus did not persistently increase cell width: initial=$initial zoomed=$zoomed" >&2
    exit 1
fi
if ! grep -F 'font_size_tenths=150' "$config_dir/config" >/dev/null 2>&1; then
    echo "zoom config was not written after keyboard zoom" >&2
    cat "$config_dir/config" >&2 || true
    exit 1
fi
kill "$app_pid"
wait "$app_pid" 2>/dev/null || true
app_pid=""
sleep 0.3

launch_app "$second_geometry_trace"
restarted="$(read_cell_width "$second_geometry_trace")"
if ! awk -v restarted="$restarted" -v initial="$initial" 'BEGIN { exit !(restarted > initial + 0.6) }'; then
    echo "restarted app did not load persisted terminal zoom: initial=$initial restarted=$restarted" >&2
    cat "$config_dir/config" >&2 || true
    exit 1
fi
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-zoom-persistence-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            config_dir.to_str().expect("config dir utf8"),
            first_geometry_trace
                .to_str()
                .expect("first geometry trace path utf8"),
            second_geometry_trace
                .to_str()
                .expect("second geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk zoom persistence e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk zoom persistence e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_respects_system_large_text_scale_under_xvfb() {
    if !has_command("xvfb-run") {
        eprintln!("skipping gtk large text scale e2e because xvfb-run is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-large-text-scale-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("large text scale e2e dir");
    let base_geometry_trace = dir.join("base-geometry.env");
    let scaled_geometry_trace = dir.join("scaled-geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
base_geometry_trace="$2"
scaled_geometry_trace="$3"

launch_app() {
    local scale="$1"
    local geometry_trace="$2"
    GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 GDK_DPI_SCALE="$scale" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
    app_pid="$!"
    for _ in {1..80}; do
        if [ -f "$geometry_trace" ]; then
            return 0
        fi
        sleep 0.1
    done
    echo "geometry did not appear for scale=$scale" >&2
    return 1
}

read_cell_width() {
    sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$1"
}

app_pid=""
trap 'if [ -n "${app_pid:-}" ]; then kill "$app_pid" 2>/dev/null || true; fi' EXIT
launch_app 1 "$base_geometry_trace"
base_width="$(read_cell_width "$base_geometry_trace")"
kill "$app_pid"
wait "$app_pid" 2>/dev/null || true
app_pid=""
sleep 0.3

launch_app 1.5 "$scaled_geometry_trace"
scaled_width="$(read_cell_width "$scaled_geometry_trace")"

if ! awk -v scaled="$scaled_width" -v base="$base_width" 'BEGIN { exit !(scaled > base * 1.35 && scaled < base * 1.70) }'; then
    echo "large text scale did not produce expected terminal cell width: base=$base_width scaled=$scaled_width" >&2
    cat "$base_geometry_trace" >&2 || true
    cat "$scaled_geometry_trace" >&2 || true
    exit 1
fi
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-large-text-scale-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            base_geometry_trace
                .to_str()
                .expect("base geometry trace path utf8"),
            scaled_geometry_trace
                .to_str()
                .expect("scaled geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk large text scale e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk large text scale e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_opens_settings_with_keyboard_shortcut_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk settings e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-settings-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("settings e2e dir");
    let config_dir = dir.join("config");
    std::fs::create_dir_all(&config_dir).expect("config dir");

    let script = r#"
set -euo pipefail
bin="$1"
config_dir="$2"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_CONFIG_DIR="$config_dir" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool key --window "$window_id" ctrl+comma
settings_id=""
for _ in {1..60}; do
    settings_id="$(xdotool search --name 'Settings' | head -n 1 || true)"
    if [ -n "$settings_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$settings_id" ]; then
    echo "settings window did not open from Ctrl+comma" >&2
    xdotool search --name '.*' getwindowname %@ >&2 || true
    exit 1
fi
xdotool windowfocus "$settings_id" || true
sleep 0.2
eval "$(xdotool getwindowgeometry --shell "$settings_id")"
switch_x="$(awk -v x="$X" -v width="$WIDTH" 'BEGIN { printf "%d", x + width - 52 }')"
switch_left_x="$(awk -v x="$X" -v width="$WIDTH" 'BEGIN { printf "%d", x + width - 82 }')"
switch_y="$(awk -v y="$Y" 'BEGIN { printf "%d", y + 124 }')"
xdotool mousemove "$switch_x" "$switch_y"
xdotool click 1
for _ in {1..60}; do
    if grep -Fx 'cursor_animation=off' "$config_dir/config" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if ! grep -Fx 'cursor_animation=off' "$config_dir/config" >/dev/null 2>&1; then
    echo "settings switch did not disable cursor animation" >&2
    cat "$config_dir/config" >&2 || true
    exit 1
fi
xdotool mousemove "$switch_left_x" "$switch_y"
xdotool click 1
for _ in {1..60}; do
    if grep -Fx 'cursor_animation=on' "$config_dir/config" >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.1
done
echo "settings switch did not re-enable cursor animation" >&2
cat "$config_dir/config" >&2 || true
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-settings-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            config_dir.to_str().expect("config dir utf8"),
        ])
        .output()
        .expect("run gtk settings e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk settings e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_supports_keyboard_selection_and_word_navigation_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!(
            "skipping gtk keyboard selection e2e because xvfb-run or xdotool is not installed"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-keyboard-selection-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2

wait_text() {
    local text="$1"
    for _ in {1..100}; do
        if grep -R "$text" "$snapshot_dir" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.1
    done
    echo "text did not appear: $text" >&2
    grep -R '❯ ' "$snapshot_dir"/*.txt >&2 || true
    return 1
}

wait_latest_text() {
    local text="$1"
    for _ in {1..100}; do
        latest_txt="$(ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1 || true)"
        if [ -n "$latest_txt" ] && grep -F "$text" "$latest_txt" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.1
    done
    echo "latest text did not become: $text" >&2
    latest_txt="$(ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1 || true)"
    [ -n "$latest_txt" ] && cat "$latest_txt" >&2
    return 1
}

clear_input() {
    xdotool key --window "$window_id" ctrl+a
    sleep 0.1
    xdotool key --window "$window_id" BackSpace
    sleep 0.2
}

xdotool type --window "$window_id" --delay 2 "abcdef"
wait_text '❯ abcdef'
xdotool key --window "$window_id" shift+Left
xdotool type --window "$window_id" --delay 2 "X"
wait_latest_text '❯ abcdeX'
clear_input

xdotool type --window "$window_id" --delay 2 "abcdef"
wait_text '❯ abcdef'
xdotool key --window "$window_id" ctrl+a
sleep 0.1
xdotool type --window "$window_id" --delay 2 "X"
wait_text '❯ X'
clear_input

xdotool type --window "$window_id" --delay 2 "one two three"
wait_text '❯ one two three'
xdotool key --window "$window_id" ctrl+Left
sleep 0.1
xdotool key --window "$window_id" ctrl+Left
sleep 0.1
xdotool type --window "$window_id" --delay 2 "X"
wait_text '❯ one Xtwo three'
clear_input

xdotool type --window "$window_id" --delay 2 "abcdef"
wait_text '❯ abcdef'
xdotool key --window "$window_id" shift+Left
sleep 0.1
xdotool key --window "$window_id" shift+Left
sleep 0.1
xdotool type --window "$window_id" --delay 2 "X"
wait_latest_text '❯ abcdX'
clear_input

xdotool type --window "$window_id" --delay 2 "abcdef"
wait_latest_text '❯ abcdef'
xdotool key --window "$window_id" shift+Left
sleep 0.1
xdotool key --window "$window_id" shift+Right
sleep 0.1
xdotool type --window "$window_id" --delay 2 "X"
wait_latest_text '❯ abcdefX'
clear_input

xdotool type --window "$window_id" --delay 2 "abcdef"
wait_latest_text '❯ abcdef'
xdotool key --window "$window_id" shift+Left shift+Right shift+Left shift+Right
xdotool type --window "$window_id" --delay 2 "X"
wait_latest_text '❯ abcdefX'
clear_input

xdotool type --window "$window_id" --delay 2 "abcdef"
wait_latest_text '❯ abcdef'
xdotool key --window "$window_id" shift+Left
sleep 0.05
xdotool key --window "$window_id" shift+Right
sleep 0.05
xdotool key --window "$window_id" shift+Left
sleep 0.05
xdotool key --window "$window_id" shift+Right
sleep 0.05
xdotool type --window "$window_id" --delay 2 "X"
wait_latest_text '❯ abcdefX'
clear_input

xdotool type --window "$window_id" --delay 2 "abcdef"
wait_latest_text '❯ abcdef'
xdotool key --window "$window_id" shift+Left
sleep 0.1
xdotool key --window "$window_id" shift+Left
sleep 0.1
xdotool key --window "$window_id" Left
sleep 0.1
xdotool type --window "$window_id" --delay 2 "X"
wait_latest_text '❯ abcdXef'
clear_input

xdotool type --window "$window_id" --delay 2 "abcdef"
wait_latest_text '❯ abcdef'
xdotool key --window "$window_id" shift+Left
sleep 0.1
xdotool key --window "$window_id" shift+Left
sleep 0.1
xdotool key --window "$window_id" Right
sleep 0.1
xdotool type --window "$window_id" --delay 2 "X"
wait_latest_text '❯ abcdefX'
clear_input

xdotool type --window "$window_id" --delay 2 "one two three"
wait_text '❯ one two three'
xdotool key --window "$window_id" ctrl+shift+Left
sleep 0.1
xdotool type --window "$window_id" --delay 2 "X"
wait_latest_text '❯ one two X'
clear_input

xdotool type --window "$window_id" --delay 2 "abcde"
wait_latest_text '❯ abcde'
xdotool key --window "$window_id" ctrl+Left
sleep 0.1
xdotool key --window "$window_id" Right
sleep 0.1
xdotool key --window "$window_id" Right
sleep 0.1
xdotool key --window "$window_id" ctrl+shift+Right
sleep 0.1
xdotool type --window "$window_id" --delay 2 "X"
wait_latest_text '❯ abX'
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-keyboard-selection-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
        ])
        .output()
        .expect("run gtk keyboard selection e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk keyboard selection e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_supports_word_deletion_and_click_clears_input_selection_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!(
            "skipping gtk word deletion/click-clear e2e because xvfb-run or xdotool is not installed"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-word-delete-click-clear-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let clipboard_trace = dir.join("clipboard.tsv");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
clipboard_trace="$3"
geometry_trace="$4"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_CLIPBOARD_TRACE="$clipboard_trace" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
latest_txt() {
    ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1
}
wait_latest_text() {
    local text="$1"
    for _ in {1..100}; do
        latest="$(latest_txt || true)"
        if [ -n "$latest" ] && grep -F "$text" "$latest" >/dev/null 2>&1 && [ -f "$geometry_trace" ]; then
            return 0
        fi
        sleep 0.1
    done
    echo "latest text did not become: $text" >&2
    latest="$(latest_txt || true)"
    [ -n "$latest" ] && cat "$latest" >&2
    cat "$clipboard_trace" >&2 || true
    return 1
}
clear_input() {
    xdotool key --window "$window_id" ctrl+a
    sleep 0.1
    xdotool key --window "$window_id" BackSpace
    sleep 0.2
}
input_row() {
    latest_json="$(ls -t "$snapshot_dir"/*.json 2>/dev/null | head -n 1)"
    sed -n 's/^  "cursor_line": \([0-9][0-9]*\),/\1/p' "$latest_json" | head -n 1
}
drag_input_cols() {
    local row="$1"
    local start_col="$2"
    local end_col="$3"
    canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    eval "$(xdotool getwindowgeometry --shell "$window_id")"
    start_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v col="$start_col" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (col * cell) }')"
    end_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v col="$end_col" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (col * cell) }')"
    mid_x="$(awk -v start="$start_x" -v end="$end_x" 'BEGIN { printf "%d", (start + end) / 2 }')"
    target_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$row" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
    xdotool mousemove "$start_x" "$target_y"
    xdotool mousedown 1
    sleep 0.12
    xdotool mousemove "$mid_x" "$target_y"
    sleep 0.12
    xdotool mousemove "$end_x" "$target_y"
    sleep 0.12
    xdotool mouseup 1
}
click_input_col() {
    local row="$1"
    local col="$2"
    canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    eval "$(xdotool getwindowgeometry --shell "$window_id")"
    target_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v col="$col" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (col * cell) }')"
    target_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$row" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
    xdotool mousemove "$target_x" "$target_y"
    xdotool click 1
}
xdotool windowfocus "$window_id" || true
sleep 0.25

xdotool type --window "$window_id" --delay 2 "one two three"
wait_latest_text '❯ one two three'
xdotool key --window "$window_id" ctrl+BackSpace
sleep 0.15
xdotool type --window "$window_id" --delay 2 "X"
wait_latest_text '❯ one two X'
clear_input

xdotool type --window "$window_id" --delay 2 "one two three"
wait_latest_text '❯ one two three'
xdotool key --window "$window_id" ctrl+Left
sleep 0.1
xdotool key --window "$window_id" ctrl+Left
sleep 0.1
xdotool key --window "$window_id" ctrl+Delete
sleep 0.15
xdotool type --window "$window_id" --delay 2 "X"
wait_latest_text '❯ one X three'
clear_input

xdotool type --window "$window_id" --delay 2 "abcdef"
wait_latest_text '❯ abcdef'
row="$(input_row)"
drag_input_cols "$row" "2.7" "4.8"
for _ in {1..80}; do
    if grep -Fx 'primary	abc' "$clipboard_trace" >/dev/null 2>&1; then
        break
    fi
    sleep 0.05
done
if ! grep -Fx 'primary	abc' "$clipboard_trace" >/dev/null 2>&1; then
    echo "input selection was not created before click-clear check" >&2
    cat "$clipboard_trace" >&2 || true
    exit 1
fi
click_input_col "$row" "8.4"
sleep 0.15
xdotool type --window "$window_id" --delay 2 "X"
wait_latest_text '❯ abcdefX'
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-word-delete-click-clear-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            clipboard_trace.to_str().expect("clipboard trace path utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk word deletion/click-clear e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk word deletion/click-clear e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let trace = read_to_string(&clipboard_trace).expect("read clipboard trace");
    assert!(trace.lines().any(|line| line == "primary\tabc"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_keeps_input_selection_after_window_resize_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!(
            "skipping gtk input selection resize e2e because xvfb-run or xdotool is not installed"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-input-selection-resize-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let clipboard_trace = dir.join("clipboard.tsv");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
clipboard_trace="$3"
geometry_trace="$4"
rm -f /tmp/chelotype.log
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_DEBUG=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_CLIPBOARD_TRACE="$clipboard_trace" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
latest_txt() {
    ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1
}
wait_latest_text() {
    local text="$1"
    for _ in {1..100}; do
        latest="$(latest_txt || true)"
        if [ -n "$latest" ] && grep -F "$text" "$latest" >/dev/null 2>&1 && [ -f "$geometry_trace" ]; then
            return 0
        fi
        sleep 0.1
    done
    echo "latest text did not become: $text" >&2
    latest="$(latest_txt || true)"
    [ -n "$latest" ] && cat "$latest" >&2
    cat "$clipboard_trace" >&2 || true
    cat /tmp/chelotype.log >&2 || true
    return 1
}
input_row() {
    latest_json="$(ls -t "$snapshot_dir"/*.json 2>/dev/null | head -n 1)"
    sed -n 's/^  "cursor_line": \([0-9][0-9]*\),/\1/p' "$latest_json" | head -n 1
}
drag_input_cols() {
    local row="$1"
    local start_col="$2"
    local end_col="$3"
    canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    eval "$(xdotool getwindowgeometry --shell "$window_id")"
    start_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v col="$start_col" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (col * cell) }')"
    end_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v col="$end_col" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (col * cell) }')"
    mid_x="$(awk -v start="$start_x" -v end="$end_x" 'BEGIN { printf "%d", (start + end) / 2 }')"
    target_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$row" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
    xdotool mousemove "$start_x" "$target_y"
    xdotool mousedown 1
    sleep 0.12
    xdotool mousemove "$mid_x" "$target_y"
    sleep 0.12
    xdotool mousemove "$end_x" "$target_y"
    sleep 0.12
    xdotool mouseup 1
}
xdotool windowfocus "$window_id" || true
sleep 0.25
xdotool type --window "$window_id" --delay 2 "resizeinput"
wait_latest_text '❯ resizeinput'
row="$(input_row)"
drag_input_cols "$row" "9.2" "12.8"
for _ in {1..80}; do
    if grep -Fx 'primary	input' "$clipboard_trace" >/dev/null 2>&1; then
        break
    fi
    sleep 0.05
done
if ! grep -Fx 'primary	input' "$clipboard_trace" >/dev/null 2>&1; then
    echo "input suffix selection was not exported before resize" >&2
    cat "$clipboard_trace" >&2 || true
    exit 1
fi
xdotool windowsize "$window_id" 900 420
for _ in {1..100}; do
    latest_json="$(ls -t "$snapshot_dir"/*.json 2>/dev/null | head -n 1 || true)"
    if [ -n "$latest_json" ] && grep -F '"selected_text": "input"' "$latest_json" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
latest_json="$(ls -t "$snapshot_dir"/*.json 2>/dev/null | head -n 1 || true)"
if [ -z "$latest_json" ] || ! grep -F '"selected_text": "input"' "$latest_json" >/dev/null 2>&1; then
    echo "input selection was not preserved after resize" >&2
    grep -R '"selected_text"' "$snapshot_dir" >&2 || true
    cat /tmp/chelotype.log >&2 || true
    exit 1
fi
xdotool type --window "$window_id" --delay 2 "X"
wait_latest_text '❯ resizeX'
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-input-selection-resize-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            clipboard_trace.to_str().expect("clipboard trace path utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk input selection resize e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk input selection resize e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let trace = read_to_string(&clipboard_trace).expect("read clipboard trace");
    assert!(trace.lines().any(|line| line == "primary\tinput"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_supports_input_undo_and_redo_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk input undo/redo e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-input-undo-redo-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let fish_edit_trace = dir.join("fish-edit.tsv");
    let config_dir = dir.join("config");
    std::fs::create_dir_all(&config_dir).expect("config dir");
    std::fs::write(config_dir.join("config"), "startup_launch_target=host\n").expect("config");
    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
fish_edit_trace="$3"
config_dir="$4"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_CONFIG_DIR="$config_dir" CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_FISH_EDIT_TRACE="$fish_edit_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
latest_txt() {
    ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1
}
snapshot_count() {
    find "$snapshot_dir" -maxdepth 1 -name '*.txt' 2>/dev/null | wc -l
}
wait_latest_text() {
    local text="$1"
    for _ in {1..100}; do
        latest="$(latest_txt || true)"
        if [ -n "$latest" ] && grep -F "$text" "$latest" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.1
    done
    echo "latest text did not become: $text" >&2
    latest="$(latest_txt || true)"
    [ -n "$latest" ] && cat "$latest" >&2
    return 1
}
wait_latest_text_after() {
    local before="$1"
    local text="$2"
    for _ in {1..100}; do
        local count
        count="$(snapshot_count)"
        latest="$(latest_txt || true)"
        if [ "$count" -gt "$before" ] && [ -n "$latest" ] && grep -F "$text" "$latest" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.1
    done
    echo "latest text after snapshot $before did not become: $text" >&2
    latest="$(latest_txt || true)"
    [ -n "$latest" ] && cat "$latest" >&2
    cat "$fish_edit_trace" >&2 || true
    return 1
}
wait_prompt_without_input() {
    for _ in {1..100}; do
        latest="$(latest_txt || true)"
        if [ -n "$latest" ] && grep -E '^❯[[:space:]]*$' "$latest" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.1
    done
    echo "prompt did not return to empty input" >&2
    latest="$(latest_txt || true)"
    [ -n "$latest" ] && cat "$latest" >&2
    cat "$fish_edit_trace" >&2 || true
    return 1
}
wait_prompt_without_input_after() {
    local before="$1"
    for _ in {1..100}; do
        local count
        count="$(snapshot_count)"
        latest="$(latest_txt || true)"
        if [ "$count" -gt "$before" ] && [ -n "$latest" ] && grep -E '^❯[[:space:]]*$' "$latest" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.1
    done
    echo "prompt after snapshot $before did not return to empty input" >&2
    latest="$(latest_txt || true)"
    [ -n "$latest" ] && cat "$latest" >&2
    cat "$fish_edit_trace" >&2 || true
    return 1
}
before="$(snapshot_count)"
xdotool type --window "$window_id" --delay 2 "a"
wait_latest_text_after "$before" '❯ a'
before="$(snapshot_count)"
xdotool type --window "$window_id" --delay 2 "b"
wait_latest_text_after "$before" '❯ ab'
before="$(snapshot_count)"
xdotool type --window "$window_id" --delay 2 "c"
wait_latest_text_after "$before" '❯ abc'
before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+z
wait_latest_text_after "$before" '❯ ab'
before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+z
wait_latest_text_after "$before" '❯ a'
before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+z
wait_prompt_without_input_after "$before"
before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+shift+z
wait_latest_text_after "$before" '❯ a'
before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+shift+z
wait_latest_text_after "$before" '❯ ab'
before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+z
wait_latest_text_after "$before" '❯ a'
before="$(snapshot_count)"
xdotool type --window "$window_id" --delay 2 "X"
wait_latest_text_after "$before" '❯ aX'
before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+shift+z
wait_latest_text_after "$before" '❯ aX'

before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+a
xdotool key --window "$window_id" BackSpace
wait_prompt_without_input_after "$before"
before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+z
wait_latest_text_after "$before" '❯ aX'
before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+a
xdotool type --window "$window_id" --delay 2 "Z"
wait_latest_text_after "$before" '❯ Z'
before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+z
wait_latest_text_after "$before" '❯ aX'
xdotool key --window "$window_id" ctrl+a
xdotool key --window "$window_id" ctrl+c
sleep 0.3
before="$(snapshot_count)"
xdotool key --window "$window_id" BackSpace
wait_prompt_without_input_after "$before"
before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+v
wait_latest_text_after "$before" '❯ aX'
before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+z
wait_prompt_without_input_after "$before"
before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+shift+z
wait_latest_text_after "$before" '❯ aX'
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-input-undo-redo-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            fish_edit_trace.to_str().expect("fish edit trace path utf8"),
            config_dir.to_str().expect("config dir utf8"),
        ])
        .output()
        .expect("run gtk input undo/redo e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk input undo/redo e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_undo_redo_treats_selection_edits_as_single_steps_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!(
            "skipping gtk selection undo/redo e2e because xvfb-run or xdotool is not installed"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-selection-undo-redo-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let clipboard_trace = dir.join("clipboard.tsv");
    let config_dir = dir.join("config");
    let geometry_trace = dir.join("geometry.env");
    std::fs::create_dir_all(&config_dir).expect("config dir");
    std::fs::write(config_dir.join("config"), "startup_launch_target=host\n").expect("config");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
clipboard_trace="$3"
config_dir="$4"
geometry_trace="$5"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_CONFIG_DIR="$config_dir" CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_CLIPBOARD_TRACE="$clipboard_trace" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
latest_txt() {
    ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1
}
snapshot_count() {
    find "$snapshot_dir" -maxdepth 1 -name '*.txt' 2>/dev/null | wc -l
}
wait_latest_text_after() {
    local before="$1"
    local text="$2"
    for _ in {1..100}; do
        local count
        count="$(snapshot_count)"
        latest="$(latest_txt || true)"
        if [ "$count" -gt "$before" ] && [ -n "$latest" ] && grep -F "$text" "$latest" >/dev/null 2>&1 && [ -f "$geometry_trace" ]; then
            return 0
        fi
        sleep 0.1
    done
    echo "latest text after snapshot $before did not become: $text" >&2
    latest="$(latest_txt || true)"
    [ -n "$latest" ] && cat "$latest" >&2
    cat "$clipboard_trace" >&2 || true
    return 1
}
wait_latest_text() {
    local text="$1"
    local before
    before="$(snapshot_count)"
    wait_latest_text_after "$before" "$text"
}
input_row() {
    latest_json="$(ls -t "$snapshot_dir"/*.json 2>/dev/null | head -n 1)"
    sed -n 's/^  "cursor_line": \([0-9][0-9]*\),/\1/p' "$latest_json" | head -n 1
}
drag_input_prefix() {
    local row="$1"
    canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    eval "$(xdotool getwindowgeometry --shell "$window_id")"
    start_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (3.0 * cell) }')"
    end_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (5.8 * cell) }')"
    mid_x="$(awk -v start="$start_x" -v end="$end_x" 'BEGIN { printf "%d", (start + end) / 2 }')"
    target_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$row" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
    xdotool mousemove "$start_x" "$target_y"
    xdotool mousedown 1
    sleep 0.12
    xdotool mousemove "$mid_x" "$target_y"
    sleep 0.12
    xdotool mousemove "$end_x" "$target_y"
    sleep 0.12
    xdotool mouseup 1
}
wait_selected_prefix() {
    for _ in {1..80}; do
        if grep -Fx 'primary	abcd' "$clipboard_trace" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.05
    done
    echo "input prefix selection did not export" >&2
    cat "$clipboard_trace" >&2 || true
    return 1
}
clear_input() {
    before="$(snapshot_count)"
    xdotool key --window "$window_id" ctrl+a
    xdotool key --window "$window_id" BackSpace
    wait_latest_text_after "$before" '❯ '
}
prepare_selected_input() {
    xdotool type --window "$window_id" --delay 2 "abcdef"
    wait_latest_text '❯ abcdef'
    row="$(input_row)"
    drag_input_prefix "$row"
    wait_selected_prefix
}

prepare_selected_input
before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+x
wait_latest_text_after "$before" '❯ ef'
before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+z
wait_latest_text_after "$before" '❯ abcdef'
before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+shift+z
wait_latest_text_after "$before" '❯ ef'
clear_input

prepare_selected_input
before="$(snapshot_count)"
xdotool key --window "$window_id" BackSpace
wait_latest_text_after "$before" '❯ ef'
before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+z
wait_latest_text_after "$before" '❯ abcdef'
before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+shift+z
wait_latest_text_after "$before" '❯ ef'
clear_input

prepare_selected_input
before="$(snapshot_count)"
xdotool key --window "$window_id" Delete
wait_latest_text_after "$before" '❯ ef'
before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+z
wait_latest_text_after "$before" '❯ abcdef'
before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+shift+z
wait_latest_text_after "$before" '❯ ef'
clear_input

prepare_selected_input
before="$(snapshot_count)"
xdotool type --window "$window_id" --delay 2 "X"
wait_latest_text_after "$before" '❯ Xef'
before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+z
wait_latest_text_after "$before" '❯ abcdef'
before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+shift+z
wait_latest_text_after "$before" '❯ Xef'
clear_input

before="$(snapshot_count)"
xdotool type --window "$window_id" --delay 2 "PASTE"
wait_latest_text_after "$before" '❯ PASTE'
xdotool key --window "$window_id" ctrl+a
for _ in {1..80}; do
    if grep -Fx 'primary	PASTE' "$clipboard_trace" >/dev/null 2>&1; then
        break
    fi
    sleep 0.05
done
xdotool key --window "$window_id" ctrl+c
for _ in {1..80}; do
    if grep -Fx 'clipboard	PASTE' "$clipboard_trace" >/dev/null 2>&1; then
        break
    fi
    sleep 0.05
done
if ! grep -Fx 'clipboard	PASTE' "$clipboard_trace" >/dev/null 2>&1; then
    echo "Ctrl+C did not populate clipboard for selected paste replacement" >&2
    cat "$clipboard_trace" >&2 || true
    exit 1
fi
clear_input

prepare_selected_input
before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+v
wait_latest_text_after "$before" '❯ PASTEef'
before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+z
wait_latest_text_after "$before" '❯ abcdef'
before="$(snapshot_count)"
xdotool key --window "$window_id" ctrl+shift+z
wait_latest_text_after "$before" '❯ PASTEef'
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-selection-undo-redo-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            clipboard_trace.to_str().expect("clipboard trace path utf8"),
            config_dir.to_str().expect("config dir utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk selection undo/redo e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk selection undo/redo e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let trace = read_to_string(&clipboard_trace).expect("read clipboard trace");
    assert!(trace.lines().any(|line| line == "primary\tabcd"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_supports_double_and_triple_click_selection_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk click selection e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-click-selection-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let clipboard_trace = dir.join("clipboard.tsv");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
clipboard_trace="$3"
geometry_trace="$4"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_CLIPBOARD_TRACE="$clipboard_trace" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "printf 'alpha beta gamma\n'"
xdotool key --window "$window_id" Return
for _ in {1..100}; do
    if grep -R '^alpha beta gamma' "$snapshot_dir"/*.txt >/dev/null 2>&1 && [ -f "$geometry_trace" ]; then
        break
    fi
    sleep 0.1
done
if ! grep -R '^alpha beta gamma' "$snapshot_dir"/*.txt >/dev/null 2>&1; then
    echo "click selection target never appeared" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
latest_txt="$(ls "$snapshot_dir"/*.txt 2>/dev/null | tail -n 1)"
marker_row="$(grep -n '^alpha beta gamma' "$latest_txt" | tail -n 1 | cut -d: -f1)"
marker_row="$((marker_row - 1))"
canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
eval "$(xdotool getwindowgeometry --shell "$window_id")"
word_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (7.5 * cell) }')"
line_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$marker_row" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
xdotool mousemove "$word_x" "$line_y"
xdotool click --repeat 2 --delay 40 1
for _ in {1..100}; do
    if grep -F 'primary	beta' "$clipboard_trace" >/dev/null 2>&1; then
        break
    fi
    sleep 0.05
done
if ! grep -F 'primary	beta' "$clipboard_trace" >/dev/null 2>&1; then
    echo "double click did not select word beta" >&2
    cat "$clipboard_trace" >&2 || true
    exit 1
fi
xdotool click --repeat 3 --delay 40 1
for _ in {1..100}; do
    if grep -F 'primary	alpha beta gamma' "$clipboard_trace" >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.05
done
echo "triple click did not select full line" >&2
cat "$clipboard_trace" >&2 || true
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-click-selection-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            clipboard_trace.to_str().expect("clipboard trace path utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk click selection e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk click selection e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let trace = read_to_string(&clipboard_trace).expect("read clipboard trace");
    assert!(trace.lines().any(|line| line == "primary\tbeta"));
    assert!(
        trace
            .lines()
            .any(|line| line == "primary\talpha beta gamma")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_replaces_active_input_after_double_and_triple_click_selection_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!(
            "skipping gtk input click replacement e2e because xvfb-run or xdotool is not installed"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-input-click-replacement-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let clipboard_trace = dir.join("clipboard.tsv");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
clipboard_trace="$3"
geometry_trace="$4"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_CLIPBOARD_TRACE="$clipboard_trace" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
latest_json() {
    ls -t "$snapshot_dir"/*.json 2>/dev/null | head -n 1
}
latest_txt() {
    ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1
}
wait_latest_text() {
    local text="$1"
    for _ in {1..100}; do
        latest="$(latest_txt || true)"
        if [ -n "$latest" ] && grep -F "$text" "$latest" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.1
    done
    echo "latest text did not become: $text" >&2
    latest="$(latest_txt || true)"
    [ -n "$latest" ] && cat "$latest" >&2
    cat "$clipboard_trace" >&2 || true
    return 1
}
wait_primary() {
    local text="$1"
    for _ in {1..80}; do
        if grep -F "primary	$text" "$clipboard_trace" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.05
    done
    echo "primary selection did not become: $text" >&2
    cat "$clipboard_trace" >&2 || true
    return 1
}
input_point() {
    local col="$1"
    json="$(latest_json)"
    cursor_line="$(sed -n 's/^  "cursor_line": \([0-9][0-9]*\),/\1/p' "$json" | head -n 1)"
    canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
    eval "$(xdotool getwindowgeometry --shell "$window_id")"
    point_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" -v col="$col" 'BEGIN { printf "%d", left + canvas_x + (col * cell) }')"
    point_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$cursor_line" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
}
clear_input() {
    xdotool key --window "$window_id" ctrl+a
    sleep 0.1
    xdotool key --window "$window_id" BackSpace
    sleep 0.2
}
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "alpha beta gamma"
wait_latest_text '❯ alpha beta gamma'
input_point "9.5"
xdotool mousemove "$point_x" "$point_y"
xdotool click --repeat 2 --delay 40 1
wait_primary "beta"
xdotool type --window "$window_id" --delay 2 "X"
wait_latest_text '❯ alpha X gamma'
clear_input

xdotool type --window "$window_id" --delay 2 "alpha beta gamma"
wait_latest_text '❯ alpha beta gamma'
input_point "9.5"
xdotool mousemove "$point_x" "$point_y"
xdotool click --repeat 3 --delay 40 1
wait_primary "alpha beta gamma"
xdotool type --window "$window_id" --delay 2 "Y"
wait_latest_text '❯ Y'
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-input-click-replacement-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            clipboard_trace.to_str().expect("clipboard trace path utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk input click replacement e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk input click replacement e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let trace = read_to_string(&clipboard_trace).expect("read clipboard trace");
    assert!(trace.lines().any(|line| line == "primary\tbeta"));
    assert!(
        trace
            .lines()
            .any(|line| line == "primary\talpha beta gamma")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_keeps_selection_copy_stable_after_scrollback_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk scroll selection e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-scroll-selection-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let clipboard_trace = dir.join("clipboard.tsv");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
clipboard_trace="$3"
geometry_trace="$4"
rm -f /tmp/chelotype.log
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_DEBUG=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_CLIPBOARD_TRACE="$clipboard_trace" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "python3 -u -c \"import time; print('SCROLL_STABLE_TARGET', flush=True); time.sleep(5.0); [print(f'filler_{i}', flush=True) for i in range(1, 81)]\""
xdotool key --window "$window_id" Return
for _ in {1..100}; do
    if grep -R '^SCROLL_STABLE_TARGET' "$snapshot_dir"/*.txt >/dev/null 2>&1 && [ -f "$geometry_trace" ]; then
        break
    fi
    sleep 0.1
done
if ! grep -R '^SCROLL_STABLE_TARGET' "$snapshot_dir"/*.txt >/dev/null 2>&1; then
    echo "scroll selection target never appeared" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
sleep 0.2
latest_txt="$(ls "$snapshot_dir"/*.txt 2>/dev/null | tail -n 1)"
marker_row="$(grep -n '^SCROLL_STABLE_TARGET' "$latest_txt" | tail -n 1 | cut -d: -f1)"
marker_row="$((marker_row - 1))"
canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
eval "$(xdotool getwindowgeometry --shell "$window_id")"
start_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (0.8 * cell) }')"
end_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (20.8 * cell) }')"
target_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$marker_row" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
xdotool mouseup 1 || true
xdotool windowfocus "$window_id" || true
xdotool mousemove "$start_x" "$target_y"
xdotool mousedown 1
sleep 0.05
xdotool mousemove "$end_x" "$target_y"
sleep 0.05
xdotool mouseup 1
for _ in {1..60}; do
    if grep -F 'primary	SCROLL_STABLE_TARGET' "$clipboard_trace" >/dev/null 2>&1; then
        break
    fi
    sleep 0.05
done
if ! grep -F 'primary	SCROLL_STABLE_TARGET' "$clipboard_trace" >/dev/null 2>&1; then
    echo "initial selected text was not exported before scroll" >&2
    cat "$clipboard_trace" >&2 || true
    cat /tmp/chelotype.log >&2 || true
    exit 1
fi
for _ in {1..120}; do
    if grep -R '^filler_80' "$snapshot_dir"/*.txt >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if ! grep -R '^filler_80' "$snapshot_dir"/*.txt >/dev/null 2>&1; then
    echo "delayed output did not move the viewport after selection" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
sleep 0.2
xdotool key --window "$window_id" ctrl+shift+c
for _ in {1..100}; do
    if grep -F 'clipboard	SCROLL_STABLE_TARGET' "$clipboard_trace" >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.1
done
echo "scroll changed copied selection text" >&2
cat "$clipboard_trace" >&2 || true
cat /tmp/chelotype.log >&2 || true
find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-scroll-selection-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            clipboard_trace.to_str().expect("clipboard trace path utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk scroll selection e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk scroll selection e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let trace = read_to_string(&clipboard_trace).expect("read clipboard trace");
    assert!(
        trace
            .lines()
            .any(|line| line == "clipboard\tSCROLL_STABLE_TARGET"),
        "selection copy did not survive scroll: {trace}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_click_moves_shell_cursor_on_current_input_row_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk click cursor e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-click-cursor-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
geometry_trace="$3"
rm -f /tmp/chelotype.log
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_DEBUG=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "abcdef"
for _ in {1..100}; do
    if grep -R '❯ abcdef' "$snapshot_dir" >/dev/null 2>&1 && [ -f "$geometry_trace" ]; then
        break
    fi
    sleep 0.1
done
latest_json="$(ls "$snapshot_dir"/*.json 2>/dev/null | tail -n 1 || true)"
if [ -z "$latest_json" ] || [ ! -f "$geometry_trace" ]; then
    echo "click setup snapshots or geometry did not appear" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
cursor_line="$(sed -n 's/^  "cursor_line": \([0-9][0-9]*\),/\1/p' "$latest_json" | head -n 1)"
cursor_col="$(sed -n 's/^  "cursor_col": \([0-9][0-9]*\),/\1/p' "$latest_json" | head -n 1)"
canvas_height="$(sed -n 's/^canvas_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
canvas_width="$(sed -n 's/^canvas_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
eval "$(xdotool getwindowgeometry --shell "$window_id")"
target_col="$((cursor_col - 6))"
target_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v col="$target_col" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + ((col + 1.5) * cell) }')"
target_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$cursor_line" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
echo "click target window=$window_id X=$X Y=$Y WIDTH=$WIDTH HEIGHT=$HEIGHT cursor=$cursor_line,$cursor_col canvas=$canvas_x,$canvas_y,$canvas_width,$canvas_height cell=$cell_width line=$line_height target=$target_x,$target_y" >&2
xdotool mousemove "$target_x" "$target_y"
xdotool click 1
sleep 0.2
xdotool type --window "$window_id" --delay 2 "Z"
for _ in {1..100}; do
    if grep -R 'aZbcdef' "$snapshot_dir" >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.1
done
echo "click did not move shell cursor before typing Z" >&2
find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
for file in "$snapshot_dir"/*.txt; do
    [ -f "$file" ] || continue
    echo "===$file" >&2
    sed -n '1,8p' "$file" >&2
done
cat /tmp/chelotype.log >&2 || true
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-click-cursor-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk click cursor e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk click cursor e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let text = snapshot_paths(&dir)
        .into_iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "txt"))
        .map(|path| read_to_string(path).expect("read text snapshot"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("aZbcdef"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_keeps_zsh_autosuggestion_on_grid_and_clicks_real_buffer_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping zsh autosuggestion e2e because xvfb-run or xdotool is not installed");
        return;
    }
    let autosuggest_plugin = std::path::Path::new(
        "/opt/oh-my-zsh/custom/plugins/zsh-autosuggestions/zsh-autosuggestions.zsh",
    );
    let syntax_plugin = std::path::Path::new(
        "/opt/oh-my-zsh/custom/plugins/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh",
    );
    if !autosuggest_plugin.exists() || !syntax_plugin.exists() {
        eprintln!("skipping zsh autosuggestion e2e because zsh plugins are not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-zsh-autosuggest-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    let zdot = dir.join("zdot");
    let snapshots = dir.join("snapshots");
    std::fs::create_dir_all(&zdot).expect("zdot dir");
    std::fs::create_dir_all(&snapshots).expect("snapshot dir");
    std::fs::write(zdot.join(".zsh_history"), "git status\n").expect("history fixture");
    std::fs::write(
        zdot.join(".zshrc"),
        format!(
            "HISTFILE=\"{}\"\nHISTSIZE=1000\nSAVEHIST=1000\nPS1=\"❯ \"\nZSH_AUTOSUGGEST_STRATEGY=(history)\nsource {}\nsource {}\n",
            zdot.join(".zsh_history").display(),
            autosuggest_plugin.display(),
            syntax_plugin.display()
        ),
    )
    .expect("zshrc fixture");
    let geometry_trace = dir.join("geometry.env");
    let autosuggest_snapshot = dir.join("autosuggest.json");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
geometry_trace="$3"
zdot="$4"
autosuggest_snapshot="$5"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" CHELOTYPE_SHELL=/usr/bin/zsh ZDOTDIR="$zdot" HOME="$zdot" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.35
xdotool type --window "$window_id" --delay 10 "git"
for _ in {1..120}; do
    latest_json="$(ls -t "$snapshot_dir"/*.json 2>/dev/null | head -n 1 || true)"
    if [ -n "$latest_json" ] && grep -F '❯ git status' "$latest_json" >/dev/null 2>&1 && [ -f "$geometry_trace" ]; then
        cp "$latest_json" "$autosuggest_snapshot"
        break
    fi
    sleep 0.1
done
if [ ! -f "$autosuggest_snapshot" ]; then
    echo "zsh autosuggestion snapshot never appeared" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
cursor_line="$(sed -n 's/^  "cursor_line": \([0-9][0-9]*\),/\1/p' "$autosuggest_snapshot" | head -n 1)"
canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
eval "$(xdotool getwindowgeometry --shell "$window_id")"
target_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (3.25 * cell) }')"
target_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$cursor_line" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
xdotool mousemove "$target_x" "$target_y"
xdotool click 1
sleep 0.2
xdotool type --window "$window_id" --delay 10 "X"
for _ in {1..120}; do
    if grep -R '❯ gXit' "$snapshot_dir" >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.1
done
echo "click did not move cursor inside real zsh buffer with an autosuggestion present" >&2
find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
for file in "$snapshot_dir"/*.txt; do
    [ -f "$file" ] || continue
    echo "===$file" >&2
    sed -n '1,14p' "$file" >&2
done
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-zsh-autosuggest-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            snapshots.to_str().expect("snapshot dir utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
            zdot.to_str().expect("zdot path utf8"),
            autosuggest_snapshot
                .to_str()
                .expect("autosuggest snapshot path utf8"),
        ])
        .output()
        .expect("run gtk zsh autosuggestion e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk zsh autosuggestion e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let snapshot = read_to_string(&autosuggest_snapshot).expect("read autosuggest snapshot");
    let snapshot: serde_json::Value =
        serde_json::from_str(&snapshot).expect("valid autosuggest snapshot json");
    let line = snapshot["lines"]
        .as_array()
        .expect("snapshot lines")
        .iter()
        .find(|line| {
            line["cells"]
                .as_array()
                .expect("line cells")
                .iter()
                .filter_map(|cell| cell["text"].as_str())
                .collect::<String>()
                .contains("❯ git status")
        })
        .expect("autosuggestion line");
    let cells = line["cells"].as_array().expect("line cells");
    assert_eq!(cells[2]["text"], "g");
    assert_eq!(cells[5]["text"], " ");
    assert_eq!(cells[6]["text"], "s");
    assert_eq!(cells[5]["fg"], "#666666");
    assert_eq!(cells[6]["fg"], "#666666");

    let text = snapshot_paths(&snapshots)
        .into_iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "txt"))
        .map(|path| read_to_string(path).expect("read text snapshot"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("❯ gXit"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_forwards_terminal_mouse_reporting_to_pty_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk mouse reporting e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-mouse-report-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
geometry_trace="$3"
rm -f /tmp/chelotype.log
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_DEBUG=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
cmd="stty raw -echo; printf '\033[?1000h\033[?1006h'; dd bs=1 count=9 2>/dev/null | od -An -tx1; printf '\033[?1006l\033[?1000l'; stty sane; printf '\nMOUSE_REPORT_DONE\n'"
xdotool type --window "$window_id" --delay 1 "$cmd"
xdotool key --window "$window_id" Return
for _ in {1..100}; do
    if grep -R '"click": true' "$snapshot_dir"/*.json >/dev/null 2>&1 && grep -R '"sgr": true' "$snapshot_dir"/*.json >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if ! grep -R '"click": true' "$snapshot_dir"/*.json >/dev/null 2>&1 || ! grep -R '"sgr": true' "$snapshot_dir"/*.json >/dev/null 2>&1; then
    echo "terminal mouse mode never appeared in snapshots" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
eval "$(xdotool getwindowgeometry --shell "$window_id")"
click_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (2.0 * cell) }')"
click_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + (4.5 * line) }')"
xdotool mousemove "$click_x" "$click_y"
xdotool mousedown 1
sleep 0.05
xdotool mouseup 1
for _ in {1..100}; do
    if grep -R 'MOUSE_REPORT_DONE' "$snapshot_dir" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if ! grep -R 'MOUSE_REPORT_DONE' "$snapshot_dir" >/dev/null 2>&1; then
    echo "mouse-report command never completed" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
if grep -R '1b 5b 3c 30 3b' "$snapshot_dir" | grep '4d' >/dev/null 2>&1; then
    exit 0
fi
if grep -F 'Write([27, 91, 60, 48, 59' /tmp/chelotype.log >/dev/null 2>&1; then
    exit 0
fi
echo "shell did not receive SGR mouse press bytes" >&2
find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
for file in "$snapshot_dir"/*.txt; do
    [ -f "$file" ] || continue
    echo "===$file" >&2
    sed -n '1,12p' "$file" >&2
done
cat /tmp/chelotype.log >&2 || true
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-mouse-report-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk mouse reporting e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk mouse reporting e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_resizes_real_window_and_terminal_grid_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk resize e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-resize-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
initial_rows=""
for _ in {1..80}; do
    latest_json="$(ls "$snapshot_dir"/*.json 2>/dev/null | tail -n 1 || true)"
    if [ -n "$latest_json" ]; then
        initial_rows="$(sed -n 's/^  "rows": \([0-9][0-9]*\),/\1/p' "$latest_json" | head -n 1)"
    fi
    if [ -n "$initial_rows" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$initial_rows" ]; then
    echo "initial resize snapshot never appeared" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
xdotool windowsize "$window_id" 900 320
for _ in {1..80}; do
    for json in "$snapshot_dir"/*.json; do
        [ -f "$json" ] || continue
        rows="$(sed -n 's/^  "rows": \([0-9][0-9]*\),/\1/p' "$json" | head -n 1)"
        if [ -n "$rows" ] && [ "$rows" -lt "$initial_rows" ]; then
            exit 0
        fi
    done
    sleep 0.1
done
echo "terminal rows did not shrink after GTK window resize" >&2
echo "initial rows: $initial_rows" >&2
grep -R '"rows"' "$snapshot_dir" >&2 || true
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-resize-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
        ])
        .output()
        .expect("run gtk resize e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk resize e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let rows = snapshot_paths(&dir)
        .into_iter()
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .map(|path| read_to_string(path).expect("read json snapshot"))
        .map(|json| {
            serde_json::from_str::<serde_json::Value>(&json).expect("valid snapshot json")["rows"]
                .as_u64()
                .expect("numeric rows")
        })
        .collect::<Vec<_>>();
    let min_rows = rows.iter().min().expect("resize snapshots");
    let max_rows = rows.iter().max().expect("resize snapshots");
    assert!(min_rows < max_rows, "resize rows did not change: {rows:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_reflows_wrapped_output_after_window_resize_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk reflow resize e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-reflow-resize-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
target="GTK_REFLOW_$(printf 'x%.0s' {1..90})"
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
xdotool windowsize "$window_id" 420 360
for _ in {1..100}; do
    latest_json="$(ls -t "$snapshot_dir"/*.json 2>/dev/null | head -n 1 || true)"
    if [ -n "$latest_json" ]; then
        cols="$(sed -n 's/^  "cols": \([0-9][0-9]*\),/\1/p' "$latest_json" | head -n 1)"
        if [ -n "$cols" ] && [ "$cols" -lt "${#target}" ]; then
            break
        fi
    fi
    sleep 0.1
done
latest_json="$(ls -t "$snapshot_dir"/*.json 2>/dev/null | head -n 1 || true)"
cols="$(sed -n 's/^  "cols": \([0-9][0-9]*\),/\1/p' "$latest_json" | head -n 1)"
if [ -z "${cols:-}" ] || [ "$cols" -ge "${#target}" ]; then
    echo "narrow precondition did not produce a wrapped-width terminal" >&2
    grep -R '"cols"' "$snapshot_dir" >&2 || true
    exit 1
fi
xdotool type --window "$window_id" --delay 1 "python3 -c 'print(\"GTK_REFLOW_\" + \"x\" * 90)'"
xdotool key --window "$window_id" Return
for _ in {1..120}; do
    if grep -R 'GTK_REFLOW_' "$snapshot_dir"/*.txt >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if ! grep -R 'GTK_REFLOW_' "$snapshot_dir"/*.txt >/dev/null 2>&1; then
    echo "reflow target never appeared while narrow" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
xdotool windowsize "$window_id" 1100 500
for _ in {1..120}; do
    latest_txt="$(ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1 || true)"
    latest_json="$(ls -t "$snapshot_dir"/*.json 2>/dev/null | head -n 1 || true)"
    if [ -n "$latest_txt" ] && [ -n "$latest_json" ] && grep -F "$target" "$latest_txt" >/dev/null 2>&1; then
        cols="$(sed -n 's/^  "cols": \([0-9][0-9]*\),/\1/p' "$latest_json" | head -n 1)"
        if [ -n "$cols" ] && [ "$cols" -ge "${#target}" ]; then
            exit 0
        fi
    fi
    sleep 0.1
done
echo "GTK resize did not reflow wrapped output into one full-width line" >&2
echo "target length: ${#target}" >&2
grep -R '"cols"' "$snapshot_dir" >&2 || true
latest_txt="$(ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1 || true)"
[ -n "$latest_txt" ] && cat "$latest_txt" >&2
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-reflow-resize-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
        ])
        .output()
        .expect("run gtk reflow resize e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk reflow resize e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let text = snapshot_paths(&dir)
        .into_iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "txt"))
        .map(|path| read_to_string(path).expect("read text snapshot"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.lines()
            .any(|line| { line.trim_end() == format!("GTK_REFLOW_{}", "x".repeat(90)) })
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_preserves_selection_highlight_after_reflow_resize_under_xvfb() {
    if !has_command("xvfb-run") || !has_command("xdotool") {
        eprintln!("skipping gtk reflow selection e2e because xvfb-run or xdotool is not installed");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "chelotype-gtk-reflow-selection-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let clipboard_trace = dir.join("clipboard.tsv");
    let geometry_trace = dir.join("geometry.env");

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
clipboard_trace="$3"
geometry_trace="$4"
target="KEEP_TOKENyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy"
rm -f /tmp/chelotype.log
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_DEBUG=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_CLIPBOARD_TRACE="$clipboard_trace" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" "$bin" &
pid="$!"
trap 'kill "$pid" 2>/dev/null || true' EXIT
window_id=""
for _ in {1..80}; do
    window_id="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [ -n "$window_id" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$window_id" ]; then
    echo "chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
xdotool windowsize "$window_id" 420 360
for _ in {1..100}; do
    latest_json="$(ls -t "$snapshot_dir"/*.json 2>/dev/null | head -n 1 || true)"
    if [ -n "$latest_json" ]; then
        cols="$(sed -n 's/^  "cols": \([0-9][0-9]*\),/\1/p' "$latest_json" | head -n 1)"
        if [ -n "$cols" ] && [ "$cols" -lt 50 ]; then
            break
        fi
    fi
    sleep 0.1
done
xdotool type --window "$window_id" --delay 1 "python3 -c 'print(\"x\" * 50 + \"KEEP_TOKEN\" + \"y\" * 30)'"
xdotool key --window "$window_id" Return
for _ in {1..120}; do
    if grep -R 'KEEP_TOKEN' "$snapshot_dir"/*.txt >/dev/null 2>&1 && [ -f "$geometry_trace" ]; then
        break
    fi
    sleep 0.1
done
if ! grep -R 'KEEP_TOKEN' "$snapshot_dir"/*.txt >/dev/null 2>&1; then
    echo "reflow selection target never appeared while narrow" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
latest_txt="$(ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1)"
latest_json="$(ls -t "$snapshot_dir"/*.json 2>/dev/null | head -n 1)"
cols="$(sed -n 's/^  "cols": \([0-9][0-9]*\),/\1/p' "$latest_json" | head -n 1)"
marker_row="$(grep -n 'KEEP_TOKEN' "$latest_txt" | tail -n 1 | cut -d: -f1)"
marker_row="$((marker_row - 1))"
marker_line="$(sed -n "$((marker_row + 1))p" "$latest_txt")"
marker_col="$(awk -v line="$marker_line" -v target="$target" 'BEGIN { print index(line, target) - 1 }')"
if [ -z "$marker_col" ] || [ "$marker_col" -lt 0 ]; then
    marker_col="$(awk -v line="$marker_line" 'BEGIN { print index(line, "KEEP_TOKEN") - 1 }')"
fi
if [ -z "$marker_col" ] || [ "$marker_col" -lt 0 ] || [ -z "$cols" ]; then
    echo "could not locate KEEP_TOKEN column" >&2
    cat "$latest_txt" >&2
    exit 1
fi
canvas_x="$(sed -n 's/^canvas_x=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
canvas_y="$(sed -n 's/^canvas_y=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
cell_width="$(sed -n 's/^cell_width=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
line_height="$(sed -n 's/^line_height=\([0-9.][0-9.]*\)$/\1/p' "$geometry_trace")"
eval "$(xdotool getwindowgeometry --shell "$window_id")"
last_absolute="$((marker_col + ${#target} - 1))"
end_row="$((marker_row + (last_absolute / cols)))"
end_col="$((last_absolute % cols))"
start_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v col="$marker_col" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + ((col + 1.2) * cell) }')"
end_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v col="$end_col" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + ((col + 0.8) * cell) }')"
start_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$marker_row" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
end_y="$(awk -v top="$Y" -v canvas_y="$canvas_y" -v row="$end_row" -v line="$line_height" 'BEGIN { printf "%d", top + canvas_y + ((row + 0.5) * line) }')"
xdotool mousemove "$start_x" "$start_y"
xdotool mousedown 1
sleep 0.1
xdotool mousemove "$end_x" "$end_y"
sleep 0.1
xdotool mouseup 1
for _ in {1..100}; do
    if grep -F "primary	$target" "$clipboard_trace" >/dev/null 2>&1 && grep -R "\"selected_text\": \"$target\"" "$snapshot_dir" >/dev/null 2>&1; then
        break
    fi
    sleep 0.05
done
if ! grep -F "primary	$target" "$clipboard_trace" >/dev/null 2>&1; then
    echo "initial KEEP_TOKEN selection did not export" >&2
    cat "$clipboard_trace" >&2 || true
    cat /tmp/chelotype.log >&2 || true
    exit 1
fi
xdotool windowsize "$window_id" 1100 500
for _ in {1..120}; do
    latest_json="$(ls -t "$snapshot_dir"/*.json 2>/dev/null | head -n 1 || true)"
    latest_txt="$(ls -t "$snapshot_dir"/*.txt 2>/dev/null | head -n 1 || true)"
    if [ -n "$latest_json" ] && [ -n "$latest_txt" ]; then
        cols="$(sed -n 's/^  "cols": \([0-9][0-9]*\),/\1/p' "$latest_json" | head -n 1)"
        if [ -n "$cols" ] && [ "$cols" -ge 80 ] && grep -F "\"selected_text\": \"$target\"" "$latest_json" >/dev/null 2>&1 && grep -F "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx$target" "$latest_txt" >/dev/null 2>&1; then
            exit 0
        fi
    fi
    sleep 0.1
done
echo "selection highlight did not survive resize reflow" >&2
grep -R '"cols"' "$snapshot_dir" >&2 || true
grep -R '"selected_text"' "$snapshot_dir" >&2 || true
cat /tmp/chelotype.log >&2 || true
exit 1
"#;

    let output = Command::new("xvfb-run")
        .args([
            "-a",
            "bash",
            "--noprofile",
            "--norc",
            "-lc",
            script,
            "chelotype-gtk-reflow-selection-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
            clipboard_trace.to_str().expect("clipboard trace path utf8"),
            geometry_trace.to_str().expect("geometry trace path utf8"),
        ])
        .output()
        .expect("run gtk reflow selection e2e under xvfb");

    assert!(
        output.status.success(),
        "gtk reflow selection e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_clean_gtk_stderr(&stderr);

    let trace = read_to_string(&clipboard_trace).expect("read clipboard trace");
    assert!(
        trace
            .lines()
            .any(|line| line == "primary\tKEEP_TOKENyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy")
    );

    let json = snapshot_paths(&dir)
        .into_iter()
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .map(|path| read_to_string(path).expect("read json snapshot"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(json.contains("\"selected_text\": \"KEEP_TOKENyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy\""));

    let _ = std::fs::remove_dir_all(&dir);
}
