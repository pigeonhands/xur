//! Xur's standard library, `std`.
//!
//! It is registered like any other native module, so embedders can run
//! programs with a different standard library, or register more modules
//! alongside it. Each submodule registers itself.

mod any;
mod assert;
mod bytes;
#[cfg(feature = "cli")]
mod cli;
mod fmt;
mod fs;
mod io;
mod json;
mod list;
mod map;
mod math;
mod mem;
mod net;
mod num;
mod path;
mod process;
pub mod re;
mod record;
mod reflect;
mod str;
mod sync;
mod thread;
mod xur;

pub use xur::{set_argv, set_invoked_by};

use xur_runtime::{Context, Registry, RunResult, RuntimeError, Value};

/// A registry holding only `std`.
pub fn registry() -> Registry {
    let registry = Registry::new();
    register(&registry);
    registry
}

/// Registers `std` and its submodules in `registry`.
pub fn register(registry: &Registry) {
    registry
        .module("std")
        .raw_function("import", import)
        .raw_function("try_import", try_import)
        .raw_function("panic", panic)
        .raw_function("argv", argv);
    registry
        .module("std::fmt")
        .raw_function("format", format);
    io::register(registry);
    fs::register(registry);
    process::register(registry);
    thread::register(registry);
    sync::register(registry);
    net::register(registry);
    any::register(registry);
    assert::register(registry);
    reflect::register(registry);
    json::register(registry);
    re::register(registry);
    math::register(registry);
    mem::register(registry);
    list::register(registry);
    map::register(registry);
    bytes::register(registry);
    str::register(registry);
    num::register(registry);
    path::register(registry);
    record::register(registry);
    xur::register(registry);
    #[cfg(feature = "cli")]
    cli::register(registry);
}

/// `std::import(path)`: loads a module, which may be computed at runtime.
fn import(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    match args {
        [Value::Str(path)] => cx.import(path),
        _ => Err(RuntimeError::new(
            "`std::import` takes a single path string",
        )),
    }
}

/// `std::try_import(path)`: like `import`, but returns `Result<module, str>`
/// instead of stopping the program.
fn try_import(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    match args {
        [Value::Str(path)] => {
            let lang = *cx.lang();
            match cx.import(path) {
                Ok(module) => Ok(Value::adt(lang.ok, vec![module])),
                Err(error) => Ok(Value::adt(lang.err, vec![Value::str(&error.message)])),
            }
        }
        _ => Err(RuntimeError::new(
            "`std::try_import` takes a single path string",
        )),
    }
}

/// `Some(value)` or `None`, as Xur values.
fn option(cx: &dyn Context, value: Option<Value>) -> Value {
    let lang = cx.lang();
    match value {
        Some(value) => Value::adt(lang.some, vec![value]),
        None => Value::adt(lang.none, Vec::new()),
    }
}

/// `Ok(value)` or `Err(message)`, as Xur values.
fn result(cx: &dyn Context, value: Result<Value, String>) -> Value {
    let lang = cx.lang();
    match value {
        Ok(value) => Value::adt(lang.ok, vec![value]),
        Err(message) => Value::adt(lang.err, vec![Value::str(&message)]),
    }
}

/// How Rust errors appear in Xur: as the `Err` of a `Result`, by message.
fn error_text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// `std::fmt::format(template, args...)`: returns a formatted string.
fn format(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let Some((Value::Str(template), args)) = args.split_first() else {
        return Err(RuntimeError::new("`format` takes a format string"));
    };
    Ok(Value::str(&fmt::format(cx, template, args)?))
}

/// `std::argv()`: the program's argument list as set by `set_argv`.
fn argv(_cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    if !args.is_empty() {
        return Err(RuntimeError::new("`std::argv` takes no arguments"));
    }
    let items = xur::ARGV.get().cloned().unwrap_or_default();
    Ok(Value::list(items))
}

/// `std::panic(format, args...)`: stops the program with a runtime error,
/// formatted like `println`.
fn panic(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let Some((Value::Str(template), args)) = args.split_first() else {
        return Err(RuntimeError::new("`panic` takes a format string"));
    };
    Err(RuntimeError::new(fmt::format(cx, template, args)?))
}
