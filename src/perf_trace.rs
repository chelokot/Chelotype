use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

fn trace_sink() -> Option<&'static crate::trace_sink::TraceSink> {
    static SINK: OnceLock<Option<crate::trace_sink::TraceSink>> = OnceLock::new();
    SINK.get_or_init(|| {
        let path = PathBuf::from(std::env::var("CHELOTYPE_PERF_TRACE").ok()?);
        crate::trace_sink::TraceSink::new(&path, "chelotype-perf-trace")
    })
    .as_ref()
}

pub fn record_duration(event: &str, duration: Duration) {
    record_line(format!("{event}\t{}\n", duration.as_micros()));
}

pub fn record_counter(event: &str, value: u64) {
    record_line(format!("{event}\t{value}\n"));
}

fn record_line(line: String) {
    let Some(sink) = trace_sink() else {
        return;
    };
    sink.record_line(line);
}

pub fn enabled() -> bool {
    trace_sink().is_some()
}
