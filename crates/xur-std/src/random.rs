use rand::Rng;
use xur_runtime::{Registry, RuntimeError, Value, XurList, lock};

pub(crate) fn register(registry: &Registry) {
    registry
        .module("std::random")
        .named_function("int", &["min", "max"], |min: i64, max: i64| -> Result<i64, String> {
            if min > max {
                return Err(format!("`int`: min ({min}) must be <= max ({max})"));
            }
            Ok(rand::rng().random_range(min..=max))
        })
        .function("float", || rand::rng().random::<f64>())
        .function("bool", || rand::rng().random::<bool>())
        .named_raw_function("choice", &["list"], |_cx, args| {
            let [Value::List(list)] = args else {
                return Err(RuntimeError::new("`choice` takes a list"));
            };
            let items = lock(list);
            if items.is_empty() {
                return Err(RuntimeError::new("`choice` called on an empty list"));
            }
            let i = rand::rng().random_range(0..items.len());
            Ok(items[i].clone())
        })
        .function("shuffle", |list: XurList| -> Value {
            let mut items = lock(&list.0).clone();
            let mut rng = rand::rng();
            for i in (1..items.len()).rev() {
                let j = rng.random_range(0..=i);
                items.swap(i, j);
            }
            Value::list(items)
        })
        .named_function("bytes", &["len"], |len: usize| -> Vec<u8> {
            let mut buf = vec![0u8; len];
            rand::rng().fill(&mut buf[..]);
            buf
        });
}
