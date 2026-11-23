//! `std::log`: tracing-based logging with a builder-style subscriber.
//!
//! ```text
//! use std::log;
//!
//! log::subscriber()
//!     .with_max_level(log::Level::Debug)
//!     .with_target(true)
//!     .with_ansi(true)
//!     .init();
//!
//! log::info("application started");
//! log::debug("value: {}", 42);
//! log::warn("something to watch");
//! ```

use std::sync::Arc;

use facet::Facet;
use xur_runtime::registry::XurType;
use xur_runtime::{Context, NativeObject, Registry, RunResult, RuntimeError, Value};

use crate::fmt;

/// Output format for the subscriber.
#[derive(Clone, Debug)]
enum Format {
    Full,
    Pretty,
    Compact,
    Json,
}

/// Where log output goes.
#[derive(Clone, Debug)]
enum Output {
    Stderr,
    File(String),
}

/// Configuration carried by the builder.
#[derive(Clone, Debug)]
struct SubscriberConfig {
    max_level: String,
    with_target: bool,
    with_file: bool,
    with_line_number: bool,
    with_ansi: bool,
    truncate: bool,
    format: Format,
    filter: Option<String>,
    output: Output,
}

impl Default for SubscriberConfig {
    fn default() -> Self {
        Self {
            max_level: "info".to_owned(),
            with_target: true,
            with_file: false,
            with_line_number: false,
            with_ansi: true,
            truncate: false,
            format: Format::Full,
            filter: None,
            output: Output::Stderr,
        }
    }
}

/// The builder exposed to xur scripts as a native object.
#[derive(Facet, Clone)]
#[facet(opaque)]
struct SubscriberBuilder(Arc<SubscriberConfig>);

/// Return a new native `SubscriberBuilder` with the same type id as `like`.
fn native(like: &Value, builder: SubscriberBuilder) -> RunResult<Value> {
    let Value::Native(object) = like else {
        return Err(RuntimeError::new("expected a `SubscriberBuilder`"));
    };
    Ok(Value::Native(Arc::new(NativeObject::new(
        object.ty,
        Box::new(builder),
    ))))
}

fn builder_arg(args: &[Value]) -> RunResult<SubscriberBuilder> {
    match args.first() {
        Some(Value::Native(object)) => object.with(|b: &SubscriberBuilder| b.clone()),
        _ => None,
    }
    .ok_or_else(|| RuntimeError::new("expected a `SubscriberBuilder`"))
}

/// Clone the config, apply a change, return a new builder value.
fn with_config(args: &[Value], change: impl FnOnce(&mut SubscriberConfig)) -> RunResult<Value> {
    let builder = builder_arg(args)?;
    let mut config = SubscriberConfig::clone(&builder.0);
    change(&mut config);
    native(&args[0], SubscriberBuilder(Arc::new(config)))
}

const LEVEL_TRACE: i64 = 0;
const LEVEL_DEBUG: i64 = 1;
const LEVEL_INFO: i64 = 2;
const LEVEL_WARN: i64 = 3;
const LEVEL_ERROR: i64 = 4;

fn parse_level(s: &str) -> RunResult<tracing::Level> {
    match s {
        "trace" => Ok(tracing::Level::TRACE),
        "debug" => Ok(tracing::Level::DEBUG),
        "info" => Ok(tracing::Level::INFO),
        "warn" => Ok(tracing::Level::WARN),
        "error" => Ok(tracing::Level::ERROR),
        _ => Err(RuntimeError::new(format!(
            "unknown level \"{s}\": expected trace, debug, info, warn, or error"
        ))),
    }
}

fn parse_level_int(v: i64) -> RunResult<&'static str> {
    match v {
        LEVEL_TRACE => Ok("trace"),
        LEVEL_DEBUG => Ok("debug"),
        LEVEL_INFO => Ok("info"),
        LEVEL_WARN => Ok("warn"),
        LEVEL_ERROR => Ok("error"),
        _ => Err(RuntimeError::new(format!("unknown level value {v}"))),
    }
}

fn level_string(cx: &mut dyn Context, args: &[Value], pos: usize) -> RunResult<String> {
    match args.get(pos) {
        Some(Value::Str(s)) => {
            parse_level(s)?;
            Ok(s.to_string())
        }
        Some(Value::Int(v)) => Ok(parse_level_int(*v)?.to_string()),
        Some(Value::Adt(adt)) => {
            let v = cx
                .repr_value(adt.ctor)
                .ok_or_else(|| RuntimeError::new("expected a Level enum value"))?;
            Ok(parse_level_int(v)?.to_string())
        }
        _ => Err(RuntimeError::new("expected a Level or level string")),
    }
}

fn bool_arg(args: &[Value], pos: usize, name: &str) -> RunResult<bool> {
    match args.get(pos) {
        Some(Value::Bool(b)) => Ok(*b),
        _ => Err(RuntimeError::new(format!("`{name}` takes a bool"))),
    }
}

fn str_arg(args: &[Value], pos: usize, name: &str) -> RunResult<Arc<str>> {
    match args.get(pos) {
        Some(Value::Str(s)) => Ok(Arc::clone(s)),
        _ => Err(RuntimeError::new(format!("`{name}` takes a string"))),
    }
}

/// Install the subscriber described by `config`.
fn init_subscriber(config: &SubscriberConfig) -> RunResult<()> {
    // EnvFilter returns Interest::sometimes() for all callsites, forcing
    // the tracing dispatcher to re-check enabled() per event.  Under
    // hook-heavy workloads (function detours, coreclr init, open/openat
    // interception) the callsite Interest cache can go stale and silently
    // drop events.  When the user only needs a simple max-level gate, we
    // skip EnvFilter entirely and use the fmt subscriber's built-in
    // with_max_level(), which yields Interest::always() — callsites below
    // the level are permanently enabled and never re-checked.
    if config.filter.is_some() || std::env::var_os("RUST_LOG").is_some() {
        return init_subscriber_envfilter(config);
    }

    let level = parse_level(&config.max_level).unwrap_or(tracing::Level::INFO);

    macro_rules! apply_common {
        ($layer:expr, $config:expr) => {
            $layer
                .with_target($config.with_target)
                .with_file($config.with_file)
                .with_line_number($config.with_line_number)
                .with_ansi($config.with_ansi)
        };
    }

    match &config.output {
        Output::Stderr => match config.format {
            Format::Full => {
                apply_common!(tracing_subscriber::fmt(), config)
                    .with_max_level(level)
                    .try_init()
                    .map_err(|e| RuntimeError::new(format!("log init failed: {e}")))?;
            }
            Format::Pretty => {
                apply_common!(tracing_subscriber::fmt().pretty(), config)
                    .with_max_level(level)
                    .try_init()
                    .map_err(|e| RuntimeError::new(format!("log init failed: {e}")))?;
            }
            Format::Compact => {
                apply_common!(tracing_subscriber::fmt().compact(), config)
                    .with_max_level(level)
                    .try_init()
                    .map_err(|e| RuntimeError::new(format!("log init failed: {e}")))?;
            }
            Format::Json => {
                tracing_subscriber::fmt()
                    .json()
                    .with_target(config.with_target)
                    .with_file(config.with_file)
                    .with_line_number(config.with_line_number)
                    .with_ansi(config.with_ansi)
                    .with_max_level(level)
                    .try_init()
                    .map_err(|e| RuntimeError::new(format!("log init failed: {e}")))?;
            }
        },
        Output::File(path) => {
            if config.truncate {
                let _ = std::fs::write(path, b"");
            }
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|e| RuntimeError::new(format!("log init: {e}")))?;
            let writer = std::sync::Mutex::new(file);
            match config.format {
                Format::Full => {
                    tracing_subscriber::fmt()
                        .with_writer(writer)
                        .with_target(config.with_target)
                        .with_file(config.with_file)
                        .with_line_number(config.with_line_number)
                        .with_ansi(false)
                        .with_max_level(level)
                        .try_init()
                        .map_err(|e| RuntimeError::new(format!("log init failed: {e}")))?;
                }
                Format::Pretty => {
                    tracing_subscriber::fmt()
                        .pretty()
                        .with_writer(writer)
                        .with_target(config.with_target)
                        .with_file(config.with_file)
                        .with_line_number(config.with_line_number)
                        .with_ansi(false)
                        .with_max_level(level)
                        .try_init()
                        .map_err(|e| RuntimeError::new(format!("log init failed: {e}")))?;
                }
                Format::Compact => {
                    tracing_subscriber::fmt()
                        .compact()
                        .with_writer(writer)
                        .with_target(config.with_target)
                        .with_file(config.with_file)
                        .with_line_number(config.with_line_number)
                        .with_ansi(false)
                        .with_max_level(level)
                        .try_init()
                        .map_err(|e| RuntimeError::new(format!("log init failed: {e}")))?;
                }
                Format::Json => {
                    tracing_subscriber::fmt()
                        .json()
                        .with_writer(writer)
                        .with_target(config.with_target)
                        .with_file(config.with_file)
                        .with_line_number(config.with_line_number)
                        .with_ansi(false)
                        .with_max_level(level)
                        .try_init()
                        .map_err(|e| RuntimeError::new(format!("log init failed: {e}")))?;
                }
            }
        }
    }

    // Callsites registered before the subscriber was set get stale Interest
    // values (usually Interest::never). Rebuild the cache so every existing
    // callsite re-evaluates against the new subscriber — this makes tracing
    // work from threads that emitted callsites before this point (hook
    // threads, spawned init threads, etc.).
    tracing::callsite::rebuild_interest_cache();

    Ok(())
}

/// EnvFilter path — used only when RUST_LOG or a custom filter is set.
fn init_subscriber_envfilter(config: &SubscriberConfig) -> RunResult<()> {
    use tracing_subscriber::prelude::*;

    let env_filter = match &config.filter {
        Some(filter) => {
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                filter
                    .parse()
                    .unwrap_or_else(|_| config.max_level.clone().into())
            })
        }
        None => tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| config.max_level.clone().into()),
    };

    macro_rules! apply_common {
        ($layer:expr, $config:expr) => {
            $layer
                .with_target($config.with_target)
                .with_file($config.with_file)
                .with_line_number($config.with_line_number)
                .with_ansi($config.with_ansi)
        };
    }

    match &config.output {
        Output::Stderr => match config.format {
            Format::Full => {
                let layer = apply_common!(tracing_subscriber::fmt::layer(), config);
                tracing_subscriber::registry()
                    .with(env_filter)
                    .with(layer)
                    .try_init()
                    .map_err(|e| RuntimeError::new(format!("log init failed: {e}")))?;
            }
            Format::Pretty => {
                let layer = apply_common!(tracing_subscriber::fmt::layer().pretty(), config);
                tracing_subscriber::registry()
                    .with(env_filter)
                    .with(layer)
                    .try_init()
                    .map_err(|e| RuntimeError::new(format!("log init failed: {e}")))?;
            }
            Format::Compact => {
                let layer = apply_common!(tracing_subscriber::fmt::layer().compact(), config);
                tracing_subscriber::registry()
                    .with(env_filter)
                    .with(layer)
                    .try_init()
                    .map_err(|e| RuntimeError::new(format!("log init failed: {e}")))?;
            }
            Format::Json => {
                let layer = tracing_subscriber::fmt::layer()
                    .json()
                    .with_target(config.with_target)
                    .with_file(config.with_file)
                    .with_line_number(config.with_line_number)
                    .with_ansi(config.with_ansi);
                tracing_subscriber::registry()
                    .with(env_filter)
                    .with(layer)
                    .try_init()
                    .map_err(|e| RuntimeError::new(format!("log init failed: {e}")))?;
            }
        },
        Output::File(path) => {
            if config.truncate {
                let _ = std::fs::write(path, b"");
            }
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|e| RuntimeError::new(format!("log init: {e}")))?;
            let writer = std::sync::Mutex::new(file);
            match config.format {
                Format::Full => {
                    let layer = tracing_subscriber::fmt::layer()
                        .with_writer(writer)
                        .with_target(config.with_target)
                        .with_file(config.with_file)
                        .with_line_number(config.with_line_number)
                        .with_ansi(false);
                    tracing_subscriber::registry()
                        .with(env_filter)
                        .with(layer)
                        .try_init()
                        .map_err(|e| RuntimeError::new(format!("log init failed: {e}")))?;
                }
                Format::Pretty => {
                    let layer = tracing_subscriber::fmt::layer()
                        .pretty()
                        .with_writer(writer)
                        .with_target(config.with_target)
                        .with_file(config.with_file)
                        .with_line_number(config.with_line_number)
                        .with_ansi(false);
                    tracing_subscriber::registry()
                        .with(env_filter)
                        .with(layer)
                        .try_init()
                        .map_err(|e| RuntimeError::new(format!("log init failed: {e}")))?;
                }
                Format::Compact => {
                    let layer = tracing_subscriber::fmt::layer()
                        .compact()
                        .with_writer(writer)
                        .with_target(config.with_target)
                        .with_file(config.with_file)
                        .with_line_number(config.with_line_number)
                        .with_ansi(false);
                    tracing_subscriber::registry()
                        .with(env_filter)
                        .with(layer)
                        .try_init()
                        .map_err(|e| RuntimeError::new(format!("log init failed: {e}")))?;
                }
                Format::Json => {
                    let layer = tracing_subscriber::fmt::layer()
                        .json()
                        .with_writer(writer)
                        .with_target(config.with_target)
                        .with_file(config.with_file)
                        .with_line_number(config.with_line_number)
                        .with_ansi(false);
                    tracing_subscriber::registry()
                        .with(env_filter)
                        .with(layer)
                        .try_init()
                        .map_err(|e| RuntimeError::new(format!("log init failed: {e}")))?;
                }
            }
        }
    }

    tracing::callsite::rebuild_interest_cache();

    Ok(())
}

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::log", |module| {
        // subscriber() — returns a new SubscriberBuilder with defaults.
        module.repr_enum(
            "Level",
            &[
                ("Trace", LEVEL_TRACE),
                ("Debug", LEVEL_DEBUG),
                ("Info", LEVEL_INFO),
                ("Warn", LEVEL_WARN),
                ("Error", LEVEL_ERROR),
            ],
        );

        module
            .ty::<SubscriberBuilder>("SubscriberBuilder")
            .raw_function_returns::<SubscriberBuilder>("with_max_level", |cx, args| {
                let level = level_string(cx, args, 1)?;
                with_config(args, |c| c.max_level = level)
            })
            .raw_function_returns::<SubscriberBuilder>("with_target", |_cx, args| {
                let b = bool_arg(args, 1, "with_target")?;
                with_config(args, |c| c.with_target = b)
            })
            .raw_function_returns::<SubscriberBuilder>("with_file", |_cx, args| {
                let b = bool_arg(args, 1, "with_file")?;
                with_config(args, |c| c.with_file = b)
            })
            .raw_function_returns::<SubscriberBuilder>("with_line_number", |_cx, args| {
                let b = bool_arg(args, 1, "with_line_number")?;
                with_config(args, |c| c.with_line_number = b)
            })
            .raw_function_returns::<SubscriberBuilder>("with_ansi", |_cx, args| {
                let b = bool_arg(args, 1, "with_ansi")?;
                with_config(args, |c| c.with_ansi = b)
            })
            .raw_function_returns::<SubscriberBuilder>("json", |_cx, args| {
                with_config(args, |c| c.format = Format::Json)
            })
            .raw_function_returns::<SubscriberBuilder>("pretty", |_cx, args| {
                with_config(args, |c| c.format = Format::Pretty)
            })
            .raw_function_returns::<SubscriberBuilder>("compact", |_cx, args| {
                with_config(args, |c| c.format = Format::Compact)
            })
            .raw_function_returns::<SubscriberBuilder>("with_filter", |_cx, args| {
                let filter = str_arg(args, 1, "with_filter")?;
                with_config(args, |c| c.filter = Some(filter.to_string()))
            })
            .raw_function_returns::<SubscriberBuilder>("truncate", |_cx, args| {
                with_config(args, |c| c.truncate = true)
            })
            .raw_function_returns::<SubscriberBuilder>("to_file", |_cx, args| {
                let path = str_arg(args, 1, "to_file")?;
                with_config(args, |c| c.output = Output::File(path.to_string()))
            })
            .raw_function_returns::<SubscriberBuilder>("to_stderr", |_cx, args| {
                with_config(args, |c| c.output = Output::Stderr)
            })
            .raw_function_returns_xur("init", XurType::UNIT, |_cx, args| {
                let builder = builder_arg(args)?;
                init_subscriber(&builder.0)?;
                Ok(Value::Unit)
            });

        // subscriber() — module-level function returning a fresh builder.
        module.function("subscriber", || {
            SubscriberBuilder(Arc::new(SubscriberConfig::default()))
        });

        // Log functions: trace, debug, info, warn, error.
        module.raw_function_returns_xur("trace", XurType::UNIT, |cx, args| {
            let msg = log_message(cx, args)?;
            tracing::trace!("{}", msg);
            Ok(Value::Unit)
        });
        module.raw_function_returns_xur("debug", XurType::UNIT, |cx, args| {
            let msg = log_message(cx, args)?;
            tracing::debug!("{}", msg);
            Ok(Value::Unit)
        });
        module.raw_function_returns_xur("info", XurType::UNIT, |cx, args| {
            let msg = log_message(cx, args)?;
            tracing::info!("{}", msg);
            Ok(Value::Unit)
        });
        module.raw_function_returns_xur("warn", XurType::UNIT, |cx, args| {
            let msg = log_message(cx, args)?;
            tracing::warn!("{}", msg);
            Ok(Value::Unit)
        });
        module.raw_function_returns_xur("error", XurType::UNIT, |cx, args| {
            let msg = log_message(cx, args)?;
            tracing::error!("{}", msg);
            Ok(Value::Unit)
        });
    });
}

/// Format a log message from args: `(template, args...)`.
fn log_message(cx: &mut dyn Context, args: &[Value]) -> RunResult<String> {
    let Some((Value::Str(template), rest)) = args.split_first() else {
        return Err(RuntimeError::new("log functions take a format string"));
    };
    if rest.is_empty() {
        Ok(template.to_string())
    } else {
        fmt::format(cx, template, rest)
    }
}
