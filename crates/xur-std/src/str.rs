//! Methods of strings.

use xur_runtime::{BuiltinType, Registry, XurStr};

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
        .method("len", |s: &str| s.chars().count() as i64)
        .method("is_empty", |s: &str| s.is_empty())
        .method("ord", |s: &str| -> Result<u32, String> {
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Ok(c as u32),
                _ => Err(format!("`ord` expects a single character, got {}-char string", s.chars().count())),
            }
        })
        .method("to_upper", |s: &str| s.to_uppercase())
        .method("to_lower", |s: &str| s.to_lowercase())
        .method("trim", |s: &str| s.trim().to_owned())
        .method("trim_start", |s: &str| s.trim_start().to_owned())
        .method("trim_end", |s: &str| s.trim_end().to_owned())
        .method("contains", |s: &str, part: XurStr| s.contains(&*part))
        .method("starts_with", |s: &str, prefix: XurStr| {
            s.starts_with(&*prefix)
        })
        .method("ends_with", |s: &str, suffix: XurStr| {
            s.ends_with(&*suffix)
        })
        .method("replace", |s: &str, from: XurStr, to: XurStr| {
            s.replace(&*from, &to)
        })
        .method("split", |s: &str, separator: XurStr| {
            s.split(&*separator).map(str::to_owned).collect::<Vec<_>>()
        })
        .method("lines", |s: &str| {
            s.lines().map(str::to_owned).collect::<Vec<_>>()
        })
        .method("repeat", |s: &str, n: u32| s.repeat(n as usize))
        .method("pad_left", |s: &str, width: usize, fill: XurStr| {
            pad_left(s, width, &fill)
        })
        .method("pad_right", |s: &str, width: usize, fill: XurStr| {
            pad_right(s, width, &fill)
        })
        .method("slice", |s: &str, start: i64, end: i64| {
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
        .method("chars", |s: &str| {
            s.chars().map(|c| c.to_string()).collect::<Vec<_>>()
        })
        .method("bytes", |s: &str| {
            s.bytes().map(|b| b as i64).collect::<Vec<i64>>()
        })
        .method("insert", |s: &str, index: i64, substr: XurStr| {
            let chars: Vec<char> = s.chars().collect();
            let len = chars.len();
            let pos = resolve_index(index, len);
            let byte_pos = chars[..pos].iter().map(|c| c.len_utf8()).sum::<usize>();
            let mut result = s.to_owned();
            result.insert_str(byte_pos, &substr);
            result
        })
        .method("remove", |s: &str, start: i64, end: i64| {
            let mut chars: Vec<char> = s.chars().collect();
            let len = chars.len();
            let start = resolve_index(start, len);
            let end = resolve_index(end, len);
            if start < end {
                chars.drain(start..end);
            }
            chars.into_iter().collect::<String>()
        })
        .method("parse_int", |s: &str| {
            s.trim().parse::<i64>().map_err(|e| e.to_string())
        })
        .method("parse_hex", |s: &str| {
            i64::from_str_radix(s.trim().trim_start_matches("0x").trim_start_matches("0X"), 16)
                .map_err(|e| e.to_string())
        })
        .method("parse_float", |s: &str| {
            s.trim().parse::<f64>().map_err(|e| e.to_string())
        })
        .method("find", |s: &str, needle: XurStr| -> Option<i64> {
            s.find(&*needle).map(|byte_offset| s[..byte_offset].chars().count() as i64)
        })
        .method("rfind", |s: &str, needle: XurStr| -> Option<i64> {
            s.rfind(&*needle).map(|byte_offset| s[..byte_offset].chars().count() as i64)
        });
}

/// Resolve a possibly-negative char index into a clamped `usize`.
fn resolve_index(index: i64, len: usize) -> usize {
    if index < 0 {
        len.saturating_sub((-index) as usize)
    } else {
        (index as usize).min(len)
    }
}

fn pad_left(s: &str, width: usize, fill: &str) -> String {
    let missing = width.saturating_sub(s.chars().count());
    let padding: String = fill.chars().cycle().take(missing).collect();
    padding + s
}

fn pad_right(s: &str, width: usize, fill: &str) -> String {
    let missing = width.saturating_sub(s.chars().count());
    let padding: String = fill.chars().cycle().take(missing).collect();
    let mut result = s.to_owned();
    result.push_str(&padding);
    result
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
