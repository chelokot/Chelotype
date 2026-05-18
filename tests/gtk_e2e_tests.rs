use serial_test::serial;
use std::fs::{read_dir, read_to_string};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

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
            "printf '\\033[31mGTK_RED_STYLE\\033[0m\\n'\n",
        )
        .env("CHELOTYPE_UI_E2E_EXPECT", "GTK_RED_STYLE")
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

    let script = r#"
set -euo pipefail
bin="$1"
snapshot_dir="$2"
rm -f /tmp/chelotype.log
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 CHELOTYPE_DEBUG=1 CHELOTYPE_SNAPSHOT=1 CHELOTYPE_SNAPSHOT_DIR="$snapshot_dir" "$bin" &
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
start_x=20
start_y=115
end_x=170
end_y="$start_y"
xdotool mousemove "$start_x" "$start_y"
xdotool mousedown 1
xdotool mousemove "$end_x" "$end_y"
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
