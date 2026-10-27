//! Methods of lists, which are shared and change in place, like Python's:
//! `let xs = [1, 2]; xs.push(3);`.

use std::sync::{Arc, Mutex};

use xur_runtime::{BuiltinType, Registry, RunResult, RuntimeError, Value, lock};

use crate::option;

type List = Arc<Mutex<Vec<Value>>>;

pub(crate) fn register(registry: &Registry) {
    registry
        .builtin(BuiltinType::List)
        .raw_function("len", |_, args| {
            let (list, []) = receiver(args)? else {
                return arity(0);
            };
            Ok(Value::Int(lock(list).len() as i64))
        })
        .raw_function("is_empty", |_, args| {
            let (list, []) = receiver(args)? else {
                return arity(0);
            };
            Ok(Value::Bool(lock(list).is_empty()))
        })
        .raw_function("push", |_, args| {
            let (list, [item]) = receiver(args)? else {
                return arity(1);
            };
            lock(list).push(item.clone());
            Ok(Value::Unit)
        })
        .raw_function("pop", |cx, args| {
            let (list, []) = receiver(args)? else {
                return arity(0);
            };
            let item = lock(list).pop();
            Ok(option(cx, item))
        })
        .raw_function("get", |cx, args| {
            let (list, [index]) = receiver(args)? else {
                return arity(1);
            };
            let item = usize::try_from(int(index)?)
                .ok()
                .and_then(|i| lock(list).get(i).cloned());
            Ok(option(cx, item))
        })
        .raw_function("first", |cx, args| {
            let (list, []) = receiver(args)? else {
                return arity(0);
            };
            let item = lock(list).first().cloned();
            Ok(option(cx, item))
        })
        .raw_function("last", |cx, args| {
            let (list, []) = receiver(args)? else {
                return arity(0);
            };
            let item = lock(list).last().cloned();
            Ok(option(cx, item))
        })
        .raw_function("insert", |_, args| {
            let (list, [index, item]) = receiver(args)? else {
                return arity(2);
            };
            let mut items = lock(list);
            let index = position(int(index)?, items.len() + 1, items.len())?;
            items.insert(index, item.clone());
            Ok(Value::Unit)
        })
        .raw_function("remove", |_, args| {
            let (list, [index]) = receiver(args)? else {
                return arity(1);
            };
            let mut items = lock(list);
            let index = position(int(index)?, items.len(), items.len())?;
            Ok(items.remove(index))
        })
        .raw_function("contains", |_, args| {
            let (list, [item]) = receiver(args)? else {
                return arity(1);
            };
            Ok(Value::Bool(lock(list).contains(item)))
        })
        .raw_function("clear", |_, args| {
            let (list, []) = receiver(args)? else {
                return arity(0);
            };
            lock(list).clear();
            Ok(Value::Unit)
        })
        .raw_function("reverse", |_, args| {
            let (list, []) = receiver(args)? else {
                return arity(0);
            };
            lock(list).reverse();
            Ok(Value::Unit)
        })
        .raw_function("extend", |_, args| {
            let (list, [Value::List(other)]) = receiver(args)? else {
                return Err(RuntimeError::new("`extend` takes a list"));
            };
            // Copy first: `xs.extend(xs)` locks the same list twice.
            let others = lock(other).clone();
            lock(list).extend(others);
            Ok(Value::Unit)
        })
        .function("to_hex", |bytes: Vec<u8>| {
            bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        })
        .raw_function("map", |cx, args| {
            let (list, [f]) = receiver(args)? else {
                return arity(1);
            };
            let items = lock(list).clone();
            let mut result = Vec::with_capacity(items.len());
            for item in items {
                result.push(cx.call(f, vec![item])?);
            }
            Ok(Value::list(result))
        })
        .raw_function("filter", |cx, args| {
            let (list, [f]) = receiver(args)? else {
                return arity(1);
            };
            let items = lock(list).clone();
            let mut result = Vec::new();
            for item in items {
                match cx.call(f, vec![item.clone()])? {
                    Value::Bool(true) => result.push(item),
                    Value::Bool(false) => {}
                    _ => return Err(RuntimeError::new("filter predicate must return a bool")),
                }
            }
            Ok(Value::list(result))
        })
        .raw_function("join", |cx, args| {
            let (list, [Value::Str(separator)]) = receiver(args)? else {
                return Err(RuntimeError::new("`join` takes a separator string"));
            };
            let items = lock(list).clone();
            let parts: Vec<_> = items.iter().map(|item| cx.display(item)).collect();
            Ok(Value::str(&parts.join(separator)))
        });
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
        Value::Int(i) => Ok(*i),
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
        let mut cx = TestCx::new(&registry);
        let xs = Value::list(vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
        let alias = xs.clone();
        call(&mut cx, "push", &[xs, Value::Int(4)]).unwrap();
        assert_eq!(items(&alias).len(), 4);
        assert_eq!(call(&mut cx, "len", &[alias]), Ok(Value::Int(4)));
    }

    #[test]
    fn pop_and_get_return_options() {
        let registry = registry();
        let mut cx = TestCx::new(&registry);
        let lang = cx.lang;
        let xs = Value::list(vec![Value::Int(1)]);
        assert_eq!(
            call(&mut cx, "get", &[xs.clone(), Value::Int(5)]),
            Ok(Value::adt(lang.none, vec![]))
        );
        assert_eq!(
            call(&mut cx, "pop", std::slice::from_ref(&xs)),
            Ok(Value::adt(lang.some, vec![Value::Int(1)]))
        );
        assert_eq!(
            call(&mut cx, "pop", &[xs]),
            Ok(Value::adt(lang.none, vec![]))
        );
    }

    #[test]
    fn insert_remove_and_bounds() {
        let registry = registry();
        let mut cx = TestCx::new(&registry);
        let xs = Value::list(vec![Value::Int(1), Value::Int(3)]);
        call(
            &mut cx,
            "insert",
            &[xs.clone(), Value::Int(1), Value::Int(2)],
        )
        .unwrap();
        assert_eq!(items(&xs), [Value::Int(1), Value::Int(2), Value::Int(3)]);
        assert_eq!(
            call(&mut cx, "remove", &[xs.clone(), Value::Int(0)]),
            Ok(Value::Int(1))
        );
        assert_eq!(
            call(&mut cx, "remove", &[xs, Value::Int(9)]),
            Err("index 9 is out of range for a list of length 2".to_owned())
        );
    }

    #[test]
    fn extend_with_itself_and_join() {
        let registry = registry();
        let mut cx = TestCx::new(&registry);
        let xs = Value::list(vec![Value::Int(1), Value::str("a")]);
        call(&mut cx, "extend", &[xs.clone(), xs.clone()]).unwrap();
        assert_eq!(
            call(&mut cx, "join", &[xs, Value::str("-")]),
            Ok(Value::str("1-a-1-a"))
        );
    }
}
