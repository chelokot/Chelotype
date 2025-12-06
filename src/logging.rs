use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::{Mutex, OnceLock};

pub fn debug_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("CHELOTYPE_DEBUG") == Ok("1".to_string()))
}

fn debug_file() -> Option<&'static Mutex<File>> {
    static FILE: OnceLock<Option<Mutex<File>>> = OnceLock::new();
    FILE.get_or_init(|| {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/chelotype.log")
            .map(Mutex::new)
            .ok()
    })
    .as_ref()
}

pub fn debug_log(event: &str) {
    if !debug_enabled() {
        return;
    }
    if let Some(file) = debug_file()
        && let Ok(mut guard) = file.lock()
    {
        let _ = writeln!(guard, "{event}");
    }
}
