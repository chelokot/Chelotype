use chelotype::backend::{RenderableContentOwned, ScreenSize, TerminalBackend};
use portable_pty::CommandBuilder;
use serial_test::serial;
use std::thread::sleep;
use std::time::{Duration, Instant};

fn interactive_shell() -> CommandBuilder {
    let mut command = CommandBuilder::new("/bin/sh");
    command.arg("-i");
    command.env("PS1", "");
    command.env("ENV", "");
    command.env("BASH_ENV", "");
    command
}

fn color_output_command() -> CommandBuilder {
    let mut command = CommandBuilder::new("/bin/sh");
    command.args([
        "-lc",
        "printf '\\033[31mCHELOTYPE_RED\\033[0m\\n'; sleep 0.1",
    ]);
    command
}

fn snapshot_contains(snapshot: &RenderableContentOwned, needle: &str) -> bool {
    snapshot_text(snapshot).contains(needle)
}

fn wait_for_snapshot<F>(backend: &mut TerminalBackend, predicate: F) -> RenderableContentOwned
where
    F: Fn(&RenderableContentOwned) -> bool,
{
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if let Some(snapshot) = backend.snapshot_renderable()
            && predicate(&snapshot)
        {
            return snapshot;
        }
        sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for terminal snapshot");
}

fn snapshot_text(snapshot: &RenderableContentOwned) -> String {
    let mut text = String::new();
    for line in &snapshot.lines {
        for cell in line {
            text.push_str(&cell.text);
        }
        text.push('\n');
    }
    text
}

#[test]
#[serial]
fn backend_writes_to_single_pty_and_reads_shell_output() {
    let mut backend = TerminalBackend::spawn(interactive_shell()).expect("spawn shell");
    backend
        .write(b"printf 'CHELOTYPE_BACKEND_OK\\n'\n")
        .expect("write command");
    let snapshot = wait_for_snapshot(&mut backend, |snapshot| {
        snapshot_text(snapshot).contains("CHELOTYPE_BACKEND_OK")
    });
    assert!(snapshot_text(&snapshot).contains("CHELOTYPE_BACKEND_OK"));
    assert!(snapshot.cursor_visible);
    let _ = backend.write(b"exit\n");
}

#[test]
#[serial]
fn backend_preserves_ansi_foreground_colors_in_cells() {
    let mut backend = TerminalBackend::spawn(color_output_command()).expect("spawn color command");
    let snapshot = wait_for_snapshot(&mut backend, |snapshot| {
        snapshot_text(snapshot).contains("CHELOTYPE_RED")
    });
    let red_cell = snapshot
        .lines
        .iter()
        .flat_map(|line| line.iter())
        .find(|cell| {
            cell.text == "C" && cell.fg.as_deref().is_some_and(|color| color != "#e5e7eb")
        });
    assert!(red_cell.is_some(), "red output cell was not colorized");
}

#[test]
#[serial]
fn backend_tracks_cursor_after_shell_echo() {
    let mut backend = TerminalBackend::spawn(interactive_shell()).expect("spawn shell");
    backend.write(b"abc").expect("write input");
    let snapshot = wait_for_snapshot(&mut backend, |snapshot| {
        snapshot_text(snapshot).contains("abc")
    });
    assert!(snapshot.cursor_col >= 3);
    let _ = backend.write(b"\x15exit\n");
}

#[test]
#[serial]
fn backend_resizes_pty_and_terminal_state() {
    let mut backend = TerminalBackend::spawn(interactive_shell()).expect("spawn shell");
    let size = ScreenSize::new(100, 24).expect("valid terminal size");
    backend.resize(size).expect("resize backend");
    let pty_size = backend.pty_size().expect("pty size");
    assert_eq!(pty_size.cols, 100);
    assert_eq!(pty_size.rows, 24);
    let snapshot = wait_for_snapshot(&mut backend, |snapshot| {
        snapshot.lines.len() == 24 && snapshot.lines.iter().all(|line| line.len() == 100)
    });
    assert_eq!(snapshot.lines.len(), 24);
    assert!(snapshot.lines.iter().all(|line| line.len() == 100));
}

#[test]
#[serial]
fn backend_dirty_snapshot_only_emits_after_state_changes() {
    let mut command = CommandBuilder::new("/bin/sh");
    command.args(["-lc", "printf 'dirty-ready'; sleep 0.2"]);
    let mut backend = TerminalBackend::spawn(command).expect("spawn command");
    let _ = wait_for_snapshot(&mut backend, |snapshot| {
        snapshot_text(snapshot).contains("dirty-ready")
    });
    assert!(
        backend.snapshot_renderable_if_dirty().is_none(),
        "unchanged terminal should not emit a dirty snapshot"
    );

    backend
        .resize(ScreenSize::new(90, 20).expect("valid terminal size"))
        .expect("resize backend");
    assert!(
        backend.snapshot_renderable_if_dirty().is_some(),
        "resize should mark the render state dirty"
    );
    assert!(
        backend.snapshot_renderable_if_dirty().is_none(),
        "dirty snapshot should be consumed after emission"
    );
    let _ = backend.write(b"\x15exit\n");
}

#[test]
#[serial]
fn backend_tracks_sgr_mouse_reporting_mode() {
    let mut backend = TerminalBackend::spawn(color_output_command()).expect("spawn command");
    assert!(
        !wait_for_snapshot(&mut backend, |_| true)
            .mouse
            .sends_press_release()
    );

    let mut backend = TerminalBackend::spawn(interactive_shell()).expect("spawn shell");
    backend
        .write(b"printf '\\033[?1000h\\033[?1006h'\n")
        .expect("enable mouse reporting");
    let snapshot = wait_for_snapshot(&mut backend, |snapshot| {
        snapshot.mouse.sends_press_release()
    });
    assert!(snapshot.mouse.click);
    assert!(snapshot.mouse.sgr);
    assert!(snapshot.mouse.sends_press_release());
    assert!(!snapshot.mouse.sends_drag());
    let _ = backend.write(b"printf '\\033[?1000l\\033[?1006l'\nexit\n");
}

#[test]
#[serial]
fn backend_tracks_sgr_mouse_drag_reporting_mode() {
    let mut backend = TerminalBackend::spawn(interactive_shell()).expect("spawn shell");
    backend
        .write(b"printf '\\033[?1002h\\033[?1006h'\n")
        .expect("enable drag mouse reporting");
    let snapshot = wait_for_snapshot(&mut backend, |snapshot| snapshot.mouse.sends_drag());
    assert!(snapshot.mouse.drag);
    assert!(snapshot.mouse.sgr);
    assert!(snapshot.mouse.sends_press_release());
    assert!(snapshot.mouse.sends_drag());
    let _ = backend.write(b"printf '\\033[?1002l\\033[?1006l'\nexit\n");
}

#[test]
#[serial]
fn backend_exposes_scrollback_display_offset() {
    let mut backend = TerminalBackend::spawn(interactive_shell()).expect("spawn shell");
    backend
        .resize(ScreenSize::new(40, 6).expect("valid terminal size"))
        .expect("resize backend");
    backend
        .write(b"for n in $(seq 1 24); do printf 'SCROLL_%02d\\n' \"$n\"; done\n")
        .expect("write scrollback command");

    let bottom = wait_for_snapshot(&mut backend, |snapshot| {
        snapshot_contains(snapshot, "SCROLL_24")
    });
    assert_eq!(bottom.display_offset, 0);
    assert!(snapshot_contains(&bottom, "SCROLL_24"));

    backend.scroll_display(10).expect("scroll up");
    let scrolled = wait_for_snapshot(&mut backend, |snapshot| {
        snapshot.display_offset > 0 && snapshot_contains(snapshot, "SCROLL_")
    });
    assert!(scrolled.display_offset > 0);
    assert!(!snapshot_contains(&scrolled, "SCROLL_24"));

    backend.scroll_to_bottom().expect("scroll bottom");
    let bottom_again = wait_for_snapshot(&mut backend, |snapshot| {
        snapshot.display_offset == 0 && snapshot_contains(snapshot, "SCROLL_24")
    });
    assert_eq!(bottom_again.display_offset, 0);
    let _ = backend.write(b"exit\n");
}
