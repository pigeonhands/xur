//! `std::io`: printing.

use std::io::Write;

use xur_runtime::{Context, Registry, RunResult, RuntimeError, Value};
use xur_runtime::registry::XurType;

use crate::fmt::format;

pub(crate) fn register(registry: &Registry) {
    registry
        .module("std::io")
        .named_raw_function_returns_xur("print", &["format", "args"], XurType::UNIT, |cx, args| print(cx, args, ""))
        .named_raw_function_returns_xur("println", &["format", "args"], XurType::UNIT, |cx, args| print(cx, args, "\n"))
        .named_raw_function_returns_xur("eprint", &["format", "args"], XurType::UNIT, |cx, args| eprint(cx, args, ""))
        .named_raw_function_returns_xur("eprintln", &["format", "args"], XurType::UNIT, |cx, args| eprint(cx, args, "\n"));
    registry
        .module("std::io::c")
        .named_function("red", &["text"], |s: String| format!("\x1b[31m{s}\x1b[0m"))
        .named_function("green", &["text"], |s: String| format!("\x1b[32m{s}\x1b[0m"))
        .named_function("yellow", &["text"], |s: String| format!("\x1b[33m{s}\x1b[0m"))
        .named_function("blue", &["text"], |s: String| format!("\x1b[34m{s}\x1b[0m"))
        .named_function("magenta", &["text"], |s: String| format!("\x1b[35m{s}\x1b[0m"))
        .named_function("cyan", &["text"], |s: String| format!("\x1b[36m{s}\x1b[0m"))
        .named_function("bold", &["text"], |s: String| format!("\x1b[1m{s}\x1b[0m"))
        .named_function("dim", &["text"], |s: String| format!("\x1b[2m{s}\x1b[0m"));
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
    cx.print(&text)?;
    Ok(Value::Unit)
}

fn eprint(cx: &mut dyn Context, args: &[Value], end: &str) -> RunResult<Value> {
    let Some((Value::Str(template), args)) = args.split_first() else {
        return Err(RuntimeError::new(
            "expected a format string as the first argument",
        ));
    };
    let mut text = format(cx, template, args)?;
    text.push_str(end);
    std::io::stderr()
        .write_all(text.as_bytes())
        .map_err(|e| RuntimeError::new(e.to_string()))?;
    Ok(Value::Unit)
}

#[cfg(test)]
mod tests {
    use xur_runtime::Value;
    use xur_runtime::testing::{TestCx, fn_id};

    use crate::registry;

    #[test]
    fn println_formats_its_arguments() {
        let registry = registry();
        let mut cx = TestCx::new(&registry, crate::PRELUDE);
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
}
