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
    echo "Chelotype window did not appear" >&2
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
    echo "Chelotype window did not appear" >&2
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
    echo "Chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "printf '\033[38;2;255;0;0mA\033[0m \033[38;2;0;255;0mB\033[0m\n'; printf 'GRID_SPACE_DONE\n'"
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
    echo "Chelotype window did not appear" >&2
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
    echo "Chelotype window did not appear" >&2
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
    echo "Chelotype window did not appear" >&2
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
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_PERF_TRACE="$perf_trace" SHELL=/usr/bin/zsh ZDOTDIR="$zdot" HOME="$zdot" "$bin" &
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
    echo "Chelotype window did not appear" >&2
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
    let render_p95 = percentile_duration(render.clone(), 95);
    let render_p99 = percentile_duration(render, 99);
    let paint_p95 = percentile_duration(paint.clone(), 95);
    let paint_p99 = percentile_duration(paint, 99);
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
    echo "Chelotype window did not appear" >&2
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
    echo "Chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "printf 'MOUSE_SELECT_OK\n'"
xdotool key --window "$window_id" Return
for _ in {1..100}; do
    if grep -R 'MOUSE_SELECT_OK' "$snapshot_dir" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if ! grep -R 'MOUSE_SELECT_OK' "$snapshot_dir" >/dev/null 2>&1; then
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
        trace.contains("primary\tMOUSE_SELECT_OK"),
        "primary selection was not exported: {trace}"
    );

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
    echo "Chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "printf 'DRAG_RELEASE_STABLE\n'"
xdotool key --window "$window_id" Return
for _ in {1..100}; do
    if grep -R 'DRAG_RELEASE_STABLE' "$snapshot_dir" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if ! grep -R 'DRAG_RELEASE_STABLE' "$snapshot_dir" >/dev/null 2>&1; then
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
        trace.contains("clipboard\tDRAG_RELEASE_STABLE"),
        "selection was not stable after release: {trace}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn gtk_e2e_copies_selection_to_clipboard_with_ctrl_shift_c_under_xvfb() {
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
    echo "Chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
xdotool type --window "$window_id" --delay 2 "printf 'CLIPBOARD_COPY_OK\n'"
xdotool key --window "$window_id" Return
for _ in {1..100}; do
    if grep -R 'CLIPBOARD_COPY_OK' "$snapshot_dir" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if ! grep -R 'CLIPBOARD_COPY_OK' "$snapshot_dir" >/dev/null 2>&1; then
    echo "text for clipboard copy never appeared" >&2
    find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
    exit 1
fi
latest_txt="$(ls "$snapshot_dir"/*.txt 2>/dev/null | tail -n 1)"
marker_row="$(grep -n '^CLIPBOARD_COPY_OK' "$latest_txt" | tail -n 1 | cut -d: -f1)"
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
    if grep -F 'primary	CLIPBOARD_COPY_OK' "$clipboard_trace" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
xdotool key --window "$window_id" ctrl+shift+c
for _ in {1..100}; do
    if grep -F 'clipboard	CLIPBOARD_COPY_OK' "$clipboard_trace" >/dev/null 2>&1; then
        exit 0
    fi
    sleep 0.1
done
echo "Ctrl+Shift+C did not export selected text to clipboard" >&2
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
        trace.contains("clipboard\tCLIPBOARD_COPY_OK"),
        "clipboard was not exported: {trace}"
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
    echo "Chelotype window did not appear" >&2
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
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" CHELOTYPE_GEOMETRY_TRACE="$geometry_trace" SHELL=/usr/bin/zsh ZDOTDIR="$zdot" HOME="$zdot" "$bin" &
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
    echo "Chelotype window did not appear" >&2
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
target_x="$(awk -v left="$X" -v canvas_x="$canvas_x" -v cell="$cell_width" 'BEGIN { printf "%d", left + canvas_x + (3.4 * cell) }')"
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
    echo "Chelotype window did not appear" >&2
    exit 1
fi
xdotool windowfocus "$window_id" || true
sleep 0.2
cmd="stty raw -echo; printf '\033[?1000h\033[?1006h'; dd bs=1 count=9 2>/dev/null | od -An -tx1; printf '\033[?1006l\033[?1000l'; stty sane; printf '\nMOUSE_REPORT_DONE\n'"
xdotool type --window "$window_id" --delay 1 "$cmd"
xdotool key --window "$window_id" Return
sleep 1
xdotool mousemove 20 115
xdotool click 1
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
echo "shell did not receive SGR mouse press bytes" >&2
find "$snapshot_dir" -maxdepth 1 -type f -print >&2 || true
for file in "$snapshot_dir"/*.txt; do
    [ -f "$file" ] || continue
    echo "===$file" >&2
    sed -n '1,12p' "$file" >&2
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
            "chelotype-gtk-mouse-report-e2e",
            env!("CARGO_BIN_EXE_chelotype"),
            dir.to_str().expect("snapshot dir utf8"),
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

    let text = snapshot_paths(&dir)
        .into_iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "txt"))
        .map(|path| read_to_string(path).expect("read text snapshot"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("MOUSE_REPORT_DONE"));
    assert!(text.contains("1b 5b 3c 30 3b"));
    assert!(text.contains("4d"));

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
    echo "Chelotype window did not appear" >&2
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
