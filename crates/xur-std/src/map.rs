//! Methods of maps (`#{k: v}`), which are shared and change in place.

use std::sync::Arc;

use dashmap::DashMap;
use xur_runtime::{BuiltinType, MapKey, Registry, RunResult, RuntimeError, Value};

use crate::option;

type Map = Arc<DashMap<MapKey, Value>>;

pub(crate) fn register(registry: &Registry) {
    registry
        .builtin(BuiltinType::Map)
        .raw_function("len", |_, args| {
            let (map, []) = receiver(args)? else {
                return arity(0);
            };
            Ok(Value::Int(map.len() as i64))
        }).returns::<i64>()
        .raw_function("is_empty", |_, args| {
            let (map, []) = receiver(args)? else {
                return arity(0);
            };
            Ok(Value::Bool(map.is_empty()))
        }).returns::<bool>()
        .raw_function("get", |cx, args| {
            let (map, [key]) = receiver(args)? else {
                return arity(1);
            };
            let k = MapKey(key.clone());
            Ok(option(cx, map.get(&k).map(|v| v.value().clone())))
        })
        .raw_function("contains_key", |_, args| {
            let (map, [key]) = receiver(args)? else {
                return arity(1);
            };
            Ok(Value::Bool(map.contains_key(&MapKey(key.clone()))))
        }).returns::<bool>()
        .raw_function("insert", |_, args| {
            let (map, [key, value]) = receiver(args)? else {
                return arity(2);
            };
            map.insert(MapKey(key.clone()), value.clone());
            Ok(Value::Unit)
        })
        .raw_function("remove", |cx, args| {
            let (map, [key]) = receiver(args)? else {
                return arity(1);
            };
            let removed = map.remove(&MapKey(key.clone())).map(|(_, v)| v);
            Ok(option(cx, removed))
        })
        .raw_function("keys", |_, args| {
            let (map, []) = receiver(args)? else {
                return arity(0);
            };
            let keys: Vec<Value> = map.iter().map(|entry| entry.key().0.clone()).collect();
            Ok(Value::list(keys))
        })
        .raw_function("values", |_, args| {
            let (map, []) = receiver(args)? else {
                return arity(0);
            };
            let values: Vec<Value> = map.iter().map(|entry| entry.value().clone()).collect();
            Ok(Value::list(values))
        })
        .raw_function("entries", |_, args| {
            let (map, []) = receiver(args)? else {
                return arity(0);
            };
            let entries: Vec<Value> = map
                .iter()
                .map(|entry| Value::tuple(vec![entry.key().0.clone(), entry.value().clone()]))
                .collect();
            Ok(Value::list(entries))
        })
        .raw_function("clear", |_, args| {
            let (map, []) = receiver(args)? else {
                return arity(0);
            };
            map.clear();
            Ok(Value::Unit)
        });
}

fn receiver(args: &[Value]) -> RunResult<(&Map, &[Value])> {
    match args {
        [Value::Map(map), rest @ ..] => Ok((map, rest)),
        _ => Err(RuntimeError::new("expected a map")),
    }
}

fn arity(expected: usize) -> RunResult<Value> {
    Err(RuntimeError::new(format!(
        "expected {expected} argument(s)"
    )))
}
