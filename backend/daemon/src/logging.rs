use std::collections::VecDeque;
use std::io;
use std::sync::Arc;

use parking_lot::RwLock;
use tracing_subscriber::{
    fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter,
};

/// Number of recent log lines kept in memory for `GetLogs` / the GUI's
/// Activity tab.
const LOG_BUFFER_CAPACITY: usize = 1000;

/// Shared ring buffer of recent formatted log lines, fed directly by the
/// `tracing` subscriber so every `info!`/`warn!`/`error!` call anywhere in
/// the daemon shows up here — no call site has to remember to log twice.
pub type LogBuffer = Arc<RwLock<VecDeque<String>>>;

/// A `tracing_subscriber` writer that appends each formatted log line to a
/// [`LogBuffer`] instead of (or in addition to) a file/stream.
#[derive(Clone)]
struct MemoryWriter {
    buffer: LogBuffer,
}

impl io::Write for MemoryWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let line = String::from_utf8_lossy(buf);
        let line = line.trim_end_matches('\n');
        if !line.is_empty() {
            let mut buffer = self.buffer.write();
            if buffer.len() >= LOG_BUFFER_CAPACITY {
                buffer.pop_front();
            }
            buffer.push_back(line.to_string());
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Initialise the global `tracing` subscriber. Returns the in-memory ring
/// buffer it feeds — wire this into `DaemonState::log_buffer` so `GetLogs`
/// (and the GUI's Activity tab) show real daemon log history instead of
/// always being empty.
///
/// * Always attaches a human-readable layer that writes to **stderr**.
/// * Always attaches an in-memory layer capped at [`LOG_BUFFER_CAPACITY`] lines.
/// * If `log_file` is `Some(path)`, also attaches a JSON layer that appends to
///   that file.
/// * The filter priority is: `RUST_LOG` env var → `level` argument → `"info"`.
pub fn init_logging(level: &str, log_file: Option<&str>) -> anyhow::Result<LogBuffer> {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));

    let log_buffer: LogBuffer = Arc::new(RwLock::new(VecDeque::with_capacity(LOG_BUFFER_CAPACITY)));

    let stderr_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(true);

    let memory_writer = MemoryWriter { buffer: log_buffer.clone() };
    let memory_layer = fmt::layer()
        .with_writer(move || memory_writer.clone())
        .with_ansi(false);

    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(stderr_layer)
        .with(memory_layer);

    match log_file {
        None => {
            registry.try_init()?;
        }
        Some(path) => {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?;

            let json_layer = fmt::layer()
                .json()
                .with_writer(file);

            registry.with(json_layer).try_init()?;
        }
    }

    Ok(log_buffer)
}
