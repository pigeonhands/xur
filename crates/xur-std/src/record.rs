//! Methods of records (named-field values from native types or layouts).

use std::sync::Arc;

use xur_runtime::{BuiltinType, Record, Registry, RunResult, RuntimeError, Value};
use xur_span::Symbol;

use crate::option;

fn sym(name: &str) -> Symbol {
    Symbol::intern(name)
}

pub(crate) fn register(registry: &Registry) {
    registry
        .builtin(BuiltinType::Record)
        .raw_function("get_field", |cx, args| {
            let (record, [Value::Str(name)]) = receiver(args)? else {
                return Err(RuntimeError::new("`get_field` takes a field name string"));
            };
            Ok(option(cx, record.field(sym(name)).cloned()))
        })
        .raw_function("set_field", |_, args| {
            let (record, [Value::Str(name), value]) = receiver(args)? else {
                return Err(RuntimeError::new(
                    "`set_field` takes a field name and a value",
                ));
            };
            let s = sym(name);
            let mut fields = record.fields.clone();
            if let Some(slot) = fields.iter_mut().find(|(n, _)| *n == s) {
                slot.1 = value.clone();
            } else {
                fields.push((s, value.clone()));
            }
            Ok(Value::Record(Arc::new(Record {
                fields,
                ..Record::clone(record)
            })))
        })
        .raw_function("get_method", |cx, args| {
            let (record, [Value::Str(name)]) = receiver(args)? else {
                return Err(RuntimeError::new(
                    "`get_method` takes a method name string",
                ));
            };
            let value = record.field(sym(name)).filter(|v| is_callable(v)).cloned();
            Ok(option(cx, value))
        })
        .raw_function("set_method", |_, args| {
            let (record, [Value::Str(name), value]) = receiver(args)? else {
                return Err(RuntimeError::new(
                    "`set_method` takes a method name and a function",
                ));
            };
            if !is_callable(value) {
                return Err(RuntimeError::new(format!(
                    "`set_method` value must be callable, got {}",
                    value.kind(),
                )));
            }
            let s = sym(name);
            let mut fields = record.fields.clone();
            if let Some(slot) = fields.iter_mut().find(|(n, _)| *n == s) {
                slot.1 = value.clone();
            } else {
                fields.push((s, value.clone()));
            }
            Ok(Value::Record(Arc::new(Record {
                fields,
                ..Record::clone(record)
            })))
        })
        .raw_function("fields", |_, args| {
            let (record, []) = receiver(args)? else {
                return Err(RuntimeError::new("`fields` takes no arguments"));
            };
            let names: Vec<Value> = record
                .fields
                .iter()
                .map(|(name, _)| Value::str(name.as_str()))
                .collect();
            Ok(Value::list(names))
        });
}

fn receiver(args: &[Value]) -> RunResult<(&Record, &[Value])> {
    match args {
        [Value::Record(record), rest @ ..] => Ok((record, rest)),
        _ => Err(RuntimeError::new("expected a record")),
    }
}

fn is_callable(value: &Value) -> bool {
    matches!(
        value,
        Value::Fn(_) | Value::Closure(_) | Value::NativeFn(_) | Value::Ctor(_)
    )
}
