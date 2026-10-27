use std::sync::{Arc, Mutex};

use xur_runtime::convert::into_value;
use xur_runtime::registry::ModuleBuilder;
use xur_runtime::{RuntimeError, Value};

use super::{DotnetEmitter, DotnetMethodBody, DotnetToken};

pub(super) fn register(dotnet: &mut ModuleBuilder<'_>) {
    let mut il = dotnet.submodule("il");
    il.raw_function("emitter", |cx, _args| {
        let e = hao::dotnet::il::Emitter::new();
        into_value(cx, DotnetEmitter(Arc::new(Mutex::new(Some(e)))))
    });

    il.ty::<DotnetEmitter>("Emitter")
        .raw_function("ldstr", |_cx, args| {
            let [this, Value::Str(s)] = args else {
                return Err(RuntimeError::new("`ldstr` takes a string"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected an Emitter"));
            };
            obj.with(|e: &DotnetEmitter| {
                e.0.lock()
                    .unwrap()
                    .as_mut()
                    .expect("emitter finished")
                    .ldstr(s.as_ref());
            })
            .ok_or_else(|| RuntimeError::new("expected an Emitter"))?;
            Ok(Value::Unit)
        })
        .raw_function("call", |_cx, args| {
            let [this, Value::Native(token)] = args else {
                return Err(RuntimeError::new("`call` takes a Token"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected an Emitter"));
            };
            let tok = token
                .with(|t: &DotnetToken| t.0.clone())
                .ok_or_else(|| RuntimeError::new("expected a Token"))?;
            obj.with(|e: &DotnetEmitter| {
                e.0.lock()
                    .unwrap()
                    .as_mut()
                    .expect("emitter finished")
                    .call(tok);
            })
            .ok_or_else(|| RuntimeError::new("expected an Emitter"))?;
            Ok(Value::Unit)
        })
        .raw_function("callvirt", |_cx, args| {
            let [this, Value::Native(token)] = args else {
                return Err(RuntimeError::new("`callvirt` takes a Token"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected an Emitter"));
            };
            let tok = token
                .with(|t: &DotnetToken| t.0.clone())
                .ok_or_else(|| RuntimeError::new("expected a Token"))?;
            obj.with(|e: &DotnetEmitter| {
                e.0.lock()
                    .unwrap()
                    .as_mut()
                    .expect("emitter finished")
                    .callvirt(tok);
            })
            .ok_or_else(|| RuntimeError::new("expected an Emitter"))?;
            Ok(Value::Unit)
        })
        .raw_function("newobj", |_cx, args| {
            let [this, Value::Native(token)] = args else {
                return Err(RuntimeError::new("`newobj` takes a Token"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected an Emitter"));
            };
            let tok = token
                .with(|t: &DotnetToken| t.0.clone())
                .ok_or_else(|| RuntimeError::new("expected a Token"))?;
            obj.with(|e: &DotnetEmitter| {
                e.0.lock()
                    .unwrap()
                    .as_mut()
                    .expect("emitter finished")
                    .newobj(tok);
            })
            .ok_or_else(|| RuntimeError::new("expected an Emitter"))?;
            Ok(Value::Unit)
        })
        .raw_function("ldc_i4", |_cx, args| {
            let [this, Value::Int(v)] = args else {
                return Err(RuntimeError::new("`ldc_i4` takes an int"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected an Emitter"));
            };
            obj.with(|e: &DotnetEmitter| {
                e.0.lock()
                    .unwrap()
                    .as_mut()
                    .expect("emitter finished")
                    .ldc_i4(*v as i32);
            })
            .ok_or_else(|| RuntimeError::new("expected an Emitter"))?;
            Ok(Value::Unit)
        })
        .raw_function("ldarg", |_cx, args| {
            let [this, Value::Int(i)] = args else {
                return Err(RuntimeError::new("`ldarg` takes an int"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected an Emitter"));
            };
            obj.with(|e: &DotnetEmitter| {
                e.0.lock()
                    .unwrap()
                    .as_mut()
                    .expect("emitter finished")
                    .ldarg(*i as u16);
            })
            .ok_or_else(|| RuntimeError::new("expected an Emitter"))?;
            Ok(Value::Unit)
        })
        .raw_function("ldloc", |_cx, args| {
            let [this, Value::Int(i)] = args else {
                return Err(RuntimeError::new("`ldloc` takes an int"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected an Emitter"));
            };
            obj.with(|e: &DotnetEmitter| {
                e.0.lock()
                    .unwrap()
                    .as_mut()
                    .expect("emitter finished")
                    .ldloc(*i as u16);
            })
            .ok_or_else(|| RuntimeError::new("expected an Emitter"))?;
            Ok(Value::Unit)
        })
        .raw_function("stloc", |_cx, args| {
            let [this, Value::Int(i)] = args else {
                return Err(RuntimeError::new("`stloc` takes an int"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected an Emitter"));
            };
            obj.with(|e: &DotnetEmitter| {
                e.0.lock()
                    .unwrap()
                    .as_mut()
                    .expect("emitter finished")
                    .stloc(*i as u16);
            })
            .ok_or_else(|| RuntimeError::new("expected an Emitter"))?;
            Ok(Value::Unit)
        })
        .raw_function("ret", |_cx, args| {
            let [this] = args else {
                return Err(RuntimeError::new("`ret` takes no arguments"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected an Emitter"));
            };
            obj.with(|e: &DotnetEmitter| {
                e.0.lock()
                    .unwrap()
                    .as_mut()
                    .expect("emitter finished")
                    .ret();
            })
            .ok_or_else(|| RuntimeError::new("expected an Emitter"))?;
            Ok(Value::Unit)
        })
        .raw_function("pop", |_cx, args| {
            let [this] = args else {
                return Err(RuntimeError::new("`pop` takes no arguments"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected an Emitter"));
            };
            obj.with(|e: &DotnetEmitter| {
                e.0.lock()
                    .unwrap()
                    .as_mut()
                    .expect("emitter finished")
                    .pop();
            })
            .ok_or_else(|| RuntimeError::new("expected an Emitter"))?;
            Ok(Value::Unit)
        })
        .raw_function("dup", |_cx, args| {
            let [this] = args else {
                return Err(RuntimeError::new("`dup` takes no arguments"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected an Emitter"));
            };
            obj.with(|e: &DotnetEmitter| {
                e.0.lock()
                    .unwrap()
                    .as_mut()
                    .expect("emitter finished")
                    .dup();
            })
            .ok_or_else(|| RuntimeError::new("expected an Emitter"))?;
            Ok(Value::Unit)
        })
        .raw_function("ldnull", |_cx, args| {
            let [this] = args else {
                return Err(RuntimeError::new("`ldnull` takes no arguments"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected an Emitter"));
            };
            obj.with(|e: &DotnetEmitter| {
                e.0.lock()
                    .unwrap()
                    .as_mut()
                    .expect("emitter finished")
                    .ldnull();
            })
            .ok_or_else(|| RuntimeError::new("expected an Emitter"))?;
            Ok(Value::Unit)
        })
        .raw_function("finish", |cx, args| {
            let [this] = args else {
                return Err(RuntimeError::new("`finish` takes no arguments"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected an Emitter"));
            };
            let body = obj
                .with(|e: &DotnetEmitter| {
                    e.0.lock()
                        .unwrap()
                        .take()
                        .ok_or_else(|| RuntimeError::new("emitter already finished"))
                        .map(|e| e.finish())
                })
                .ok_or_else(|| RuntimeError::new("expected an Emitter"))??;
            into_value(cx, DotnetMethodBody(body))
        });
}
