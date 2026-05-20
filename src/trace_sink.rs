use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::time::Duration;

pub struct TraceSink {
    sender: SyncSender<String>,
}

impl TraceSink {
    pub fn new(path: &Path, thread_name: &str) -> Option<Self> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()?;
        let (sender, receiver) = sync_channel::<String>(16_384);
        let thread_name = thread_name.to_string();
        let _ = std::thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                let mut writer = BufWriter::new(file);
                let mut pending = 0usize;
                loop {
                    match receiver.recv_timeout(Duration::from_millis(5)) {
                        Ok(line) => {
                            let _ = writer.write_all(line.as_bytes());
                            pending += 1;
                            if pending >= 256 {
                                let _ = writer.flush();
                                pending = 0;
                            }
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                            if pending > 0 {
                                let _ = writer.flush();
                                pending = 0;
                            }
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                            let _ = writer.flush();
                            break;
                        }
                    }
                }
            });
        Some(Self { sender })
    }

    pub fn record_line(&self, line: String) {
        match self.sender.try_send(line) {
            Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
        }
    }
}
