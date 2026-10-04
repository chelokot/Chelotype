use chelotype::backend::TerminalBackend;
use chelotype::session_context::{ProcessSnapshot, SessionContext, classify_process_group};
use portable_pty::CommandBuilder;
use std::path::Path;
use std::time::{Duration, Instant};

fn process(
    pid: i32,
    parent_pid: i32,
    process_group_id: i32,
    session_id: i32,
    effective_uid: u32,
    executable: &str,
) -> ProcessSnapshot {
    ProcessSnapshot::new(
        pid,
        parent_pid,
        process_group_id,
        session_id,
        effective_uid,
        executable,
    )
}

fn wait_for_context(backend: &TerminalBackend, expected: impl Fn(SessionContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        let context = backend.session_context();
        if expected(context) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!(
        "terminal context did not become ready: {:?}",
        backend.session_context()
    );
}

#[test]
fn process_context_uses_foreground_ancestry_and_exact_executable_identity() {
    let processes = vec![
        process(100, 1, 100, 100, 1000, "/usr/bin/bash"),
        process(200, 100, 200, 100, 1000, "/tmp/ssh-wrapper"),
        process(300, 200, 200, 100, 0, "/usr/bin/bash"),
        process(400, 1, 400, 400, 1000, "/usr/bin/ssh"),
    ];
    assert_eq!(
        classify_process_group(200, &processes),
        SessionContext::Root
    );
    assert!(!classify_process_group(200, &processes).is_ssh());
    assert_eq!(classify_process_group(400, &processes), SessionContext::Ssh);
}

#[test]
fn process_context_does_not_cross_session_boundaries() {
    let processes = vec![
        process(100, 1, 100, 100, 1000, "/usr/bin/bash"),
        process(200, 100, 200, 100, 1000, "/usr/bin/bash"),
        process(300, 200, 200, 300, 0, "/usr/bin/bash"),
    ];
    assert_eq!(
        classify_process_group(200, &processes),
        SessionContext::Normal
    );
}

#[test]
fn pty_detector_reports_a_known_local_shell_context() {
    let mut command = CommandBuilder::new("/bin/sh");
    command.args(["-c", "sleep 2"]);
    let mut backend = TerminalBackend::spawn(command).expect("spawn shell process");
    wait_for_context(&backend, |context| context == SessionContext::Normal);
    backend.shutdown();
}

#[test]
fn pty_detector_reports_an_active_ssh_client() {
    if !Path::new("/usr/bin/ssh").is_file() {
        return;
    }
    let mut command = CommandBuilder::new("/usr/bin/ssh");
    command.args([
        "-F",
        "/dev/null",
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=10",
        "-o",
        "ProxyCommand=/usr/bin/sleep 5",
        "chelotype-session-context-test.invalid",
        "true",
    ]);
    let mut backend = TerminalBackend::spawn(command).expect("spawn ssh process");
    wait_for_context(&backend, SessionContext::is_ssh);
    backend.shutdown();
}

#[test]
fn pty_detector_reports_an_authenticated_ssh_session_when_fixture_is_configured() {
    let Ok(port) = std::env::var("CHELOTYPE_SESSION_CONTEXT_SSH_PORT") else {
        return;
    };
    let Ok(key) = std::env::var("CHELOTYPE_SESSION_CONTEXT_SSH_KEY") else {
        return;
    };
    if !Path::new("/usr/bin/ssh").is_file() || !Path::new(&key).is_file() {
        return;
    }
    let mut command = CommandBuilder::new("/usr/bin/ssh");
    command.args([
        "-F",
        "/dev/null",
        "-i",
        key.as_str(),
        "-p",
        port.as_str(),
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=no",
        "-o",
        "UserKnownHostsFile=/dev/null",
        "-o",
        "ConnectTimeout=3",
        "chelokot@127.0.0.1",
        "sleep",
        "2",
    ]);
    let mut backend = TerminalBackend::spawn(command).expect("spawn authenticated ssh process");
    wait_for_context(&backend, SessionContext::is_ssh);
    backend.shutdown();
}

#[test]
fn pty_detector_finds_ssh_in_a_foreground_process_group() {
    if !Path::new("/usr/bin/ssh").is_file() {
        return;
    }
    let mut command = CommandBuilder::new("/bin/sh");
    command.args([
        "-c",
        "/usr/bin/sleep 5 | /usr/bin/ssh -F /dev/null -o BatchMode=yes -o ConnectTimeout=10 -o 'ProxyCommand=/usr/bin/sleep 5' chelotype-session-context-test.invalid true",
    ]);
    let mut backend = TerminalBackend::spawn(command).expect("spawn ssh pipeline");
    wait_for_context(&backend, SessionContext::is_ssh);
    backend.shutdown();
}

#[cfg(unix)]
#[test]
fn pty_detector_reports_root_when_the_test_process_is_root() {
    if unsafe { libc::geteuid() } != 0 {
        return;
    }
    let mut command = CommandBuilder::new("/bin/sh");
    command.args(["-c", "sleep 2"]);
    let mut backend = TerminalBackend::spawn(command).expect("spawn root process");
    wait_for_context(&backend, SessionContext::is_root);
    backend.shutdown();
}

#[cfg(unix)]
#[test]
fn pty_detector_observes_a_sudo_root_process_when_passwordless_sudo_is_available() {
    if unsafe { libc::geteuid() } == 0 || !Path::new("/usr/bin/sudo").is_file() {
        return;
    }
    let status = std::process::Command::new("/usr/bin/sudo")
        .args(["-n", "-u", "root", "/usr/bin/true"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("probe passwordless sudo");
    if !status.success() {
        return;
    }
    let mut command = CommandBuilder::new("/usr/bin/sudo");
    command.args(["-n", "-u", "root", "/bin/sh", "-c", "sleep 2"]);
    let mut backend = TerminalBackend::spawn(command).expect("spawn sudo root process");
    wait_for_context(&backend, SessionContext::is_root);
    backend.shutdown();
}
