//! `std::io`: printing.

use xur_runtime::{Context, Registry, RunResult, RuntimeError, Value};

use crate::fmt::format;

pub(crate) fn register(registry: &Registry) {
    registry
        .module("std::io")
        .raw_function("print", |cx, args| print(cx, args, ""))
        .raw_function("println", |cx, args| print(cx, args, "\n"));
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

#[cfg(test)]
mod tests {
    use xur_runtime::Value;
    use xur_runtime::testing::{TestCx, fn_id};

    use crate::registry;

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
}
