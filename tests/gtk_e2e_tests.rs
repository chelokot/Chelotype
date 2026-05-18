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
    assert!(!stderr.contains("Gtk-WARNING"), "{stderr}");
    assert!(!stderr.contains("panic"), "{stderr}");
    assert!(!stderr.contains("error:"), "{stderr}");

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
