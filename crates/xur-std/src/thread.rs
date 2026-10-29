//! `std::thread`: running code in parallel.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use facet::Facet;
use xur_runtime::convert::into_value;
use xur_runtime::{Registry, RunResult, RuntimeError, Value, lock};

/// A running thread. Joining it gives the result of its function.
#[derive(Facet, Clone)]
#[facet(opaque)]
pub struct JoinHandle(Arc<Mutex<Option<thread::JoinHandle<RunResult<Value>>>>>);

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::thread", |module| {
        module
            .named_raw_function_returns::<JoinHandle>("spawn", &["f"], |cx, args| {
                let [f] = args else {
                    return Err(RuntimeError::new("`spawn` takes a function or closure"));
                };
                let handle = cx.spawn(f.clone())?;
                into_value(cx, JoinHandle(Arc::new(Mutex::new(Some(handle)))))
            })
            .named_function("sleep", &["millis"], |millis: u64| {
                thread::sleep(Duration::from_millis(millis))
            })
            .ty::<JoinHandle>("JoinHandle")
            .raw_function("join", |_, args| {
                let [Value::Native(object)] = args else {
                    return Err(RuntimeError::new("`join` is a method of `JoinHandle`"));
                };
                let handle = object
                    .with(|handle: &JoinHandle| lock(&handle.0).take())
                    .flatten()
                    .ok_or_else(|| RuntimeError::new("the thread was already joined"))?;
                // A runtime error in the thread is one in the joining thread.
                handle
                    .join()
                    .map_err(|_| RuntimeError::new("the thread panicked"))?
            });
    });
}
