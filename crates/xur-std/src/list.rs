//! Methods of lists, which are shared and change in place, like Python's:
//! `let xs = [1, 2]; xs.push(3);`.

use std::cell::Cell;
use std::sync::{Arc, Mutex};

use xur_runtime::{
    BuiltinType, IntValue, Opt, Raise, Registry, RunResult, RuntimeError, Value, XurList, lock,
    with_pair,
};
use xur_runtime::registry::{ElemHint, XurType};

use crate::option;

type List = Arc<Mutex<Vec<Value>>>;

pub(crate) fn register(registry: &Registry) {
    registry
        .builtin(BuiltinType::List)
        .method("len", |list: &[Value]| -> i64 { list.len() as i64 })
        .method("is_empty", |list: &[Value]| -> bool { list.is_empty() })
        .method_mut("push", |list: &mut Vec<Value>, item: Value| {
            list.push(item);
        })
        .method_mut("pop", |list: &mut Vec<Value>| -> Opt<Value> { Opt(list.pop()) })
        .method("get", |list: &[Value], index: i64| -> Opt<Value> {
            Opt(usize::try_from(index).ok().and_then(|i| list.get(i).cloned()))
        })
        .method("first", |list: &[Value]| -> Opt<Value> { Opt(list.first().cloned()) })
        .method("last", |list: &[Value]| -> Opt<Value> { Opt(list.last().cloned()) })
        .raw_function_returns_xur("insert", XurType::UNIT, |_, args| {
            let (list, [index, item]) = receiver(args)? else {
                return arity(2);
            };
            let mut items = lock(list);
            let index = position(int(index)?, items.len() + 1, items.len())?;
            items.insert(index, item.clone());
            Ok(Value::Unit)
        })
        .method_mut("remove", |list: &mut Vec<Value>, index: i64| -> Raise<Value> {
            Raise((|| {
                let index = position(index, list.len(), list.len())?;
                Ok(list.remove(index))
            })())
        })
        .method("contains", |list: &[Value], item: Value| -> bool {
            list.contains(&item)
        })
        .method_mut("clear", |list: &mut Vec<Value>| { list.clear(); })
        .method_mut("reverse", |list: &mut Vec<Value>| { list.reverse(); })
        .method("extend", |list: &XurList, other: XurList| {
            // Copy first: `xs.extend(xs)` locks the same list twice.
            let others = lock(&other.0).clone();
            lock(&list.0).extend(others);
        })
        .method("to_hex", |list: &[Value]| -> Result<String, String> {
            // Validate each element is a byte, erroring on a non-int or
            // out-of-range value instead of silently truncating `as u8`.
            list.iter()
                .map(|v| match v {
                    Value::Int(i) => u8::try_from(i.to_i128())
                        .map(|b| format!("{b:02x}"))
                        .map_err(|_| format!("to_hex: {} is not a byte (0..=255)", i.to_i128())),
                    _ => Err("to_hex: list contains a non-integer element".to_string()),
                })
                .collect::<Result<String, String>>()
        })
        .raw_function_returns_xur("map", XurType::LIST, |cx, args| {
            let (list, [f]) = receiver(args)? else {
                return arity(1);
            };
            let items = lock(list).clone();
            let mut result = Vec::with_capacity(items.len());
            for item in items {
                result.push(cx.call(f, &[item])?);
            }
            Ok(Value::list(result))
        })
        .raw_function_returns_xur("filter", XurType::List(Some(ElemHint::Param(0))), |cx, args| {
            let (list, [f]) = receiver(args)? else {
                return arity(1);
            };
            let items = lock(list).clone();
            let mut result = Vec::new();
            for item in items {
                match cx.call(f, std::slice::from_ref(&item))? {
                    Value::Bool(true) => result.push(item),
                    Value::Bool(false) => {}
                    _ => return Err(RuntimeError::new("filter predicate must return a bool")),
                }
            }
            Ok(Value::list(result))
        })
        .raw_function_returns::<String>("join", |cx, args| {
            let (list, [Value::Str(separator)]) = receiver(args)? else {
                return Err(RuntimeError::new("`join` takes a separator string"));
            };
            let items = lock(list).clone();
            let parts: Vec<_> = items.iter().map(|item| cx.display(item)).collect();
            Ok(Value::str(&parts.join(separator)))
        })
        // -- Callback-based methods ------------------------------------------
        .raw_function_returns_xur("sort_by", XurType::LIST, |cx, args| {
            let (list, [f]) = receiver(args)? else {
                return arity(1);
            };
            let mut items = lock(list).clone();
            let failed = Cell::new(false);
            let error: Cell<Option<RuntimeError>> = Cell::new(None);
            items.sort_by(|a, b| {
                if failed.get() {
                    return std::cmp::Ordering::Equal;
                }
                match cx.call(f, &[a.clone(), b.clone()]) {
                    Ok(Value::Int(n)) => {
                        let v = n.to_i128();
                        if v < 0 {
                            std::cmp::Ordering::Less
                        } else if v > 0 {
                            std::cmp::Ordering::Greater
                        } else {
                            std::cmp::Ordering::Equal
                        }
                    }
                    Ok(_) => {
                        failed.set(true);
                        error.set(Some(RuntimeError::new(
                            "sort_by comparator must return an Int (-1, 0, or 1)",
                        )));
                        std::cmp::Ordering::Equal
                    }
                    Err(e) => {
                        failed.set(true);
                        error.set(Some(e));
                        std::cmp::Ordering::Equal
                    }
                }
            });
            if let Some(e) = error.take() {
                return Err(e);
            }
            Ok(Value::list(items))
        })
        .raw_function("find", |cx, args| {
            let (list, [f]) = receiver(args)? else {
                return arity(1);
            };
            let items = lock(list).clone();
            let mut found = None;
            for item in items {
                match cx.call(f, std::slice::from_ref(&item))? {
                    Value::Bool(true) => {
                        found = Some(item);
                        break;
                    }
                    Value::Bool(false) => {}
                    _ => return Err(RuntimeError::new("find predicate must return a bool")),
                }
            }
            Ok(option(cx, found))
        })
        .raw_function("find_index", |cx, args| {
            let (list, [f]) = receiver(args)? else {
                return arity(1);
            };
            let items = lock(list).clone();
            let mut found = None;
            for (i, item) in items.into_iter().enumerate() {
                match cx.call(f, std::slice::from_ref(&item))? {
                    Value::Bool(true) => {
                        found = Some(Value::int(i as i64));
                        break;
                    }
                    Value::Bool(false) => {}
                    _ => {
                        return Err(RuntimeError::new(
                            "find_index predicate must return a bool",
                        ))
                    }
                }
            }
            Ok(option(cx, found))
        })
        .raw_function_returns::<bool>("any", |cx, args| {
            let (list, [f]) = receiver(args)? else {
                return arity(1);
            };
            let items = lock(list).clone();
            for item in items {
                match cx.call(f, std::slice::from_ref(&item))? {
                    Value::Bool(true) => return Ok(Value::Bool(true)),
                    Value::Bool(false) => {}
                    _ => return Err(RuntimeError::new("any predicate must return a bool")),
                }
            }
            Ok(Value::Bool(false))
        })
        .raw_function_returns::<bool>("all", |cx, args| {
            let (list, [f]) = receiver(args)? else {
                return arity(1);
            };
            let items = lock(list).clone();
            for item in items {
                match cx.call(f, std::slice::from_ref(&item))? {
                    Value::Bool(true) => {}
                    Value::Bool(false) => return Ok(Value::Bool(false)),
                    _ => return Err(RuntimeError::new("all predicate must return a bool")),
                }
            }
            Ok(Value::Bool(true))
        })
        .raw_function("reduce", |cx, args| {
            let (list, [f]) = receiver(args)? else {
                return arity(1);
            };
            let items = lock(list).clone();
            let mut iter = items.into_iter();
            let Some(mut acc) = iter.next() else {
                return Err(RuntimeError::new("reduce on an empty list"));
            };
            for item in iter {
                acc = cx.call(f, &[acc, item])?;
            }
            Ok(acc)
        })
        .raw_function_returns_xur("flat_map", XurType::LIST, |cx, args| {
            let (list, [f]) = receiver(args)? else {
                return arity(1);
            };
            let items = lock(list).clone();
            let mut result = Vec::new();
            for item in items {
                let mapped = cx.call(f, &[item])?;
                match mapped {
                    Value::List(inner) => result.extend(lock(&inner).clone()),
                    other => result.push(other),
                }
            }
            Ok(Value::list(result))
        })
        // -- Non-callback methods --------------------------------------------
        .raw_function_returns_xur("sort", XurType::LIST, |_, args| {
            let (list, []) = receiver(args)? else {
                return arity(0);
            };
            let mut items = lock(list).clone();
            let err = Cell::new(false);
            items.sort_by(|a, b| {
                if err.get() {
                    return std::cmp::Ordering::Equal;
                }
                match compare(a, b) {
                    Some(ord) => ord,
                    None => {
                        err.set(true);
                        std::cmp::Ordering::Equal
                    }
                }
            });
            if err.get() {
                return Err(RuntimeError::new(
                    "sort: elements are not comparable (must be all Int, all Float, or all Str)",
                ));
            }
            Ok(Value::list(items))
        })
        .method("enumerate", |list: &[Value]| -> Value {
            Value::list(
                list.iter()
                    .enumerate()
                    .map(|(i, item)| Value::list(vec![Value::int(i as i64), item.clone()]))
                    .collect(),
            )
        })
        .method("flatten", |list: &[Value]| -> Value {
            let mut result = Vec::new();
            for item in list {
                match item {
                    Value::List(inner) => result.extend(lock(inner).clone()),
                    other => result.push(other.clone()),
                }
            }
            Value::list(result)
        })
        .method("slice", |list: &[Value], start: i64, end: i64| -> Value {
            let len = list.len();
            let start = resolve_index(start, len);
            let end = resolve_index(end, len);
            if start >= end {
                return Value::list(Vec::new());
            }
            Value::list(list[start..end].to_vec())
        })
        .method("dedup", |list: &[Value]| -> Value {
            let mut result: Vec<Value> = Vec::new();
            for item in list {
                if result.last() != Some(item) {
                    result.push(item.clone());
                }
            }
            Value::list(result)
        })
        .method("sum", |items: &[Value]| -> Raise<Value> {
            Raise((|| {
                let Some(first) = items.first() else {
                    return Err(RuntimeError::new("sum on an empty list"));
                };
                match first {
                    Value::Int(_) => {
                        let mut total: i128 = 0;
                        for item in items {
                            match item {
                                Value::Int(n) => total += n.to_i128(),
                                _ => return Err(RuntimeError::new("sum: mixed types")),
                            }
                        }
                        // Prefer i64, then u64, so a lone large u64 still sums.
                        i64::try_from(total)
                            .map(Value::int)
                            .or_else(|_| u64::try_from(total).map(|u| Value::Int(IntValue::U64(u))))
                            .map_err(|_| RuntimeError::new("sum: integer overflow"))
                    }
                    Value::Float(_) => {
                        let mut total: f64 = 0.0;
                        for item in items {
                            match item {
                                Value::Float(n) => total += n.as_f64(),
                                _ => return Err(RuntimeError::new("sum: mixed types")),
                            }
                        }
                        Ok(Value::float(total))
                    }
                    _ => Err(RuntimeError::new("sum: elements must be Int or Float")),
                }
            })())
        })
        .method("min", |items: &[Value]| -> Raise<Opt<Value>> {
            Raise((|| {
                let Some((first, rest)) = items.split_first() else {
                    return Ok(Opt(None));
                };
                let mut best = first;
                for item in rest {
                    match compare(item, best) {
                        Some(std::cmp::Ordering::Less) => best = item,
                        Some(_) => {}
                        None => return Err(RuntimeError::new("min: elements are not comparable")),
                    }
                }
                Ok(Opt(Some(best.clone())))
            })())
        })
        .method("max", |items: &[Value]| -> Raise<Opt<Value>> {
            Raise((|| {
                let Some((first, rest)) = items.split_first() else {
                    return Ok(Opt(None));
                };
                let mut best = first;
                for item in rest {
                    match compare(item, best) {
                        Some(std::cmp::Ordering::Greater) => best = item,
                        Some(_) => {}
                        None => return Err(RuntimeError::new("max: elements are not comparable")),
                    }
                }
                Ok(Opt(Some(best.clone())))
            })())
        })
        .method("zip", |list: &XurList, other: XurList| -> Value {
            // with_pair locks once when `xs.zip(xs)` aliases, twice otherwise.
            with_pair(&list.0, &other.0, |left, right| {
                let result = left
                    .iter()
                    .zip(right.iter())
                    .map(|(a, b)| Value::list(vec![a.clone(), b.clone()]))
                    .collect();
                Value::list(result)
            })
        });
}

fn compare(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Value::Int(a), Value::Int(b)) => Some(a.cmp(b)),
        (Value::Float(a), Value::Float(b)) => a.as_f64().partial_cmp(&b.as_f64()),
        (Value::Str(a), Value::Str(b)) => Some(a.cmp(b)),
        _ => None,
    }
}

fn resolve_index(index: i64, len: usize) -> usize {
    if index < 0 {
        len.saturating_sub((-index) as usize)
    } else {
        (index as usize).min(len)
    }
}

/// Splits a method call's arguments into the list and the rest.
fn receiver(args: &[Value]) -> RunResult<(&List, &[Value])> {
    match args {
        [Value::List(list), rest @ ..] => Ok((list, rest)),
        _ => Err(RuntimeError::new("expected a list")),
    }
}

fn arity(expected: usize) -> RunResult<Value> {
    Err(RuntimeError::new(format!(
        "expected {expected} argument(s)"
    )))
}

fn int(value: &Value) -> RunResult<i64> {
    match value {
        Value::Int(i) => i.to_i64().ok_or_else(|| {
            RuntimeError::new(format!("{i} does not fit in i64"))
        }),
        other => Err(RuntimeError::new(format!(
            "expected an integer index, found {}",
            other.kind()
        ))),
    }
}

/// `index` as a position below `limit` in a list of length `len`: inserting
/// may use one position past the end, removing may not.
fn position(index: i64, limit: usize, len: usize) -> RunResult<usize> {
    usize::try_from(index)
        .ok()
        .filter(|&i| i < limit)
        .ok_or_else(|| {
            RuntimeError::new(format!(
                "index {index} is out of range for a list of length {len}"
            ))
        })
}

#[cfg(test)]
mod tests {
    use xur_runtime::testing::TestCx;
    use xur_runtime::{BuiltinType, Value, lock};

    use crate::registry;

    fn call(cx: &mut TestCx<'_>, name: &str, args: &[Value]) -> Result<Value, String> {
        let id = cx
            .registry
            .builtin_method(BuiltinType::List, name)
            .expect("method exists");
        cx.call(id, args).map_err(|e| e.message)
    }

    fn items(list: &Value) -> Vec<Value> {
        let Value::List(items) = list else {
            unreachable!()
        };
        lock(items).clone()
    }

    #[test]
    fn push_changes_the_list_in_place() {
        let registry = registry();
        let mut cx = TestCx::new(&registry, crate::PRELUDE);
        let xs = Value::list(vec![Value::int(1), Value::int(2), Value::int(3)]);
        let alias = xs.clone();
        call(&mut cx, "push", &[xs, Value::int(4)]).unwrap();
        assert_eq!(items(&alias).len(), 4);
        assert_eq!(call(&mut cx, "len", &[alias]), Ok(Value::int(4)));
    }

    #[test]
    fn pop_and_get_return_options() {
        let registry = registry();
        let mut cx = TestCx::new(&registry, crate::PRELUDE);
        let lang = cx.lang;
        let xs = Value::list(vec![Value::int(1)]);
        assert_eq!(
            call(&mut cx, "get", &[xs.clone(), Value::int(5)]),
            Ok(Value::adt(lang.none, vec![]))
        );
        assert_eq!(
            call(&mut cx, "pop", std::slice::from_ref(&xs)),
            Ok(Value::adt(lang.some, vec![Value::int(1)]))
        );
        assert_eq!(
            call(&mut cx, "pop", &[xs]),
            Ok(Value::adt(lang.none, vec![]))
        );
    }

    #[test]
    fn insert_remove_and_bounds() {
        let registry = registry();
        let mut cx = TestCx::new(&registry, crate::PRELUDE);
        let xs = Value::list(vec![Value::int(1), Value::int(3)]);
        call(
            &mut cx,
            "insert",
            &[xs.clone(), Value::int(1), Value::int(2)],
        )
        .unwrap();
        assert_eq!(items(&xs), [Value::int(1), Value::int(2), Value::int(3)]);
        assert_eq!(
            call(&mut cx, "remove", &[xs.clone(), Value::int(0)]),
            Ok(Value::int(1))
        );
        assert_eq!(
            call(&mut cx, "remove", &[xs, Value::int(9)]),
            Err("index 9 is out of range for a list of length 2".to_owned())
        );
    }

    #[test]
    fn extend_with_itself_and_join() {
        let registry = registry();
        let mut cx = TestCx::new(&registry, crate::PRELUDE);
        let xs = Value::list(vec![Value::int(1), Value::str("a")]);
        call(&mut cx, "extend", &[xs.clone(), xs.clone()]).unwrap();
        assert_eq!(
            call(&mut cx, "join", &[xs, Value::str("-")]),
            Ok(Value::str("1-a-1-a"))
        );
    }
}
