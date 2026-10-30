//! `stub::hooking` — inline function hooking via [hooking-rs](https://docs.rs/hooking).
//!
//! ```text
//! use std::stub::hooking;
//!
//! let hook = hooking::create(target_addr, detour_addr);
//! hook.apply();
//! let original = hook.original();
//! hook.remove();
//! ```

use std::ffi::CString;
use std::sync::Mutex;

use facet::Facet;
use xur_runtime::registry::ModuleBuilder;
use xur_runtime::{Registry, RuntimeError, Value};

#[derive(Facet)]
#[facet(opaque)]
struct XurHook(Mutex<hooking::Hook<'static>>);

unsafe impl Send for XurHook {}
unsafe impl Sync for XurHook {}

impl std::fmt::Display for XurHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Hook")
    }
}

impl Clone for XurHook {
    fn clone(&self) -> Self {
        unreachable!("hooks are not cloneable")
    }
}

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::stub::hooking", |m| {
    m.raw_function("create", |_cx, args| {
        let [Value::Int(target), Value::Int(dest)] = args else {
            return Err(RuntimeError::new(
                "`create` takes (target: int, destination: int)",
            ));
        };
        let hook = unsafe {
            hooking::Hook::create(*target as *mut u8, *dest as *mut u8)
        }
        .map_err(|e| RuntimeError::new(format!("{e}")))?;
        let wrapped = XurHook(Mutex::new(hook));
        xur_runtime::convert::into_value(_cx, wrapped)
    });

    m.raw_function("by_name", |_cx, args| {
        let (module, symbol, dest) = match args {
            [Value::Str(symbol), Value::Int(dest)] => (None, symbol.clone(), *dest),
            [Value::Str(module), Value::Str(symbol), Value::Int(dest)] => {
                (Some(module.clone()), symbol.clone(), *dest)
            }
            _ => {
                return Err(RuntimeError::new(
                    "`by_name` takes (symbol, dest) or (module, symbol, dest)",
                ))
            }
        };
        let module_cstr = module
            .as_deref()
            .map(|s| CString::new(s).map_err(|e| RuntimeError::new(format!("{e}"))))
            .transpose()?;
        let symbol_cstr =
            CString::new(symbol.as_ref()).map_err(|e| RuntimeError::new(format!("{e}")))?;
        let hook = unsafe {
            hooking::Hook::by_name(
                module_cstr.as_deref(),
                &symbol_cstr,
                dest as *mut u8,
            )
        }
        .map_err(|e| RuntimeError::new(format!("{e}")))?;
        let wrapped = XurHook(Mutex::new(hook));
        xur_runtime::convert::into_value(_cx, wrapped)
    });

    let mut ty = m.ty::<XurHook>("Hook");

    ty.raw_function("apply", |_cx, args| {
        let [this] = args else {
            return Err(RuntimeError::new("`apply` takes no arguments"));
        };
        with_hook(this, |hook| {
            unsafe { hook.apply_hook() }.map_err(|e| RuntimeError::new(format!("{e}")))?;
            Ok(Value::Unit)
        })
    });

    ty.raw_function("remove", |_cx, args| {
        let [this] = args else {
            return Err(RuntimeError::new("`remove` takes no arguments"));
        };
        with_hook(this, |hook| {
            unsafe { hook.remove_hook() }.map_err(|e| RuntimeError::new(format!("{e}")))?;
            Ok(Value::Unit)
        })
    });

    ty.raw_function("original", |_cx, args| {
        let [this] = args else {
            return Err(RuntimeError::new("`original` takes no arguments"));
        };
        with_hook(this, |hook| {
            Ok(Value::Int(hook.original_function().as_ptr() as i64))
        })
    });

    ty.raw_function("target", |_cx, args| {
        let [this] = args else {
            return Err(RuntimeError::new("`target` takes no arguments"));
        };
        with_hook(this, |hook| {
            Ok(Value::Int(hook.data.symbol_address.as_ptr() as i64))
        })
    });

    });
}

fn with_hook(
    val: &Value,
    f: impl FnOnce(&mut hooking::Hook<'static>) -> Result<Value, RuntimeError>,
) -> Result<Value, RuntimeError> {
    let Value::Native(obj) = val else {
        return Err(RuntimeError::new("expected a `Hook`"));
    };
    obj.with(|wrapper: &XurHook| {
        let mut guard = wrapper
            .0
            .lock()
            .map_err(|_| RuntimeError::new("hook lock poisoned"))?;
        f(&mut guard)
    })
    .ok_or_else(|| RuntimeError::new("expected a `Hook`"))?
}
