//! Xur's standard library, `std`.
//!
//! It is registered like any other native module, so embedders can run
//! programs with a different standard library, or register more modules
//! alongside it. Each submodule registers itself.

mod any;
mod fmt;
mod io;
mod list;
mod math;
mod num;
mod str;

use xur_runtime::{Context, Registry, RunResult, RuntimeError, Value};

/// A registry holding only `std`.
pub fn registry() -> Registry {
    let mut registry = Registry::new();
    register(&mut registry);
    registry
}

/// Registers `std` and its submodules in `registry`.
pub fn register(registry: &mut Registry) {
    registry.module("std").raw_function("import", import);
    io::register(registry);
    any::register(registry);
    math::register(registry);
    list::register(registry);
    str::register(registry);
    num::register(registry);
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
