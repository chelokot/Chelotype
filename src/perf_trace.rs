use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

fn trace_file() -> Option<&'static Mutex<File>> {
    static FILE: OnceLock<Option<Mutex<File>>> = OnceLock::new();
    FILE.get_or_init(|| {
        let path = std::env::var("CHELOTYPE_PERF_TRACE").ok()?;
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map(Mutex::new)
            .ok()
    })
    .as_ref()
}

pub fn record_duration(event: &str, duration: Duration) {
    let Some(file) = trace_file() else {
        return;
    };
    if let Ok(mut guard) = file.lock() {
        let _ = writeln!(guard, "{event}\t{}", duration.as_micros());
    }
}

pub fn record_counter(event: &str, value: u64) {
    let Some(file) = trace_file() else {
        return;
    };
    if let Ok(mut guard) = file.lock() {
        let _ = writeln!(guard, "{event}\t{value}");
    }
}
