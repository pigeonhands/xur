//! Format strings, as used by `std::io::print` and `println`.

use xur_runtime::{Context, RunResult, RuntimeError, Value};

/// Replaces each `{}` in `template` by the next argument, displayed; `{{`
/// and `}}` write literal braces.
pub(crate) fn format(cx: &dyn Context, template: &str, args: &[Value]) -> RunResult<String> {
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

#[cfg(test)]
mod tests {
    use xur_runtime::Value;
    use xur_runtime::testing::{TestCx, fn_id};

    use crate::registry;

    #[test]
    fn format_errors() {
        let registry = registry();
        let mut cx = TestCx::new(&registry, crate::PRELUDE);
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
}
