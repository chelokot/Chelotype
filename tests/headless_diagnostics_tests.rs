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
        .find(|path| path.extension().is_some_and(|ext| ext == extension))
        .expect("snapshot file")
        .clone()
}

#[test]
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
    assert!(json.contains("\"text\""));
    assert!(json.contains("chelotype headless snapshot"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
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
            "printf \\x27UNICODE Я λ 中\\x5cn\\x27\\n|printf \\x27\\x5c033[1mBOLD\\x5c033[0m \\x5c033[3mITALIC\\x5c033[0m \\x5c033[4mUNDER\\x5c033[0m \\x5c033[35mMAGENTA\\x5c033[0m\\x5cn\\x27\\n",
        )
        .env("CHELOTYPE_HEADLESS_EXPECT", "UNICODE Я λ 中|BOLD ITALIC UNDER MAGENTA")
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
    assert!(json.contains("\"bold\": true"));
    assert!(json.contains("\"italic\": true"));
    assert!(json.contains("\"underline\": true"));
    assert!(json.contains("MAGENTA"));

    let _ = std::fs::remove_dir_all(&dir);
}
