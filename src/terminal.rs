use gtk::{gio, glib};
use vte::prelude::*;
use vte::{PtyFlags, Terminal};

pub fn start_shell(terminal: &Terminal) {
    let shell_path = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    let argv = [shell_path.as_str()];
    spawn_process(terminal, &argv);
}

pub fn spawn_process(terminal: &Terminal, argv: &[&str]) {
    let envv_owned: Vec<String> = std::env::vars()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    let envv: Vec<&str> = envv_owned.iter().map(String::as_str).collect();

    terminal.spawn_async(
        PtyFlags::DEFAULT,
        None::<&str>,
        argv,
        &envv,
        glib::SpawnFlags::SEARCH_PATH,
        || {},
        -1,
        None::<&gio::Cancellable>,
        |result| {
            if let Err(error) = result {
                eprintln!("Failed to start process: {error}");
            }
        },
    );
}
