//! Methods of strings: plain Rust functions, with the receiver converted to
//! a `String` by facet.

use xur_runtime::{BuiltinType, Registry};

pub(crate) fn register(registry: &Registry) {
    registry
        .module("std::str")
        .named_function("chr", &["codepoint"], |n: u32| -> Result<String, String> {
            char::from_u32(n)
                .map(|c| c.to_string())
                .ok_or_else(|| format!("{n:#x} is not a valid Unicode codepoint"))
        });
    registry
        .builtin(BuiltinType::Str)
        .function("len", |s: String| s.chars().count() as i64)
        .function("is_empty", |s: String| s.is_empty())
        .function("ord", |s: String| -> Result<i64, String> {
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Ok(c as i64),
                _ => Err(format!("`ord` expects a single character, got {}-char string", s.chars().count())),
            }
        })
        .function("to_upper", |s: String| s.to_uppercase())
        .function("to_lower", |s: String| s.to_lowercase())
        .function("trim", |s: String| s.trim().to_owned())
        .function("contains", |s: String, part: String| s.contains(&part))
        .function("starts_with", |s: String, prefix: String| {
            s.starts_with(&prefix)
        })
        .function("ends_with", |s: String, suffix: String| {
            s.ends_with(&suffix)
        })
        .function("replace", |s: String, from: String, to: String| {
            s.replace(&from, &to)
        })
        .function("split", |s: String, separator: String| {
            s.split(&separator).map(str::to_owned).collect::<Vec<_>>()
        })
        .function("lines", |s: String| {
            s.lines().map(str::to_owned).collect::<Vec<_>>()
        })
        .function("repeat", |s: String, n: u32| s.repeat(n as usize))
        .function("pad_left", |s: String, width: usize, fill: String| {
            pad_left(s, width, &fill)
        })
        .function("parse_int", |s: String| {
            s.trim().parse::<i64>().map_err(|e| e.to_string())
        })
        .function("parse_float", |s: String| {
            s.trim().parse::<f64>().map_err(|e| e.to_string())
        });
}

/// `s` with `fill` repeated in front until it is `width` characters long.
fn pad_left(s: String, width: usize, fill: &str) -> String {
    let missing = width.saturating_sub(s.chars().count());
    let padding: String = fill.chars().cycle().take(missing).collect();
    padding + &s
}

#[cfg(test)]
mod tests {
    use xur_runtime::testing::TestCx;
    use xur_runtime::{BuiltinType, Value};

    use crate::registry;

    fn call(name: &str, args: &[Value]) -> Value {
        let registry = registry();
        let mut cx = TestCx::new(&registry, crate::PRELUDE);
        let id = registry
            .builtin_method(BuiltinType::Str, name)
            .expect("method exists");
        cx.call(id, args).unwrap()
    }

    #[test]
    fn string_methods() {
        assert_eq!(call("to_upper", &[Value::str("abc")]), Value::str("ABC"));
        assert_eq!(call("len", &[Value::str("αβ")]), Value::Int(2));
        assert_eq!(
            call("split", &[Value::str("a,b"), Value::str(",")]),
            Value::list(vec![Value::str("a"), Value::str("b")])
        );
    }

    #[test]
    fn ord_returns_codepoint() {
        let registry = registry();
        let lang = TestCx::new(&registry, crate::PRELUDE).lang;
        assert_eq!(
            call("ord", &[Value::str("A")]),
            Value::adt(lang.ok, vec![Value::Int(65)])
        );
    }

    #[test]
    fn ord_rejects_multi_char() {
        let result = call("ord", &[Value::str("AB")]);
        assert!(matches!(result, Value::Adt(_)));
    }

    #[test]
    fn parsing_returns_results() {
        let registry = registry();
        let lang = TestCx::new(&registry, crate::PRELUDE).lang;
        assert_eq!(
            call("parse_int", &[Value::str(" 42 ")]),
            Value::adt(lang.ok, vec![Value::Int(42)])
        );
        assert_eq!(
            call("parse_int", &[Value::str("x")]),
            Value::adt(lang.err, vec![Value::str("invalid digit found in string")])
        );
    }
}
