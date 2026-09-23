use chelotype::backend::{RenderableContentOwned, ScreenSize, TerminalBackend};
use chelotype::cell_text::lines_to_text;
use portable_pty::CommandBuilder;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct ApplicationFixture {
    directory: PathBuf,
}

impl ApplicationFixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "chelotype-apps-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        Self { directory }
    }
}

impl Drop for ApplicationFixture {
    fn drop(&mut self) {
        let socket = self.directory.join("tmux.sock");
        if socket.exists() {
            let _ = Command::new("tmux")
                .arg("-S")
                .arg(socket)
                .arg("kill-server")
                .output();
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn application_available(name: &str) -> bool {
    let available = Command::new(name).arg("--version").output().is_ok();
    if std::env::var_os("CHELOTYPE_REQUIRE_E2E").is_some() {
        assert!(available, "required E2E application missing: {name}");
    }
    if !available {
        eprintln!("skipping application compatibility test: {name} is not installed");
    }
    available
}

fn wait_for(
    backend: &mut TerminalBackend,
    predicate: impl Fn(&RenderableContentOwned) -> bool,
) -> RenderableContentOwned {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last_text = String::new();
    while Instant::now() < deadline {
        if let Some(snapshot) = backend.snapshot_renderable() {
            if predicate(&snapshot) {
                return snapshot;
            }
            last_text = lines_to_text(&snapshot.lines);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("application did not reach expected terminal state: {last_text}");
}

#[test]
fn neovim_edits_unicode_survives_resize_and_restores_the_shell() {
    if !application_available("nvim") {
        return;
    }
    let fixture = ApplicationFixture::new();
    let mut command = CommandBuilder::new("/bin/sh");
    command.args([
        "-c",
        "nvim -u NONE -n -i NONE -c 'set shortmess+=I' buffer.txt; printf '\\nEDITOR_EXITED:%s\\n' \"$?\"; sleep 1",
    ]);
    command.cwd(&fixture.directory);
    command.env("NVIM_APPNAME", "chelotype-test");
    let mut backend = TerminalBackend::spawn(command).unwrap();
    wait_for(&mut backend, |snapshot| {
        lines_to_text(&snapshot.lines).contains("buffer.txt")
    });
    let line = "Chelotype: Привіт 世界 e\u{301}";
    backend.write(format!("i{line}\x1b").as_bytes()).unwrap();
    wait_for(&mut backend, |snapshot| {
        lines_to_text(&snapshot.lines).contains(line)
    });
    backend.resize(ScreenSize::new(52, 15).unwrap()).unwrap();
    wait_for(&mut backend, |snapshot| {
        snapshot.lines.len() == 15 && lines_to_text(&snapshot.lines).contains(line)
    });
    backend.write(b":wq\r").unwrap();
    wait_for(&mut backend, |snapshot| {
        lines_to_text(&snapshot.lines).contains("EDITOR_EXITED:0")
    });
    assert_eq!(
        std::fs::read_to_string(fixture.directory.join("buffer.txt")).unwrap(),
        format!("{line}\n")
    );
    assert!(!backend.bracketed_paste_mode());
}

#[test]
fn tmux_routes_input_updates_pty_size_and_restores_mouse_mode_on_exit() {
    if !application_available("tmux") {
        return;
    }
    let fixture = ApplicationFixture::new();
    std::fs::write(fixture.directory.join("tmux.conf"), "set -g mouse on\n").unwrap();
    let mut command = CommandBuilder::new("/bin/sh");
    command.args([
        "-c",
        "tmux -S ./tmux.sock -f ./tmux.conf new-session -s chelotype 'env PS1=TMUX_READY\\> /bin/sh'; printf '\\nMULTIPLEXER_EXITED:%s\\n' \"$?\"; sleep 1",
    ]);
    command.cwd(&fixture.directory);
    command.env("PS1", "TMUX_READY> ");
    command.env("ENV", "");
    command.env_remove("TMUX");
    let mut backend = TerminalBackend::spawn(command).unwrap();
    wait_for(&mut backend, |snapshot| {
        snapshot.mouse.sends_press_release()
            && lines_to_text(&snapshot.lines).contains("TMUX_READY>")
    });
    backend.write(b"printf '\\116ESTED_OK\\n'\r").unwrap();
    wait_for(&mut backend, |snapshot| {
        lines_to_text(&snapshot.lines).contains("NESTED_OK")
    });
    backend.resize(ScreenSize::new(72, 20).unwrap()).unwrap();
    backend.write(b"stty size\r").unwrap();
    wait_for(&mut backend, |snapshot| {
        lines_to_text(&snapshot.lines).contains("19 72")
    });
    backend.write(b"exit\r").unwrap();
    let snapshot = wait_for(&mut backend, |snapshot| {
        lines_to_text(&snapshot.lines).contains("MULTIPLEXER_EXITED:0")
    });
    assert!(!snapshot.mouse.sends_press_release());
}
