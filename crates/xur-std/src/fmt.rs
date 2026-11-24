//! Format strings, as used by `std::io::print` and `println`.

use std::fmt::Write;

use xur_runtime::{Context, RunResult, RuntimeError, Value};

/// Replaces each `{}` in `template` by the next argument, displayed; `{{`
/// and `}}` write literal braces.  `{:x}` and `{:X}` format integers as
/// lowercase or uppercase hexadecimal.
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
            ('{', Some(':')) => {
                chars.next(); // consume ':'
                let spec = chars.next().ok_or_else(|| {
                    RuntimeError::new("unexpected end of format spec after `{:`")
                })?;
                let close = chars.next().ok_or_else(|| {
                    RuntimeError::new("unexpected end of format spec, expected `}`")
                })?;
                if close != '}' {
                    return Err(RuntimeError::new(format!(
                        "expected `}}` after format spec `:{spec}`, found `{close}`"
                    )));
                }
                let arg = args.next().ok_or_else(|| {
                    RuntimeError::new("the format string has more format specifiers than arguments")
                })?;
                match spec {
                    'x' => match arg {
                        Value::Int(n) => write!(out, "{:x}", n.to_i128()).unwrap(),
                        _ => {
                            return Err(RuntimeError::new(
                                "format spec `:x` requires an integer argument",
                            ))
                        }
                    },
                    'X' => match arg {
                        Value::Int(n) => write!(out, "{:X}", n.to_i128()).unwrap(),
                        _ => {
                            return Err(RuntimeError::new(
                                "format spec `:X` requires an integer argument",
                            ))
                        }
                    },
                    _ => {
                        return Err(RuntimeError::new(format!(
                            "unknown format spec `:{spec}`; supported: `:x`, `:X`"
                        )))
                    }
                }
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
            error(&mut cx, &[Value::str("x"), Value::int(1)]),
            "more arguments than `{}` in the format string"
        );
    }

    #[test]
    fn hex_format() {
        let registry = registry();
        let cx = TestCx::new(&registry, crate::PRELUDE);
        let result = super::format(&cx, "addr: {:x}", &[Value::int(0xDEAD)]).unwrap();
        assert_eq!(result, "addr: dead");
        let result = super::format(&cx, "ADDR: {:X}", &[Value::int(0xBEEF)]).unwrap();
        assert_eq!(result, "ADDR: BEEF");
        let result =
            super::format(&cx, "{} at {:x}", &[Value::str("foo"), Value::int(255)]).unwrap();
        assert_eq!(result, "foo at ff");
    }

    #[test]
    fn hex_format_requires_int() {
        let registry = registry();
        let cx = TestCx::new(&registry, crate::PRELUDE);
        let err = super::format(&cx, "{:x}", &[Value::str("nope")]).unwrap_err();
        assert!(err.message.contains("integer"), "{}", err.message);
    }

    #[test]
    fn no_args_passes_through_literal() {
        let registry = registry();
        let mut cx = TestCx::new(&registry, crate::PRELUDE);
        let print = fn_id(&registry, "std::io::print");
        // With no extra args, braces are literal text, not format directives.
        cx.call(print, &[Value::str("{}")]).unwrap();
        cx.call(print, &[Value::str("{")]).unwrap();
        cx.call(print, &[Value::str("hello {world}")]).unwrap();
    }
}
