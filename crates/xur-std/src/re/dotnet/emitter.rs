use std::sync::{Arc, Mutex};

use xur_runtime::convert::into_value;
use xur_runtime::registry::{ModuleBuilder, XurType};
use xur_runtime::{RuntimeError, Value};

use super::{DotnetEmitter, DotnetMethodBody, DotnetToken};

macro_rules! emit {
    ($e:expr) => {
        $e.0.lock().unwrap().as_mut().expect("emitter finished")
    };
}

pub(super) fn register(dotnet: &mut ModuleBuilder<'_>) {
    let mut il = dotnet.submodule("il");
    il.function("emitter", || {
        DotnetEmitter(Arc::new(Mutex::new(Some(hao::dotnet::il::Emitter::new()))))
    });

    il.ty::<DotnetEmitter>("Emitter")
        .method("ldstr", |e: &DotnetEmitter, s: String| { emit!(e).ldstr(&s); })
        .raw_function_returns_xur("call", XurType::UNIT, |_cx, args| {
            let [this, Value::Native(token)] = args else {
                return Err(RuntimeError::new("`call` takes a Token"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected an Emitter"));
            };
            let tok = token
                .with(|t: &DotnetToken| t.0.clone())
                .ok_or_else(|| RuntimeError::new("expected a Token"))?;
            obj.with(|e: &DotnetEmitter| { emit!(e).call(tok); })
                .ok_or_else(|| RuntimeError::new("expected an Emitter"))?;
            Ok(Value::Unit)
        })
        .raw_function_returns_xur("callvirt", XurType::UNIT, |_cx, args| {
            let [this, Value::Native(token)] = args else {
                return Err(RuntimeError::new("`callvirt` takes a Token"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected an Emitter"));
            };
            let tok = token
                .with(|t: &DotnetToken| t.0.clone())
                .ok_or_else(|| RuntimeError::new("expected a Token"))?;
            obj.with(|e: &DotnetEmitter| { emit!(e).callvirt(tok); })
                .ok_or_else(|| RuntimeError::new("expected an Emitter"))?;
            Ok(Value::Unit)
        })
        .raw_function_returns_xur("newobj", XurType::UNIT, |_cx, args| {
            let [this, Value::Native(token)] = args else {
                return Err(RuntimeError::new("`newobj` takes a Token"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected an Emitter"));
            };
            let tok = token
                .with(|t: &DotnetToken| t.0.clone())
                .ok_or_else(|| RuntimeError::new("expected a Token"))?;
            obj.with(|e: &DotnetEmitter| { emit!(e).newobj(tok); })
                .ok_or_else(|| RuntimeError::new("expected an Emitter"))?;
            Ok(Value::Unit)
        })
        .method("ldc_i4", |e: &DotnetEmitter, v: i64| { emit!(e).ldc_i4(v as i32); })
        .method("ldarg", |e: &DotnetEmitter, i: i64| { emit!(e).ldarg(i as u16); })
        .method("ldloc", |e: &DotnetEmitter, i: i64| { emit!(e).ldloc(i as u16); })
        .method("stloc", |e: &DotnetEmitter, i: i64| { emit!(e).stloc(i as u16); })
        .method("ret", |e: &DotnetEmitter| { emit!(e).ret(); })
        .method("pop", |e: &DotnetEmitter| { emit!(e).pop(); })
        .method("dup", |e: &DotnetEmitter| { emit!(e).dup(); })
        .method("ldnull", |e: &DotnetEmitter| { emit!(e).ldnull(); })
        .raw_function_returns::<DotnetMethodBody>("finish", |cx, args| {
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
