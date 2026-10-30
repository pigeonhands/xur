//! `stub::tracing` — script-controlled log collection.
//!
//! The script picks a sink by calling `tracing::register(sink)`:
//!
//!     tracing::register("file", "/tmp/game.log")           // log to file
//!     tracing::register("file", "/tmp/game.log", "trace")  // with level
//!     tracing::register("stdout")                           // log to stdout
//!     tracing::register("stdout", "debug")                  // with level
//!     tracing::register("memory")                           // buffer for drain()
//!     tracing::register("memory", "warn")                   // with level
//!
//! The optional last argument sets the log level filter (default: "info").
//! Valid levels: "trace", "debug", "info", "warn", "error".
//!
//! Then `tracing::drain()` returns buffered lines (only works after
//! registering with "memory").

use std::sync::{Arc, Mutex, OnceLock};

use tracing_subscriber::Layer;
use xur_runtime::registry::ModuleBuilder;
use xur_runtime::{RuntimeError, Value};

static LOG_BUFFER: OnceLock<Arc<Mutex<Vec<String>>>> = OnceLock::new();

type FilteredRegistry = tracing_subscriber::layer::Layered<
    tracing_subscriber::EnvFilter,
    tracing_subscriber::Registry,
>;

pub(crate) fn register(m: &mut ModuleBuilder<'_>) {
    m.raw_function("register", |_cx, args| {
        let Some(Value::Str(kind)) = args.first() else {
            return Err(RuntimeError::new(
                "`register` takes a sink: \"file\", \"stdout\", or \"memory\"",
            ));
        };
        match kind.as_ref() {
            "file" => {
                let Some(Value::Str(path)) = args.get(1) else {
                    return Err(RuntimeError::new(
                        "`register(\"file\", path)` requires a path",
                    ));
                };
                let level = parse_level(args.get(2))?;
                let file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path.as_ref())
                    .map_err(|e| RuntimeError::new(format!("tracing: {e}")))?;
                install_fmt(
                    tracing_subscriber::fmt::layer()
                        .with_writer(std::sync::Mutex::new(file))
                        .with_ansi(false)
                        .boxed(),
                    &level,
                );
            }
            "stdout" => {
                let level = parse_level(args.get(1))?;
                install_fmt(
                    tracing_subscriber::fmt::layer()
                        .with_writer(std::io::stdout)
                        .boxed(),
                    &level,
                );
            }
            "memory" => {
                let level = parse_level(args.get(1))?;
                install_memory(&level);
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "unknown sink \"{other}\": expected \"file\", \"stdout\", or \"memory\"",
                )));
            }
        }
        Ok(Value::Unit)
    });

    m.raw_function("info", |_cx, args| {
        let [Value::Str(msg)] = args else {
            return Err(RuntimeError::new("`info` takes a message string"));
        };
        tracing::info!("{}", msg.as_ref());
        Ok(Value::Unit)
    });

    m.raw_function("debug", |_cx, args| {
        let [Value::Str(msg)] = args else {
            return Err(RuntimeError::new("`debug` takes a message string"));
        };
        tracing::debug!("{}", msg.as_ref());
        Ok(Value::Unit)
    });

    m.raw_function("warn", |_cx, args| {
        let [Value::Str(msg)] = args else {
            return Err(RuntimeError::new("`warn` takes a message string"));
        };
        tracing::warn!("{}", msg.as_ref());
        Ok(Value::Unit)
    });

    m.raw_function("error", |_cx, args| {
        let [Value::Str(msg)] = args else {
            return Err(RuntimeError::new("`error` takes a message string"));
        };
        tracing::error!("{}", msg.as_ref());
        Ok(Value::Unit)
    });

    m.raw_function("drain", |_cx, args| {
        if !args.is_empty() {
            return Err(RuntimeError::new("`drain` takes no arguments"));
        }
        let Some(buf) = LOG_BUFFER.get() else {
            return Err(RuntimeError::new(
                "tracing not registered with \"memory\" — call `tracing::register(\"memory\")` first",
            ));
        };
        let lines = buf
            .lock()
            .map(|mut buf| std::mem::take(&mut *buf))
            .unwrap_or_default();
        Ok(Value::list(lines.into_iter().map(|s| Value::str(&s)).collect()))
    });
}

fn parse_level(arg: Option<&Value>) -> Result<String, RuntimeError> {
    match arg {
        None => Ok("info".into()),
        Some(Value::Str(s)) => {
            let l = s.as_ref();
            match l {
                "trace" | "debug" | "info" | "warn" | "error" => Ok(l.into()),
                _ => Err(RuntimeError::new(format!(
                    "unknown level \"{l}\": expected \"trace\", \"debug\", \"info\", \"warn\", or \"error\"",
                ))),
            }
        }
        Some(other) => Err(RuntimeError::new(format!(
            "level must be a string, got {}",
            other.kind(),
        ))),
    }
}

fn make_filter(level: &str) -> tracing_subscriber::EnvFilter {
    tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| level.into())
}

fn install_fmt(fmt_layer: Box<dyn Layer<FilteredRegistry> + Send + Sync>, level: &str) {
    use tracing_subscriber::prelude::*;

    let _ = tracing_subscriber::registry()
        .with(make_filter(level))
        .with(fmt_layer)
        .try_init();
}

fn install_memory(level: &str) {
    use tracing_subscriber::prelude::*;

    let buffer = Arc::clone(LOG_BUFFER.get_or_init(|| Arc::new(Mutex::new(Vec::new()))));

    let _ = tracing_subscriber::registry()
        .with(make_filter(level))
        .with(LogCollector { buffer })
        .try_init();
}

// -----------------------------------------------------------------------
// Tracing layer
// -----------------------------------------------------------------------

struct LogCollector {
    buffer: Arc<Mutex<Vec<String>>>,
}

impl<S: tracing::Subscriber> Layer<S> for LogCollector {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let meta = event.metadata();
        let mut visitor = MessageVisitor(String::new());
        event.record(&mut visitor);
        let line = format!("{} {}: {}", meta.level(), meta.target(), visitor.0);
        if let Ok(mut buf) = self.buffer.lock() {
            buf.push(line);
        }
    }
}

struct MessageVisitor(String);

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        } else if !self.0.is_empty() {
            self.0.push_str(&format!(" {}={value:?}", field.name()));
        } else {
            self.0 = format!("{}={value:?}", field.name());
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_string();
        } else if !self.0.is_empty() {
            self.0.push_str(&format!(" {}={value}", field.name()));
        } else {
            self.0 = format!("{}={value}", field.name());
        }
    }
}
