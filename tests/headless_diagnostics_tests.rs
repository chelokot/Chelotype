use serial_test::serial;
use std::fs::{read_dir, read_to_string};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn snapshot_paths(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut paths = read_dir(dir)
        .expect("snapshot files")
        .map(|entry| entry.expect("snapshot entry").path())
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn snapshot_file_with_extension(
    paths: &[std::path::PathBuf],
    extension: &str,
) -> std::path::PathBuf {
    paths
        .iter()
        .find(|path| {
            path.extension().is_some_and(|ext| ext == extension)
                && !(extension == "json"
                    && path
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().ends_with(".render.json")))
        })
        .expect("snapshot file")
        .clone()
}

fn snapshot_file_ending_with(paths: &[std::path::PathBuf], suffix: &str) -> std::path::PathBuf {
    paths
        .iter()
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(suffix))
        })
        .expect("snapshot file")
        .clone()
}

#[test]
#[serial]
fn headless_mode_writes_deterministic_snapshot_files_without_warnings() {
    let dir = std::env::temp_dir().join(format!(
        "chelotype-headless-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let output = Command::new(env!("CARGO_BIN_EXE_chelotype"))
        .env("CHELOTYPE_HEADLESS", "1")
        .env("CHELOTYPE_SNAPSHOT_DIR", &dir)
        .output()
        .expect("run headless binary");
    assert!(
        output.status.success(),
        "headless failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("WARNING"), "{stderr}");
    assert!(!stderr.contains("Gtk-WARNING"), "{stderr}");
    assert!(!stderr.contains("error:"), "{stderr}");

    let paths = snapshot_paths(&dir);
    assert!(
        paths
            .iter()
            .any(|path| path.extension().is_some_and(|ext| ext == "json"))
    );
    assert!(
        paths
            .iter()
            .any(|path| path.extension().is_some_and(|ext| ext == "html"))
    );
    assert!(
        paths
            .iter()
            .any(|path| path.extension().is_some_and(|ext| ext == "txt"))
    );
    assert!(paths.iter().any(|path| {
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().ends_with(".markup.html"))
    }));
    assert!(paths.iter().any(|path| {
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().ends_with(".render.json"))
    }));

    let text_snapshot = snapshot_file_with_extension(&paths, "txt");
    let text = read_to_string(text_snapshot).expect("read text snapshot");
    assert!(text.contains("chelotype headless snapshot"));
    assert!(text.contains("color-test"));
    assert!(text.contains("red green blue"));

    let json_snapshot = snapshot_file_with_extension(&paths, "json");
    let json = read_to_string(json_snapshot).expect("read json snapshot");
    assert!(json.contains("\"rows\""));
    assert!(json.contains("\"cols\""));
    assert!(json.contains("\"cursor_line\""));
    assert!(json.contains("\"display_offset\""));
    assert!(json.contains("\"mouse\""));
    assert!(json.contains("\"wrapped\""));
    assert!(json.contains("\"wrap_continuation\""));
    assert!(json.contains("\"semantic_prompt\""));
    assert!(json.contains("\"cells\""));
    assert!(json.contains("\"text\""));
    assert!(json.contains("chelotype headless snapshot"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn headless_mode_exports_selection_render_dump() {
    let dir = std::env::temp_dir().join(format!(
        "chelotype-headless-selection-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let output = Command::new(env!("CARGO_BIN_EXE_chelotype"))
        .env("CHELOTYPE_HEADLESS", "1")
        .env("CHELOTYPE_SNAPSHOT_DIR", &dir)
        .env("CHELOTYPE_HEADLESS_ACTIONS", "printf 'selection-export\\n'")
        .env("CHELOTYPE_HEADLESS_EXPECT", "selection-export")
        .env("CHELOTYPE_HEADLESS_SELECTION", "0,0:0,2")
        .output()
        .expect("run selection headless binary");
    assert!(
        output.status.success(),
        "headless failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("WARNING"), "{stderr}");
    assert!(!stderr.contains("Gtk-WARNING"), "{stderr}");
    assert!(!stderr.contains("error:"), "{stderr}");

    let paths = snapshot_paths(&dir);
    let render_dump = paths
        .iter()
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".render.json"))
        })
        .expect("render dump");
    let json = read_to_string(render_dump).expect("read render dump");
    assert!(json.contains("\"selection\""));
    assert!(json.contains("\"selected_text\""));
    assert!(json.contains("\"lines\""));
    assert!(json.contains("\"runs\""));
    assert!(json.contains("\"start_column\""));
    assert!(json.contains("\"selected\": true"));
    assert!(json.contains("\"region\""));
    assert!(json.contains("\"end\""));

    let markup_dump = paths
        .iter()
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".markup.html"))
        })
        .expect("markup dump");
    let markup = read_to_string(markup_dump).expect("read markup dump");
    assert!(markup.contains("background=\"#264f78\""));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn headless_mode_replays_scripted_interaction_actions() {
    let dir = std::env::temp_dir().join(format!(
        "chelotype-headless-scripted-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let output = Command::new(env!("CARGO_BIN_EXE_chelotype"))
        .env("CHELOTYPE_HEADLESS", "1")
        .env("CHELOTYPE_SNAPSHOT_DIR", &dir)
        .env(
            "CHELOTYPE_HEADLESS_ACTIONS",
            "printf 'typed-before typed-after\\n'\n|printf 'cursor-left-right\\n'\n",
        )
        .env(
            "CHELOTYPE_HEADLESS_EXPECT",
            "typed-before typed-after|cursor-left-right",
        )
        .output()
        .expect("run scripted headless binary");
    assert!(
        output.status.success(),
        "headless failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("WARNING"), "{stderr}");
    assert!(!stderr.contains("Gtk-WARNING"), "{stderr}");
    assert!(!stderr.contains("error:"), "{stderr}");

    let text_snapshot = snapshot_file_with_extension(&snapshot_paths(&dir), "txt");
    let text = read_to_string(text_snapshot).expect("read text snapshot");
    assert!(text.contains("typed-before typed-after"));
    assert!(text.contains("cursor-left-right"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn headless_mode_replays_keyboard_events_through_input_mapping() {
    let dir = std::env::temp_dir().join(format!(
        "chelotype-headless-keyboard-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let target = dir.join("keyboard-edit.txt");
    let output = Command::new(env!("CARGO_BIN_EXE_chelotype"))
        .env("CHELOTYPE_HEADLESS", "1")
        .env("CHELOTYPE_SNAPSHOT_DIR", &dir)
        .env(
            "CHELOTYPE_HEADLESS_EVENTS",
            format!(
                "raw:stty erase '^?'\\n|raw:cat > {}\\n|text:abc|key:Backspace|text:d|key:Enter|key:Ctrl+d|raw:printf 'EDITED='; cat {}; echo\\n",
                target.display(),
                target.display()
            ),
        )
        .env("CHELOTYPE_HEADLESS_EXPECT", "EDITED=abd")
        .output()
        .expect("run keyboard headless binary");
    assert!(
        output.status.success(),
        "headless failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("WARNING"), "{stderr}");
    assert!(!stderr.contains("Gtk-WARNING"), "{stderr}");
    assert!(!stderr.contains("error:"), "{stderr}");

    let text_snapshot = snapshot_file_with_extension(&snapshot_paths(&dir), "txt");
    let text = read_to_string(text_snapshot).expect("read text snapshot");
    assert!(text.contains("EDITED=abd"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn headless_mode_replays_arrow_key_events_through_input_mapping() {
    let dir = std::env::temp_dir().join(format!(
        "chelotype-headless-arrows-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let target = dir.join("arrow-keys.bin");
    let output = Command::new(env!("CARGO_BIN_EXE_chelotype"))
        .env("CHELOTYPE_HEADLESS", "1")
        .env("CHELOTYPE_SNAPSHOT_DIR", &dir)
        .env(
            "CHELOTYPE_HEADLESS_EVENTS",
            format!(
                "raw:cat > {}\\n|key:Left|key:Right|key:Enter|key:Ctrl+d|raw:printf 'ARROWS='; od -An -tx1 {}; echo\\n",
                target.display(),
                target.display()
            ),
        )
        .env("CHELOTYPE_HEADLESS_EXPECT", "ARROWS= 1b 5b 44 1b 5b 43")
        .output()
        .expect("run arrow headless binary");
    assert!(
        output.status.success(),
        "headless failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("WARNING"), "{stderr}");
    assert!(!stderr.contains("Gtk-WARNING"), "{stderr}");
    assert!(!stderr.contains("error:"), "{stderr}");

    let text_snapshot = snapshot_file_with_extension(&snapshot_paths(&dir), "txt");
    let text = read_to_string(text_snapshot).expect("read text snapshot");
    assert!(text.contains("ARROWS= 1b 5b 44 1b 5b 43"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn headless_mode_replays_mouse_drag_selection_events() {
    let dir = std::env::temp_dir().join(format!(
        "chelotype-headless-mouse-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let output = Command::new(env!("CARGO_BIN_EXE_chelotype"))
        .env("CHELOTYPE_HEADLESS", "1")
        .env("CHELOTYPE_SNAPSHOT_DIR", &dir)
        .env(
            "CHELOTYPE_HEADLESS_EVENTS",
            "raw:printf 'mouse-selection\\n'\\n|mouse:press:left:0,0|mouse:drag:5,0|mouse:release:5,0",
        )
        .env("CHELOTYPE_HEADLESS_EXPECT", "mouse-selection")
        .output()
        .expect("run mouse headless binary");
    assert!(
        output.status.success(),
        "headless failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("WARNING"), "{stderr}");
    assert!(!stderr.contains("Gtk-WARNING"), "{stderr}");
    assert!(!stderr.contains("error:"), "{stderr}");

    let paths = snapshot_paths(&dir);
    let render_dump = snapshot_file_ending_with(&paths, ".render.json");
    let json = read_to_string(render_dump).expect("read render dump");
    assert!(json.contains("\"selection\""));
    assert!(json.contains("\"selected_text\""));
    assert!(json.contains("\"column\": 6"));

    let markup_dump = snapshot_file_ending_with(&paths, ".markup.html");
    let markup = read_to_string(markup_dump).expect("read markup dump");
    assert!(markup.contains("background=\"#264f78\""));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn headless_mode_replays_mouse_click_as_shell_cursor_movement() {
    let dir = std::env::temp_dir().join(format!(
        "chelotype-headless-mouse-click-cursor-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let output = Command::new(env!("CARGO_BIN_EXE_chelotype"))
        .env("CHELOTYPE_HEADLESS", "1")
        .env("CHELOTYPE_SNAPSHOT_DIR", &dir)
        .env(
            "CHELOTYPE_HEADLESS_EVENTS",
            "text:abcdef|wait:abcdef|mouse:press:left:2,1|mouse:release:2,1|text:Z|key:Enter",
        )
        .env("CHELOTYPE_HEADLESS_EXPECT", "abZcdef")
        .env("CHELOTYPE_HEADLESS_STEP_MS", "120")
        .output()
        .expect("run mouse click cursor headless binary");
    assert!(
        output.status.success(),
        "headless failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("WARNING"), "{stderr}");
    assert!(!stderr.contains("Gtk-WARNING"), "{stderr}");
    assert!(!stderr.contains("error:"), "{stderr}");

    let text_snapshot = snapshot_file_with_extension(&snapshot_paths(&dir), "txt");
    let text = read_to_string(text_snapshot).expect("read text snapshot");
    assert!(text.contains("abZcdef"), "{text}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn headless_mode_replays_resize_event() {
    let dir = std::env::temp_dir().join(format!(
        "chelotype-headless-resize-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let output = Command::new(env!("CARGO_BIN_EXE_chelotype"))
        .env("CHELOTYPE_HEADLESS", "1")
        .env("CHELOTYPE_SNAPSHOT_DIR", &dir)
        .env("CHELOTYPE_HEADLESS_EVENTS", "resize:40x6")
        .env("CHELOTYPE_HEADLESS_EXPECT", "")
        .output()
        .expect("run resize headless binary");
    assert!(
        output.status.success(),
        "headless failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("WARNING"), "{stderr}");
    assert!(!stderr.contains("Gtk-WARNING"), "{stderr}");
    assert!(!stderr.contains("error:"), "{stderr}");

    let json_snapshot = snapshot_file_with_extension(&snapshot_paths(&dir), "json");
    let json: serde_json::Value =
        serde_json::from_str(&read_to_string(json_snapshot).expect("read json snapshot"))
            .expect("parse json snapshot");
    assert_eq!(json["cols"], 40);
    assert_eq!(json["rows"], 6);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn headless_mode_replays_scroll_event() {
    let dir = std::env::temp_dir().join(format!(
        "chelotype-headless-scroll-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let output = Command::new(env!("CARGO_BIN_EXE_chelotype"))
        .env("CHELOTYPE_HEADLESS", "1")
        .env("CHELOTYPE_SNAPSHOT_DIR", &dir)
        .env(
            "CHELOTYPE_HEADLESS_EVENTS",
            "resize:40x6|raw:for n in $(seq 1 24); do echo HEADLESS_SCROLL_$n; done\\n|wait:HEADLESS_SCROLL_24|scroll:8",
        )
        .env("CHELOTYPE_HEADLESS_EXPECT", "")
        .output()
        .expect("run scroll headless binary");
    assert!(
        output.status.success(),
        "headless failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("WARNING"), "{stderr}");
    assert!(!stderr.contains("Gtk-WARNING"), "{stderr}");
    assert!(!stderr.contains("error:"), "{stderr}");

    let json_snapshot = snapshot_file_with_extension(&snapshot_paths(&dir), "json");
    let json: serde_json::Value =
        serde_json::from_str(&read_to_string(json_snapshot).expect("read json snapshot"))
            .expect("parse json snapshot");
    assert!(
        json["display_offset"].as_u64().expect("display offset") > 0,
        "{json}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn headless_mode_replays_backspace_and_arrow_cursor_actions() {
    let dir = std::env::temp_dir().join(format!(
        "chelotype-headless-editing-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let output = Command::new(env!("CARGO_BIN_EXE_chelotype"))
        .env("CHELOTYPE_HEADLESS", "1")
        .env("CHELOTYPE_SNAPSHOT_DIR", &dir)
        .env("CHELOTYPE_HEADLESS_ACTIONS", "printf 'abc\\nEDIT_OK\\n'")
        .env("CHELOTYPE_HEADLESS_EXPECT", "abc|EDIT_OK")
        .output()
        .expect("run editing headless binary");
    assert!(
        output.status.success(),
        "headless failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("WARNING"), "{stderr}");
    assert!(!stderr.contains("Gtk-WARNING"), "{stderr}");
    assert!(!stderr.contains("error:"), "{stderr}");

    let text_snapshot = snapshot_file_with_extension(&snapshot_paths(&dir), "txt");
    let text = read_to_string(text_snapshot).expect("read text snapshot");
    assert!(text.contains("abc"));
    assert!(text.contains("EDIT_OK"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[serial]
fn headless_mode_exports_unicode_and_style_cells_in_json() {
    let dir = std::env::temp_dir().join(format!(
        "chelotype-headless-style-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let output = Command::new(env!("CARGO_BIN_EXE_chelotype"))
        .env("CHELOTYPE_HEADLESS", "1")
        .env("CHELOTYPE_SNAPSHOT_DIR", &dir)
        .env(
            "CHELOTYPE_HEADLESS_ACTIONS",
            "printf \\x27UNICODE Я λ 中\\x5cn\\x27\\n|printf 'COMBO é WIDE 中\\n'\n|printf \\x27\\x5c033[1mBOLD\\x5c033[0m \\x5c033[3mITALIC\\x5c033[0m \\x5c033[4mUNDER\\x5c033[0m \\x5c033[35mMAGENTA\\x5c033[0m\\x5cn\\x27\\n",
        )
        .env(
            "CHELOTYPE_HEADLESS_EXPECT",
            "UNICODE Я λ 中|COMBO é WIDE 中|BOLD ITALIC UNDER MAGENTA",
        )
        .output()
        .expect("run style headless binary");
    assert!(
        output.status.success(),
        "headless failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("WARNING"), "{stderr}");
    assert!(!stderr.contains("Gtk-WARNING"), "{stderr}");
    assert!(!stderr.contains("error:"), "{stderr}");

    let paths = snapshot_paths(&dir);
    let json_snapshot = snapshot_file_with_extension(&paths, "json");
    let json = read_to_string(json_snapshot).expect("read json snapshot");
    assert!(json.contains("UNICODE Я λ 中"));
    assert!(json.contains("COMBO é WIDE 中"));
    assert!(json.contains("\"text\": \"é\""));
    assert!(json.contains("\"wide\": true"));
    assert!(json.contains("\"wide_spacer\": true"));
    assert!(json.contains("\"bold\": true"));
    assert!(json.contains("\"italic\": true"));
    assert!(json.contains("\"underline\": true"));
    assert!(json.contains("MAGENTA"));

    let _ = std::fs::remove_dir_all(&dir);
}
