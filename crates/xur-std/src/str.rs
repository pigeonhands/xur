//! Methods of strings: plain Rust functions, with the receiver converted to
//! a `String` by facet.

use xur_runtime::{BuiltinType, Context, Registry, RunResult, RuntimeError, Value};

use crate::option;

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
        .function("ord", |s: String| -> Result<u32, String> {
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Ok(c as u32),
                _ => Err(format!("`ord` expects a single character, got {}-char string", s.chars().count())),
            }
        })
        .function("to_upper", |s: String| s.to_uppercase())
        .function("to_lower", |s: String| s.to_lowercase())
        .function("trim", |s: String| s.trim().to_owned())
        .function("trim_start", |s: String| s.trim_start().to_owned())
        .function("trim_end", |s: String| s.trim_end().to_owned())
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
        .function("pad_right", |s: String, width: usize, fill: String| {
            pad_right(s, width, &fill)
        })
        .function("slice", |s: String, start: i64, end: i64| {
            let chars: Vec<char> = s.chars().collect();
            let len = chars.len();
            let start = resolve_index(start, len);
            let end = resolve_index(end, len);
            if start >= end {
                String::new()
            } else {
                chars[start..end].iter().collect()
            }
        })
        .function("chars", |s: String| {
            s.chars().map(|c| c.to_string()).collect::<Vec<_>>()
        })
        .function("bytes", |s: String| {
            s.bytes().map(|b| b as i64).collect::<Vec<i64>>()
        })
        .function("insert", |s: String, index: i64, substr: String| {
            let chars: Vec<char> = s.chars().collect();
            let len = chars.len();
            let pos = resolve_index(index, len);
            let byte_pos = chars[..pos].iter().map(|c| c.len_utf8()).sum::<usize>();
            let mut result = s;
            result.insert_str(byte_pos, &substr);
            result
        })
        .function("remove", |s: String, start: i64, end: i64| {
            let mut chars: Vec<char> = s.chars().collect();
            let len = chars.len();
            let start = resolve_index(start, len);
            let end = resolve_index(end, len);
            if start < end {
                chars.drain(start..end);
            }
            chars.into_iter().collect::<String>()
        })
        .function("parse_int", |s: String| {
            s.trim().parse::<i64>().map_err(|e| e.to_string())
        })
        .function("parse_hex", |s: String| {
            i64::from_str_radix(s.trim().trim_start_matches("0x").trim_start_matches("0X"), 16)
                .map_err(|e| e.to_string())
        })
        .function("parse_float", |s: String| {
            s.trim().parse::<f64>().map_err(|e| e.to_string())
        })
        .raw_function("find", find)
        .raw_function("rfind", rfind);
}

/// Resolve a possibly-negative char index into a clamped `usize`.
fn resolve_index(index: i64, len: usize) -> usize {
    if index < 0 {
        len.saturating_sub((-index) as usize)
    } else {
        (index as usize).min(len)
    }
}

/// `s` with `fill` repeated in front until it is `width` characters long.
fn pad_left(s: String, width: usize, fill: &str) -> String {
    let missing = width.saturating_sub(s.chars().count());
    let padding: String = fill.chars().cycle().take(missing).collect();
    padding + &s
}

/// `s` with `fill` repeated behind until it is `width` characters long.
fn pad_right(s: String, width: usize, fill: &str) -> String {
    let missing = width.saturating_sub(s.chars().count());
    let padding: String = fill.chars().cycle().take(missing).collect();
    s + &padding
}

/// `str.find(needle)` — returns the char index of the first occurrence, or `None`.
fn find(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [Value::Str(s), Value::Str(needle)] = args else {
        return Err(RuntimeError::new("`find` takes a string needle"));
    };
    let found = s.find(&**needle).map(|byte_offset| {
        Value::int(s[..byte_offset].chars().count() as i64)
    });
    Ok(option(cx, found))
}

/// `str.rfind(needle)` — returns the char index of the last occurrence, or `None`.
fn rfind(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [Value::Str(s), Value::Str(needle)] = args else {
        return Err(RuntimeError::new("`rfind` takes a string needle"));
    };
    let found = s.rfind(&**needle).map(|byte_offset| {
        Value::int(s[..byte_offset].chars().count() as i64)
    });
    Ok(option(cx, found))
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
        assert_eq!(call("len", &[Value::str("αβ")]), Value::int(2));
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
            Value::adt(lang.ok, vec![Value::int(65)])
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
            Value::adt(lang.ok, vec![Value::int(42)])
        );
        assert_eq!(
            call("parse_int", &[Value::str("x")]),
            Value::adt(lang.err, vec![Value::str("invalid digit found in string")])
        );
    }

    #[test]
    fn parse_hex_returns_results() {
        let registry = registry();
        let lang = TestCx::new(&registry, crate::PRELUDE).lang;
        assert_eq!(
            call("parse_hex", &[Value::str("ff")]),
            Value::adt(lang.ok, vec![Value::int(255)])
        );
        assert_eq!(
            call("parse_hex", &[Value::str("0xFF")]),
            Value::adt(lang.ok, vec![Value::int(255)])
        );
        assert_eq!(
            call("parse_hex", &[Value::str(" 7ee45f400000 ")]),
            Value::adt(lang.ok, vec![Value::int(0x7ee45f400000)])
        );
        assert_eq!(
            call("parse_hex", &[Value::str("zz")]),
            Value::adt(lang.err, vec![Value::str("invalid digit found in string")])
        );
    }

    #[test]
    fn trim_start_and_end() {
        assert_eq!(
            call("trim_start", &[Value::str("  hello  ")]),
            Value::str("hello  ")
        );
        assert_eq!(
            call("trim_end", &[Value::str("  hello  ")]),
            Value::str("  hello")
        );
    }

    #[test]
    fn pad_right_pads_to_width() {
        assert_eq!(
            call("pad_right", &[Value::str("hi"), Value::int(5), Value::str(".")]),
            Value::str("hi...")
        );
        // Already at width — no padding.
        assert_eq!(
            call("pad_right", &[Value::str("hello"), Value::int(3), Value::str(".")]),
            Value::str("hello")
        );
    }

    #[test]
    fn find_returns_char_index() {
        let registry = registry();
        let lang = TestCx::new(&registry, crate::PRELUDE).lang;
        assert_eq!(
            call("find", &[Value::str("hello world"), Value::str("world")]),
            Value::adt(lang.some, vec![Value::int(6)])
        );
        assert_eq!(
            call("find", &[Value::str("hello"), Value::str("xyz")]),
            Value::adt(lang.none, vec![])
        );
    }

    #[test]
    fn rfind_returns_last_char_index() {
        let registry = registry();
        let lang = TestCx::new(&registry, crate::PRELUDE).lang;
        assert_eq!(
            call("rfind", &[Value::str("abcabc"), Value::str("abc")]),
            Value::adt(lang.some, vec![Value::int(3)])
        );
    }

    #[test]
    fn find_with_multibyte_chars() {
        // "αβγδ" — each Greek letter is 2 bytes, but 1 char.
        let registry = registry();
        let lang = TestCx::new(&registry, crate::PRELUDE).lang;
        assert_eq!(
            call("find", &[Value::str("αβγδ"), Value::str("γ")]),
            Value::adt(lang.some, vec![Value::int(2)])
        );
    }

    #[test]
    fn slice_with_positive_and_negative_indices() {
        assert_eq!(
            call("slice", &[Value::str("hello"), Value::int(1), Value::int(4)]),
            Value::str("ell")
        );
        // Negative indices count from end.
        assert_eq!(
            call("slice", &[Value::str("hello"), Value::int(0), Value::int(-1)]),
            Value::str("hell")
        );
        assert_eq!(
            call("slice", &[Value::str("hello"), Value::int(-3), Value::int(-1)]),
            Value::str("ll")
        );
    }

    #[test]
    fn chars_returns_list_of_single_chars() {
        assert_eq!(
            call("chars", &[Value::str("abc")]),
            Value::list(vec![Value::str("a"), Value::str("b"), Value::str("c")])
        );
    }

    #[test]
    fn bytes_returns_list_of_byte_values() {
        assert_eq!(
            call("bytes", &[Value::str("AB")]),
            Value::list(vec![Value::int(65), Value::int(66)])
        );
    }

    #[test]
    fn insert_at_char_position() {
        assert_eq!(
            call("insert", &[Value::str("helo"), Value::int(3), Value::str("l")]),
            Value::str("hello")
        );
    }

    #[test]
    fn remove_char_range() {
        assert_eq!(
            call("remove", &[Value::str("hello"), Value::int(1), Value::int(4)]),
            Value::str("ho")
        );
    }
}
