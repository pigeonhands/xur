//! `std::cli`: argument parsing (pico-args) and interactive prompts (inquire).

use std::ffi::OsString;
use std::sync::Mutex;

use facet::Facet;
use xur_runtime::convert::into_value;
use xur_runtime::{NativeObject, Registry, RunResult, RuntimeError, Value, lock};

use crate::{error_text, option, result};

// ---------------------------------------------------------------------------
// Args — imperative argument parser (pico-args)
// ---------------------------------------------------------------------------

#[derive(Facet)]
#[facet(opaque)]
pub struct Args(Mutex<pico_args::Arguments>);

fn with_args<R>(object: &NativeObject, f: impl FnOnce(&Args) -> R) -> RunResult<R> {
    object
        .with(f)
        .ok_or_else(|| RuntimeError::new("expected an `Args` object"))
}

fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_owned().into_boxed_str())
}

fn drain_args(inner: &mut pico_args::Arguments) -> Vec<OsString> {
    std::mem::replace(inner, pico_args::Arguments::from_vec(Vec::new())).finish()
}

fn keys(args: &[Value]) -> Result<pico_args::Keys, RuntimeError> {
    match args {
        [Value::Str(long)] => Ok(pico_args::Keys::from(leak_str(&**long))),
        [Value::Str(short), Value::Str(long)] => {
            Ok(pico_args::Keys::from([leak_str(&**short), leak_str(&**long)]))
        }
        _ => Err(RuntimeError::new(
            "expected one or two string key arguments",
        )),
    }
}

// ---------------------------------------------------------------------------
// Prompts — interactive terminal UI (inquire)
// ---------------------------------------------------------------------------

fn prompt_text(args: &[Value]) -> RunResult<Value> {
    let [Value::Str(message)] = args else {
        return Err(RuntimeError::new("`prompt` takes a message string"));
    };
    inquire::Text::new(&**message)
        .prompt()
        .map(|s| Value::str(&s))
        .map_err(|e| RuntimeError::new(error_text(e)))
}

fn prompt_confirm(args: &[Value]) -> RunResult<Value> {
    let [Value::Str(message)] = args else {
        return Err(RuntimeError::new("`confirm` takes a message string"));
    };
    inquire::Confirm::new(&**message)
        .prompt()
        .map(Value::Bool)
        .map_err(|e| RuntimeError::new(error_text(e)))
}

fn prompt_select(args: &[Value]) -> RunResult<Value> {
    let [Value::Str(message), Value::List(options)] = args else {
        return Err(RuntimeError::new(
            "`select` takes a message and a list of strings",
        ));
    };
    let options: Vec<String> = lock(options)
        .iter()
        .map(|v| match v {
            Value::Str(s) => Ok(s.to_string()),
            _ => Err(RuntimeError::new("options must be strings")),
        })
        .collect::<RunResult<_>>()?;
    if options.is_empty() {
        return Err(RuntimeError::new("options list must not be empty"));
    }
    inquire::Select::new(&**message, options)
        .prompt()
        .map(|s| Value::str(&s))
        .map_err(|e| RuntimeError::new(error_text(e)))
}

fn prompt_multi_select(args: &[Value]) -> RunResult<Value> {
    let [Value::Str(message), Value::List(options)] = args else {
        return Err(RuntimeError::new(
            "`multi_select` takes a message and a list of strings",
        ));
    };
    let options: Vec<String> = lock(options)
        .iter()
        .map(|v| match v {
            Value::Str(s) => Ok(s.to_string()),
            _ => Err(RuntimeError::new("options must be strings")),
        })
        .collect::<RunResult<_>>()?;
    if options.is_empty() {
        return Err(RuntimeError::new("options list must not be empty"));
    }
    inquire::MultiSelect::new(&**message, options)
        .prompt()
        .map(|selected| Value::list(selected.into_iter().map(|s| Value::str(&s)).collect()))
        .map_err(|e| RuntimeError::new(error_text(e)))
}

fn prompt_password(args: &[Value]) -> RunResult<Value> {
    let [Value::Str(message)] = args else {
        return Err(RuntimeError::new("`password` takes a message string"));
    };
    inquire::Password::new(&**message)
        .without_confirmation()
        .prompt()
        .map(|s| Value::str(&s))
        .map_err(|e| RuntimeError::new(error_text(e)))
}

fn prompt_editor(args: &[Value]) -> RunResult<Value> {
    let [Value::Str(message)] = args else {
        return Err(RuntimeError::new("`editor` takes a message string"));
    };
    inquire::Editor::new(&**message)
        .prompt()
        .map(|s| Value::str(&s))
        .map_err(|e| RuntimeError::new(error_text(e)))
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::cli", |module| {
        module
            .raw_function("args", |cx, args| {
                if !args.is_empty() {
                    return Err(RuntimeError::new("`cli::args` takes no arguments"));
                }
                let argv: Vec<OsString> = crate::xur::ARGV
                    .get()
                    .map(|values| {
                        values
                            .iter()
                            .skip(1)
                            .filter_map(|v| match v {
                                Value::Str(s) => Some(OsString::from(&**s)),
                                _ => None,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let inner = pico_args::Arguments::from_vec(argv);
                into_value(cx, Args(Mutex::new(inner)))
            })
            .ty_borrowing::<Args>("Args")
            .raw_function("contains", |_cx, args| {
                let [Value::Native(object), key_args @ ..] = args else {
                    return Err(RuntimeError::new("expected an `Args` object"));
                };
                let keys = keys(key_args)?;
                let found = with_args(object, |a| a.0.lock().unwrap().contains(keys))?;
                Ok(Value::Bool(found))
            })
            .raw_function("option", |cx, args| {
                let [Value::Native(object), key_args @ ..] = args else {
                    return Err(RuntimeError::new("expected an `Args` object"));
                };
                let keys = keys(key_args)?;
                let value: Option<String> = with_args(object, |a| {
                    a.0.lock().unwrap().opt_value_from_str(keys)
                })?
                .map_err(|e| RuntimeError::new(error_text(e)))?;
                Ok(option(cx, value.map(|s| Value::str(&s))))
            })
            .raw_function("value", |_cx, args| {
                let [Value::Native(object), key_args @ ..] = args else {
                    return Err(RuntimeError::new("expected an `Args` object"));
                };
                let keys = keys(key_args)?;
                let value: String = with_args(object, |a| {
                    a.0.lock().unwrap().value_from_str(keys)
                })?
                .map_err(|e| RuntimeError::new(error_text(e)))?;
                Ok(Value::str(&value))
            })
            .raw_function("subcommand", |cx, args| {
                let [Value::Native(object)] = args else {
                    return Err(RuntimeError::new("`subcommand` takes no arguments"));
                };
                let sub: Result<Option<String>, _> = with_args(object, |a| {
                    a.0.lock().unwrap().subcommand()
                })?;
                match sub {
                    Ok(sub) => {
                        let val = option(cx, sub.map(|s| Value::str(&s)));
                        Ok(result(cx, Ok(val)))
                    }
                    Err(e) => Ok(result(cx, Err(error_text(e)))),
                }
            })
            .raw_function("free", |cx, args| {
                let [Value::Native(object)] = args else {
                    return Err(RuntimeError::new("`free` takes no arguments"));
                };
                let free: Option<String> = with_args(object, |a| {
                    a.0.lock().unwrap().opt_free_from_str()
                })?
                .map_err(|e| RuntimeError::new(error_text(e)))?;
                Ok(option(cx, free.map(|s| Value::str(&s))))
            })
            .raw_function("remaining", |_cx, args| {
                let [Value::Native(object)] = args else {
                    return Err(RuntimeError::new("`remaining` takes no arguments"));
                };
                let remaining = with_args(object, |a| drain_args(&mut a.0.lock().unwrap()))?;
                Ok(Value::list(
                    remaining
                        .into_iter()
                        .map(|os| Value::str(&os.to_string_lossy()))
                        .collect(),
                ))
            })
            .raw_function("finish", |cx, args| {
                let [Value::Native(object)] = args else {
                    return Err(RuntimeError::new("`finish` takes no arguments"));
                };
                let remaining = with_args(object, |a| drain_args(&mut a.0.lock().unwrap()))?;
                if remaining.is_empty() {
                    Ok(result(cx, Ok(Value::tuple(vec![]))))
                } else {
                    let names: Vec<_> = remaining
                        .iter()
                        .map(|os| os.to_string_lossy().into_owned())
                        .collect();
                    Ok(result(
                        cx,
                        Err(format!("unexpected arguments: {}", names.join(", "))),
                    ))
                }
            })
            // Prompts
            .raw_function("prompt", |_cx, args| prompt_text(args))
            .raw_function("confirm", |_cx, args| prompt_confirm(args))
            .raw_function("select", |_cx, args| prompt_select(args))
            .raw_function("multi_select", |_cx, args| prompt_multi_select(args))
            .raw_function("password", |_cx, args| prompt_password(args))
            .raw_function("editor", |_cx, args| prompt_editor(args));
    });
}
