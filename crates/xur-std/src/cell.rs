//! `std::cell`: shared mutable values for cross-closure state.

use std::sync::{Arc, RwLock};

use facet::Facet;
use xur_runtime::{NativeObject, Registry, RunResult, RuntimeError, Value};

#[derive(Facet, Clone)]
#[facet(opaque)]
pub struct Cell(Arc<RwLock<Value>>);

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::cell", |module| {
        module
            .raw_function("new", |cx, args| {
                let [value] = args else {
                    return Err(RuntimeError::new("`cell::new` takes one argument"));
                };
                let ty = cx.registry().type_of_shape(Cell::SHAPE)
                    .ok_or_else(|| RuntimeError::new("Cell type not registered"))?;
                let type_args = value.builtin_type().into_iter().collect();
                let cell = Cell(Arc::new(RwLock::new(value.clone())));
                let obj = NativeObject::with_type_args(ty, Box::new(cell), type_args);
                Ok(Value::Native(obj.into()))
            });
        module
            .ty::<Cell>("Cell")
            .raw_function("get", |_cx, args| {
                let [Value::Native(object)] = args else {
                    return Err(RuntimeError::new("`get` takes no arguments"));
                };
                with(object, |cell: &Cell| {
                    cell.0.read().unwrap_or_else(|p| p.into_inner()).clone()
                })
            })
            .raw_function("set", |_cx, args| {
                let [Value::Native(object), value] = args else {
                    return Err(RuntimeError::new("`set` takes one argument"));
                };
                if let Some(&expected) = object.type_args().first() {
                    if value.builtin_type() != Some(expected) {
                        return Err(RuntimeError::new(format!(
                            "Cell<{expected}> cannot hold a {}",
                            value.kind(),
                        )));
                    }
                }
                let cell = object
                    .with(|c: &Cell| c.clone())
                    .ok_or_else(|| RuntimeError::new("expected a `Cell`"))?;
                *cell.0.write().unwrap_or_else(|p| p.into_inner()) = value.clone();
                Ok(Value::Unit)
            });
    });
}

fn with<T: 'static, R>(object: &NativeObject, f: impl FnOnce(&T) -> R) -> RunResult<R> {
    object
        .with(f)
        .ok_or_else(|| RuntimeError::new("expected a `Cell`"))
}

