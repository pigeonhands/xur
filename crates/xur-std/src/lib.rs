//! Xur's standard library, `std`.
//!
//! It is registered like any other native module, so embedders can run
//! programs with a different standard library, or register more modules
//! alongside it.

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
    registry
        .module("std::io")
        .raw_function("print", |cx, args| print(cx, args, ""))
        .raw_function("println", |cx, args| print(cx, args, "\n"));
    registry
        .module("std::any")
        .raw_function("type_name", type_name);
    registry
        .module("std::math")
        .function("sqrt", |x: f64| x.sqrt())
        .function("pow", |x: f64, y: f64| x.powf(y))
        .function("abs", |x: f64| x.abs())
        .function("min", |a: i64, b: i64| a.min(b))
        .function("max", |a: i64, b: i64| a.max(b));
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

/// `print(format, args...)`: each `{}` in `format` is replaced by the next
/// argument; `{{` and `}}` write literal braces.
fn print(cx: &mut dyn Context, args: &[Value], end: &str) -> RunResult<Value> {
    let Some((Value::Str(template), args)) = args.split_first() else {
        return Err(RuntimeError::new(
            "expected a format string as the first argument",
        ));
    };
    let mut text = format(cx, template, args)?;
    text.push_str(end);
    cx.stdout()
        .write_all(text.as_bytes())
        .map_err(|error| RuntimeError::new(format!("failed to write to stdout: {error}")))?;
    Ok(Value::Unit)
}

fn format(cx: &dyn Context, template: &str, args: &[Value]) -> RunResult<String> {
    let mut out = String::with_capacity(template.len());
    let mut args = args.iter();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('{', Some('{')) | ('}', Some('}')) => {
                chars.next();
                out.push(c);
            }
            ('{', Some('}')) => {
                chars.next();
                let arg = args.next().ok_or_else(|| {
                    RuntimeError::new("the format string has more `{}` than arguments")
                })?;
                out.push_str(&cx.display(arg));
            }
            ('{' | '}', _) => {
                return Err(RuntimeError::new(format!(
                    "unmatched `{c}` in format string; write `{c}{c}` for a literal brace"
                )));
            }
            _ => out.push(c),
        }
    }
    if args.next().is_some() {
        return Err(RuntimeError::new(
            "more arguments than `{}` in the format string",
        ));
    }
    Ok(out)
}

/// `type_name::<T>()`: the name of `T`.
fn type_name(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    match (cx.type_args(), args) {
        ([name], []) => Ok(Value::str(name)),
        _ => Err(RuntimeError::new(
            "`type_name` takes one type argument and no arguments: `type_name::<T>()`",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xur_runtime::testing::{TestCx, fn_id};

    #[test]
    fn println_formats_its_arguments() {
        let registry = registry();
        let mut cx = TestCx::new(&registry);
        let println = fn_id(&registry, "std::io::println");
        let args = [
            Value::str("{} + {} = {{{}}}"),
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
        ];
        assert_eq!(cx.call(println, &args), Ok(Value::Unit));
        assert_eq!(cx.output(), "1 + 2 = {3}\n");
    }

    #[test]
    fn format_errors() {
        let registry = registry();
        let mut cx = TestCx::new(&registry);
        let print = fn_id(&registry, "std::io::print");
        let error = |cx: &mut TestCx<'_>, args: &[Value]| cx.call(print, args).unwrap_err().message;
        assert_eq!(
            error(&mut cx, &[Value::str("{}")]),
            "the format string has more `{}` than arguments"
        );
        assert_eq!(
            error(&mut cx, &[Value::str("x"), Value::Int(1)]),
            "more arguments than `{}` in the format string"
        );
        assert_eq!(
            error(&mut cx, &[Value::str("{")]),
            "unmatched `{` in format string; write `{{` for a literal brace"
        );
    }

    #[test]
    fn type_name_uses_the_type_argument() {
        let registry = registry();
        let mut cx = TestCx::new(&registry);
        cx.type_args = vec!["ZeroPacked".into()];
        assert_eq!(
            cx.call(fn_id(&registry, "std::any::type_name"), &[]),
            Ok(Value::str("ZeroPacked"))
        );
    }

    #[test]
    fn math_functions_are_ordinary_rust_functions() {
        let registry = registry();
        let mut cx = TestCx::new(&registry);
        let sqrt = fn_id(&registry, "std::math::sqrt");
        assert_eq!(cx.call(sqrt, &[Value::Float(9.0)]), Ok(Value::Float(3.0)));
        let max = fn_id(&registry, "std::math::max");
        assert_eq!(
            cx.call(max, &[Value::Int(2), Value::Int(5)]),
            Ok(Value::Int(5))
        );
    }
}
