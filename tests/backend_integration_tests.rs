use chelotype::backend::{RenderableContentOwned, ScreenSize, TerminalBackend};
use chelotype::terminal_palette::default_terminal_palette;
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

fn fish_command() -> Option<CommandBuilder> {
    ["/usr/bin/fish", "/bin/fish"]
        .into_iter()
        .find(|path| std::path::Path::new(path).is_file())
        .map(CommandBuilder::new)
}

fn chelotype_fish_command() -> Option<CommandBuilder> {
    ["/usr/bin/fish", "/bin/fish"]
        .into_iter()
        .find(|path| std::path::Path::new(path).is_file())
        .map(|path| {
            let previous_shell = std::env::var_os("CHELOTYPE_SHELL");
            unsafe {
                std::env::set_var("CHELOTYPE_SHELL", path);
            }
            let command = chelotype::shell::default_shell_command();
            if let Some(previous_shell) = previous_shell {
                unsafe {
                    std::env::set_var("CHELOTYPE_SHELL", previous_shell);
                }
            } else {
                unsafe {
                    std::env::remove_var("CHELOTYPE_SHELL");
                }
            }
            command
        })
}

fn snapshot_contains(snapshot: &RenderableContentOwned, needle: &str) -> bool {
    snapshot_text(snapshot).contains(needle)
}

fn wait_for_snapshot<F>(backend: &mut TerminalBackend, predicate: F) -> RenderableContentOwned
where
    F: Fn(&RenderableContentOwned) -> bool,
{
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut last_snapshot = None;
    while Instant::now() < deadline {
        if let Some(snapshot) = backend.snapshot_renderable() {
            if predicate(&snapshot) {
                return snapshot;
            }
            last_snapshot = Some((
                snapshot.cursor_line,
                snapshot.cursor_col,
                snapshot_text(&snapshot),
            ));
        }
        sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for terminal snapshot; last={last_snapshot:?}");
}

fn snapshot_text(snapshot: &RenderableContentOwned) -> String {
    let mut text = String::new();
    for line in &snapshot.lines {
        for cell in line.iter() {
            text.push_str(&cell.text);
        }
        text.push('\n');
    }
    text
}

fn wait_for_input_cursor_bridge(backend: &mut TerminalBackend) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if backend
            .input_cursor_bridge_ready()
            .expect("input cursor bridge readiness")
        {
            return;
        }
        sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for input cursor bridge");
}

fn visible_nonblank_lines(snapshot: &RenderableContentOwned) -> Vec<String> {
    snapshot
        .lines
        .iter()
        .map(|line| {
            line.iter()
                .map(|cell| cell.text.as_ref())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .filter(|line| !line.is_empty())
        .collect()
}

#[test]
#[serial]
fn backend_starts_fish_and_handles_startup_terminal_queries() {
    let Some(command) = fish_command() else {
        eprintln!("skipping fish backend test because fish is not installed");
        return;
    };

    let mut backend = TerminalBackend::spawn(command).expect("spawn fish");
    let _ = wait_for_snapshot(&mut backend, |snapshot| {
        snapshot_contains(snapshot, "Welcome to fish") || snapshot.cursor_visible
    });
    backend
        .write(b"printf 'FISH_DEFAULT_OK\\n'\n")
        .expect("write fish command");
    let snapshot = wait_for_snapshot(&mut backend, |snapshot| {
        snapshot_contains(snapshot, "FISH_DEFAULT_OK")
    });

    assert!(snapshot_contains(&snapshot, "FISH_DEFAULT_OK"));
    let _ = backend.write(b"exit\n");
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
            cell.text == "C"
                && cell
                    .fg
                    .as_deref()
                    .is_some_and(|color| color != default_terminal_palette().foreground)
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
fn backend_fish_cursor_target_bridge_moves_commandline_cursor_directly() {
    let Some(command) = chelotype_fish_command() else {
        eprintln!("skipping fish cursor target bridge test because fish is not installed");
        return;
    };

    let mut backend = TerminalBackend::spawn(command).expect("spawn fish");
    let _ = wait_for_snapshot(&mut backend, |snapshot| snapshot.cursor_visible);
    wait_for_input_cursor_bridge(&mut backend);
    backend.write(b"abcde").expect("write fish input");
    let _ = wait_for_snapshot(&mut backend, |snapshot| {
        snapshot_text(snapshot).contains("abcde")
    });
    assert!(
        backend
            .write_input_cursor_target(2)
            .expect("write cursor target"),
        "fish backend should expose cursor target bridge"
    );
    let snapshot = wait_for_snapshot(&mut backend, |snapshot| {
        snapshot_text(snapshot).contains("abcde") && snapshot.cursor_col == 4
    });

    assert_eq!(snapshot.cursor_col, 4);
    let _ = backend.write(b"\x15exit\n");
}

#[test]
#[serial]
fn backend_fish_cursor_target_bridge_replaces_long_wrapped_input() {
    let Some(command) = chelotype_fish_command() else {
        eprintln!("skipping fish cursor target bridge test because fish is not installed");
        return;
    };

    let mut backend =
        TerminalBackend::spawn_with_size(command, ScreenSize::new(80, 24).expect("valid size"))
            .expect("spawn fish");
    let _ = wait_for_snapshot(&mut backend, |snapshot| snapshot.cursor_visible);
    wait_for_input_cursor_bridge(&mut backend);
    let input = "a".repeat(1024);
    backend
        .write(input.as_bytes())
        .expect("write long fish input");
    let _ = wait_for_snapshot(&mut backend, |snapshot| {
        snapshot_text(snapshot).contains("aaaaaaaaaaaaaaaa")
    });
    assert!(
        backend
            .write_input_cursor_target(0)
            .expect("write cursor target"),
        "fish backend should expose cursor target bridge"
    );
    let mut replacement = Vec::with_capacity(1024 * 4 + 7);
    for _ in 0..1024 {
        replacement.extend_from_slice(b"\x1b[3~");
    }
    replacement.extend_from_slice(b"PASTE");
    backend.write(&replacement).expect("replace long input");

    let snapshot = wait_for_snapshot(&mut backend, |snapshot| {
        let text = snapshot_text(snapshot);
        text.contains("PASTE") && !text.contains("aaaaaaaaaaaaaaaa")
    });

    assert!(snapshot_text(&snapshot).contains("PASTE"));
    let _ = backend.write(b"\x15exit\n");
}

#[test]
#[serial]
fn backend_fish_replace_range_bridge_preserves_empty_and_queued_whitespace_replacements() {
    let Some(command) = chelotype_fish_command() else {
        eprintln!("skipping fish replace range bridge test because fish is not installed");
        return;
    };

    let mut backend = TerminalBackend::spawn(command).expect("spawn fish");
    let _ = wait_for_snapshot(&mut backend, |snapshot| snapshot.cursor_visible);
    wait_for_input_cursor_bridge(&mut backend);
    backend.write(b"abcdef").expect("write fish input");
    let _ = wait_for_snapshot(&mut backend, |snapshot| {
        snapshot_text(snapshot).contains("abcdef")
    });

    assert!(
        backend
            .write_input_replace_range(2..4, "")
            .expect("delete fish input range")
    );
    let snapshot = wait_for_snapshot(&mut backend, |snapshot| {
        visible_nonblank_lines(snapshot)
            .iter()
            .any(|line| line.ends_with("abef"))
    });
    assert!(
        visible_nonblank_lines(&snapshot)
            .iter()
            .any(|line| line.ends_with("abef"))
    );

    assert!(
        backend
            .write_input_replace_range(2..2, "X\n")
            .expect("insert multiline fish input range")
    );
    let snapshot = wait_for_snapshot(&mut backend, |snapshot| {
        let lines = visible_nonblank_lines(snapshot);
        lines.iter().any(|line| line.ends_with("abX"))
            && lines.iter().any(|line| line.trim_start() == "ef")
    });
    let lines = visible_nonblank_lines(&snapshot);
    assert!(lines.iter().any(|line| line.ends_with("abX")));
    assert!(lines.iter().any(|line| line.trim_start() == "ef"));

    assert!(
        backend
            .write_input_replace_range(0..6, "abcdef")
            .expect("reset fish input")
    );
    let _ = wait_for_snapshot(&mut backend, |snapshot| {
        visible_nonblank_lines(snapshot)
            .iter()
            .any(|line| line.ends_with("abcdef"))
    });
    assert!(
        backend
            .write_input_replace_range(0..1, "X")
            .expect("queue first fish replacement")
    );
    assert!(
        backend
            .write_input_replace_range(1..2, "Y")
            .expect("queue second fish replacement")
    );
    let snapshot = wait_for_snapshot(&mut backend, |snapshot| {
        visible_nonblank_lines(snapshot)
            .iter()
            .any(|line| line.ends_with("XYcdef"))
    });
    assert!(
        visible_nonblank_lines(&snapshot)
            .iter()
            .any(|line| line.ends_with("XYcdef"))
    );

    backend
        .write(chelotype::shell::INPUT_UNDO_SEQUENCE)
        .expect("undo queued replacement");
    let _ = wait_for_snapshot(&mut backend, |snapshot| {
        visible_nonblank_lines(snapshot)
            .iter()
            .any(|line| line.ends_with("Xbcdef"))
    });
    backend
        .write(chelotype::shell::INPUT_REDO_SEQUENCE)
        .expect("redo queued replacement");
    let snapshot = wait_for_snapshot(&mut backend, |snapshot| {
        visible_nonblank_lines(snapshot)
            .iter()
            .any(|line| line.ends_with("XYcdef"))
    });
    assert!(
        visible_nonblank_lines(&snapshot)
            .iter()
            .any(|line| line.ends_with("XYcdef"))
    );
    let _ = backend.write(b"\x15exit\n");
}

#[test]
#[serial]
fn backend_fish_accepts_bracketed_paste() {
    let Some(command) = chelotype_fish_command() else {
        eprintln!("skipping fish bracketed paste test because fish is not installed");
        return;
    };

    let mut backend = TerminalBackend::spawn(command).expect("spawn fish");
    let _ = wait_for_snapshot(&mut backend, |snapshot| snapshot.cursor_visible);

    backend
        .write(b"\x1b[200~printf 'BRACKETED_PASTE_OK\\n'\x1b[201~\n")
        .expect("write bracketed paste command");
    let snapshot = wait_for_snapshot(&mut backend, |snapshot| {
        snapshot_contains(snapshot, "BRACKETED_PASTE_OK")
    });

    assert!(snapshot_contains(&snapshot, "BRACKETED_PASTE_OK"));
    let _ = backend.write(b"exit\n");
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
    assert!(
        !backend.refresh_dirty(),
        "unchanged terminal should not stay dirty after a clean snapshot"
    );

    backend
        .resize(ScreenSize::new(90, 20).expect("valid terminal size"))
        .expect("resize backend");
    assert!(
        backend.refresh_dirty(),
        "resize should be observable before taking a new snapshot"
    );
    assert!(
        backend.snapshot_renderable_if_dirty().is_some(),
        "resize should mark the render state dirty"
    );
    assert!(
        backend.snapshot_renderable_if_dirty().is_none(),
        "dirty snapshot should be consumed after emission"
    );
    assert!(
        !backend.refresh_dirty(),
        "dirty probe should be clean after the dirty snapshot is consumed"
    );
    let _ = backend.write(b"\x15exit\n");
}

#[test]
#[serial]
fn backend_shutdown_stops_a_backpressured_pty_reader() {
    let mut command = CommandBuilder::new("/bin/sh");
    command.args(["-lc", "trap '' HUP; yes CHELOTYPE_SHUTDOWN"]);
    let mut backend = TerminalBackend::spawn(command).expect("spawn output command");
    sleep(Duration::from_millis(100));

    let started = Instant::now();
    backend.shutdown();

    assert!(
        started.elapsed() < Duration::from_secs(1),
        "shutdown should not wait for a full PTY channel"
    );
    assert_eq!(
        backend
            .write(b"after shutdown")
            .expect_err("closed writer")
            .kind(),
        std::io::ErrorKind::BrokenPipe
    );
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
    assert!(
        backend.can_scroll_display(10).expect("can scroll up"),
        "scrollback should be reachable from bottom"
    );
    assert!(
        backend.available_scroll_lines(10).expect("up distance") > 0,
        "scrollback should report available lines above bottom"
    );
    assert_eq!(
        backend
            .available_scroll_lines(-10)
            .expect("bottom distance"),
        0
    );
    assert!(
        !backend
            .can_scroll_display(-10)
            .expect("cannot scroll below bottom"),
        "bottom should not allow scrolling further down"
    );
    assert!(
        !backend
            .scroll_display_changed(-10)
            .expect("scroll bottom limit"),
        "scrolling down at bottom should not report viewport movement"
    );

    assert!(
        backend.scroll_display_changed(10).expect("scroll up"),
        "scrolling up into scrollback should report viewport movement"
    );
    let scrolled = wait_for_snapshot(&mut backend, |snapshot| {
        snapshot.display_offset > 0 && snapshot_contains(snapshot, "SCROLL_")
    });
    assert!(scrolled.display_offset > 0);
    assert!(
        backend
            .can_scroll_display(-10)
            .expect("can scroll back toward bottom"),
        "scrolled viewport should be able to return toward bottom"
    );
    assert!(
        backend
            .available_scroll_lines(-10)
            .expect("distance back toward bottom")
            > 0,
        "scrolled viewport should report available lines back toward bottom"
    );
    assert!(!snapshot_contains(&scrolled, "SCROLL_24"));

    backend.scroll_to_bottom().expect("scroll bottom");
    let bottom_again = wait_for_snapshot(&mut backend, |snapshot| {
        snapshot.display_offset == 0 && snapshot_contains(snapshot, "SCROLL_24")
    });
    assert_eq!(bottom_again.display_offset, 0);
    let _ = backend.write(b"exit\n");
}

#[test]
#[serial]
fn backend_reflows_wrapped_scrollback_after_resize() {
    let mut backend = TerminalBackend::spawn(interactive_shell()).expect("spawn shell");
    backend
        .resize(ScreenSize::new(18, 8).expect("valid terminal size"))
        .expect("resize backend narrow");
    backend
        .write(b"printf 'REFLOW_123456789_abcdefghijklmnopqrstuvwxyz\\n'\n")
        .expect("write reflow command");

    let narrow = wait_for_snapshot(&mut backend, |snapshot| {
        snapshot_contains(snapshot, "REFLOW_123456789")
            && snapshot
                .line_metadata
                .iter()
                .any(|metadata| metadata.wrapped || metadata.wrap_continuation)
    });
    let narrow_lines = visible_nonblank_lines(&narrow);
    let narrow_reflow_rows = narrow_lines
        .iter()
        .filter(|line| {
            line.contains("REFLOW_")
                || line.contains("abcdefghijklmnopqrstuvwxyz")
                || line.contains("_abcdef")
        })
        .count();
    assert!(
        narrow_reflow_rows >= 2,
        "narrow output should visibly wrap: {narrow_lines:?}"
    );

    backend
        .resize(ScreenSize::new(42, 8).expect("valid terminal size"))
        .expect("resize backend wide");
    let wide = wait_for_snapshot(&mut backend, |snapshot| {
        snapshot.lines.len() == 8
            && snapshot.lines.iter().all(|line| line.len() == 42)
            && snapshot_contains(snapshot, "REFLOW_123456789_abcdefghijklmnopqrstu")
    });
    let wide_lines = visible_nonblank_lines(&wide);
    assert!(
        wide_lines
            .iter()
            .any(|line| line == "REFLOW_123456789_abcdefghijklmnopqrstuvwxy"),
        "wide resize should reflow wrapped output into a wider row: {wide_lines:?}"
    );
    assert!(
        wide_lines.iter().any(|line| line == "z"),
        "wide resize should preserve the wrapped tail after reflow: {wide_lines:?}"
    );
    let _ = backend.write(b"exit\n");
}
