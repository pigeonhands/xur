use rand::Rng;
use xur_runtime::{Registry, RuntimeError, Value, lock};

pub(crate) fn register(registry: &Registry) {
    registry
        .module("std::random")
        .function("int", |min: i64, max: i64| {
            rand::rng().random_range(min..=max)
        })
        .function("float", || rand::rng().random::<f64>())
        .function("bool", || rand::rng().random::<bool>())
        .raw_function("choice", |_cx, args| {
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
        .raw_function("shuffle", |_cx, args| {
            let [Value::List(list)] = args else {
                return Err(RuntimeError::new("`shuffle` takes a list"));
            };
            let mut items = lock(list).clone();
            let mut rng = rand::rng();
            for i in (1..items.len()).rev() {
                let j = rng.random_range(0..=i);
                items.swap(i, j);
            }
            Ok(Value::list(items))
        })
        .function("bytes", |len: i64| {
            let mut buf = vec![0u8; len as usize];
            rand::rng().fill(&mut buf[..]);
            buf
        });
}
