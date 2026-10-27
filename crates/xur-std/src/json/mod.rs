//! `std::json`: JSON parsing and serialisation.

mod de;
mod ser;

use std::sync::Arc;

use xur_runtime::{Context, Record, Registry, RunResult, RuntimeError, Value, lock};
use xur_span::Symbol;

/// A newtype so `serde::Deserialize` and `serde::Serialize` can be
/// implemented for xur values without orphan issues.
pub(crate) struct JsonValue(pub Value);

impl JsonValue {
    /// JSON null, as xur's `None`.
    fn null() -> Self {
        // Constructed with a placeholder DefId; the real None ctor is set
        // by the `parse` function which has access to Context.
        Self(Value::Unit)
    }
}

pub(crate) fn register(registry: &Registry) {
    registry
        .module("std::json")
        .raw_function("parse", parse)
        .raw_function("stringify", stringify);
}

/// `std::json::parse(text) -> Result<value, str>`
fn parse(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [Value::Str(text)] = args else {
        return Err(RuntimeError::new("`json::parse` takes a single string"));
    };
    let res = match serde_json::from_str::<JsonValue>(text) {
        Ok(JsonValue(value)) => Ok(fixup_nulls(cx, &value)),
        Err(e) => Err(crate::error_text(e)),
    };
    Ok(crate::result(cx, res))
}

/// `std::json::stringify(value) -> Result<str, str>`
fn stringify(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [value] = args else {
        return Err(RuntimeError::new("`json::stringify` takes a single value"));
    };
    let prepared = prepare_for_json(cx, value)?;
    let res = serde_json::to_string(&JsonValue(prepared)).map_err(crate::error_text);
    Ok(crate::result(cx, res.map(|s| Value::str(&s))))
}

/// Converts Adt values to Records with named fields so the serializer
/// can emit JSON objects with keys instead of arrays.
fn prepare_for_json(cx: &mut dyn Context, value: &Value) -> RunResult<Value> {
    match value {
        Value::Adt(adt) => {
            if let Some(info) = cx.adt_info(adt.ctor) {
                let mut fields = Vec::with_capacity(info.fields.len());
                for (i, fi) in info.fields.iter().enumerate() {
                    let name = fi.name.unwrap_or_else(|| Symbol::intern(&i.to_string()));
                    let val = adt.fields.get(i).cloned().unwrap_or(Value::Unit);
                    fields.push((name, prepare_for_json(cx, &val)?));
                }
                Ok(Value::Record(Arc::new(Record::new(fields))))
            } else {
                // No field info (e.g. None, Ok, Err) — keep as Adt.
                Ok(value.clone())
            }
        }
        Value::List(items) => {
            let items = lock(items);
            let prepared: RunResult<Vec<_>> =
                items.iter().map(|v| prepare_for_json(cx, v)).collect();
            Ok(Value::list(prepared?))
        }
        Value::Record(record) => {
            let mut fields = Vec::with_capacity(record.fields.len());
            for (name, v) in &record.fields {
                fields.push((*name, prepare_for_json(cx, v)?));
            }
            Ok(Value::Record(Arc::new(Record {
                ty: record.ty,
                variant: record.variant,
                fields,
            })))
        }
        Value::Tuple(items) => {
            let prepared: RunResult<Vec<_>> =
                items.iter().map(|v| prepare_for_json(cx, v)).collect();
            Ok(Value::tuple(prepared?))
        }
        _ => Ok(value.clone()),
    }
}

/// The deserializer cannot build `None` ADTs (it doesn't have a Context),
/// so they are left as `Unit` and fixed up here.
fn fixup_nulls(cx: &dyn Context, value: &Value) -> Value {
    match value {
        Value::Unit => crate::option(cx, None),
        Value::List(items) => {
            let items = lock(items);
            Value::list(items.iter().map(|v| fixup_nulls(cx, v)).collect())
        }
        Value::Record(record) => {
            let fields = record
                .fields
                .iter()
                .map(|(name, v)| (*name, fixup_nulls(cx, v)))
                .collect();
            Value::Record(
                Record {
                    ty: record.ty,
                    variant: record.variant,
                    fields,
                }
                .into(),
            )
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use xur_runtime::testing::{TestCx, fn_id};
    use xur_runtime::{Record, lock};
    use xur_span::Symbol;

    use super::*;

    fn registry() -> Registry {
        let registry = Registry::new();
        register(&registry);
        registry
    }

    fn call_parse(cx: &mut TestCx<'_>, json: &str) -> Value {
        let id = fn_id(cx.registry, "std::json::parse");
        cx.call(id, &[Value::str(json)]).unwrap()
    }

    fn call_stringify(cx: &mut TestCx<'_>, value: Value) -> Value {
        let id = fn_id(cx.registry, "std::json::stringify");
        cx.call(id, &[value]).unwrap()
    }

    /// Unwraps an Ok(value) from a xur Result ADT.
    fn unwrap_ok(cx: &TestCx<'_>, result: &Value) -> Value {
        let Value::Adt(adt) = result else {
            panic!("expected a Result ADT, got {result:?}");
        };
        assert_eq!(adt.ctor, cx.lang.ok, "expected Ok, got Err");
        adt.fields[0].clone()
    }

    /// Unwraps an Err(message) from a xur Result ADT.
    fn unwrap_err(cx: &TestCx<'_>, result: &Value) -> String {
        let Value::Adt(adt) = result else {
            panic!("expected a Result ADT, got {result:?}");
        };
        assert_eq!(adt.ctor, cx.lang.err, "expected Err, got Ok");
        match &adt.fields[0] {
            Value::Str(s) => s.to_string(),
            other => panic!("expected string error, got {other:?}"),
        }
    }

    fn field(record: &Value, name: &str) -> Value {
        let Value::Record(r) = record else {
            panic!("expected Record, got {record:?}");
        };
        r.field(Symbol::intern(name))
            .unwrap_or_else(|| panic!("no field `{name}`"))
            .clone()
    }

    #[test]
    fn parse_object() {
        let reg = registry();
        let mut cx = TestCx::new(&reg);
        let result = call_parse(&mut cx, r#"{"name": "Alice", "age": 30}"#);
        let value = unwrap_ok(&cx, &result);

        assert_eq!(field(&value, "name"), Value::str("Alice"));
        assert_eq!(field(&value, "age"), Value::Int(30));
    }

    #[test]
    fn parse_nested_object() {
        let reg = registry();
        let mut cx = TestCx::new(&reg);
        let result = call_parse(
            &mut cx,
            r#"{"user": {"name": "Bob"}, "scores": [10, 20]}"#,
        );
        let value = unwrap_ok(&cx, &result);

        assert_eq!(field(&field(&value, "user"), "name"), Value::str("Bob"));
        let Value::List(scores) = field(&value, "scores") else {
            panic!("expected list");
        };
        let scores = lock(&scores);
        assert_eq!(*scores, vec![Value::Int(10), Value::Int(20)]);
    }

    #[test]
    fn parse_null_becomes_none() {
        let reg = registry();
        let mut cx = TestCx::new(&reg);
        let result = call_parse(&mut cx, r#"{"x": null}"#);
        let value = unwrap_ok(&cx, &result);
        let x = field(&value, "x");
        let Value::Adt(adt) = &x else {
            panic!("expected None ADT for null, got {x:?}");
        };
        assert_eq!(adt.ctor, cx.lang.none);
    }

    #[test]
    fn parse_array() {
        let reg = registry();
        let mut cx = TestCx::new(&reg);
        let result = call_parse(&mut cx, r#"[1, "two", true]"#);
        let value = unwrap_ok(&cx, &result);
        let Value::List(items) = &value else {
            panic!("expected list, got {value:?}");
        };
        let items = lock(items);
        assert_eq!(items[0], Value::Int(1));
        assert_eq!(items[1], Value::str("two"));
        assert_eq!(items[2], Value::Bool(true));
    }

    #[test]
    fn parse_scalars() {
        let reg = registry();
        let mut cx = TestCx::new(&reg);

        let r = call_parse(&mut cx, "42");
        assert_eq!(unwrap_ok(&cx, &r), Value::Int(42));

        let r = call_parse(&mut cx, "3.14");
        assert_eq!(unwrap_ok(&cx, &r), Value::Float(3.14));

        let r = call_parse(&mut cx, "true");
        assert_eq!(unwrap_ok(&cx, &r), Value::Bool(true));

        let r = call_parse(&mut cx, r#""hello""#);
        assert_eq!(unwrap_ok(&cx, &r), Value::str("hello"));
    }

    #[test]
    fn parse_invalid_json_returns_err() {
        let reg = registry();
        let mut cx = TestCx::new(&reg);
        let result = call_parse(&mut cx, "not json");
        let msg = unwrap_err(&cx, &result);
        assert!(msg.contains("expected"), "error was: {msg}");
    }

    #[test]
    fn stringify_object() {
        let reg = registry();
        let mut cx = TestCx::new(&reg);
        let record = Value::Record(
            Record::new(vec![
                (Symbol::intern("a"), Value::Int(1)),
                (Symbol::intern("b"), Value::str("two")),
            ])
            .into(),
        );
        let result = call_stringify(&mut cx, record);
        let json = unwrap_ok(&cx, &result);
        let Value::Str(s) = &json else {
            panic!("expected string");
        };
        let parsed: serde_json::Value = serde_json::from_str(s).unwrap();
        assert_eq!(parsed["a"], 1);
        assert_eq!(parsed["b"], "two");
    }

    #[test]
    fn stringify_list() {
        let reg = registry();
        let mut cx = TestCx::new(&reg);
        let list = Value::list(vec![Value::Int(1), Value::Bool(false)]);
        let result = call_stringify(&mut cx, list);
        let json = unwrap_ok(&cx, &result);
        assert_eq!(json, Value::str("[1,false]"));
    }

    #[test]
    fn roundtrip() {
        let reg = registry();
        let mut cx = TestCx::new(&reg);
        let input = r#"{"name":"Alice","tags":["a","b"],"score":99}"#;
        let r = call_parse(&mut cx, input);
        let parsed = unwrap_ok(&cx, &r);
        let r = call_stringify(&mut cx, parsed);
        let stringified = unwrap_ok(&cx, &r);
        let Value::Str(s) = &stringified else {
            panic!("expected string");
        };
        let a: serde_json::Value = serde_json::from_str(input).unwrap();
        let b: serde_json::Value = serde_json::from_str(s).unwrap();
        assert_eq!(a, b);
    }
}
