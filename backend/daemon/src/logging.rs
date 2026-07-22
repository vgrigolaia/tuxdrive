use tracing_subscriber::{
    fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter,
};

/// Initialise the global `tracing` subscriber.
///
/// * Always attaches a human-readable layer that writes to **stderr**.
/// * If `log_file` is `Some(path)`, also attaches a JSON layer that appends to
///   that file.
/// * The filter priority is: `RUST_LOG` env var → `level` argument → `"info"`.
pub fn init_logging(level: &str, log_file: Option<&str>) -> anyhow::Result<()> {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));

    let stderr_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(true);

    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(stderr_layer);

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

    Ok(())
}
