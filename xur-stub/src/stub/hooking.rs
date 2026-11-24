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
use xur_runtime::{Registry, RuntimeError, Value};

#[derive(Facet, Clone)]
#[facet(opaque)]
pub(crate) struct XurSkip;

impl std::fmt::Display for XurSkip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Skip")
    }
}

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

#[derive(Facet)]
#[repr(C)]
#[allow(dead_code)]
enum HookError {
    LockPoisoned,
    CreateFailed(String),
    ApplyFailed(String),
    RemoveFailed(String),
}

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::stub::hooking", |m| {
        m.ty::<XurSkip>("Skip");
        m.shape_type("HookError", HookError::SHAPE);

        m.function("Skip", || -> XurSkip { XurSkip });

        m.named_function(
            "create",
            &["target", "dest"],
            |target: i64, dest: i64| -> Result<XurHook, HookError> {
                let hook =
                    unsafe { hooking::Hook::create(target as *mut u8, dest as *mut u8) }
                        .map_err(|e| HookError::CreateFailed(format!("{e}")))?;
                Ok(XurHook(Mutex::new(hook)))
            },
        );

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
                hooking::Hook::by_name(module_cstr.as_deref(), &symbol_cstr, dest as *mut u8)
            }
            .map_err(|e| RuntimeError::new(format!("{e}")))?;
            let wrapped = XurHook(Mutex::new(hook));
            xur_runtime::convert::into_value(_cx, wrapped)
        });

        let mut ty = m.ty::<XurHook>("Hook");

        ty.method("apply", |h: &XurHook| -> Result<(), HookError> {
            let mut guard = h.0.lock().map_err(|_| HookError::LockPoisoned)?;
            unsafe { guard.apply_hook() }.map_err(|e| HookError::ApplyFailed(format!("{e}")))
        });

        ty.method("remove", |h: &XurHook| -> Result<(), HookError> {
            let mut guard = h.0.lock().map_err(|_| HookError::LockPoisoned)?;
            unsafe { guard.remove_hook() }.map_err(|e| HookError::RemoveFailed(format!("{e}")))
        });

        ty.method("original", |h: &XurHook| -> Result<i64, HookError> {
            let guard = h.0.lock().map_err(|_| HookError::LockPoisoned)?;
            Ok(guard.original_function().as_ptr() as i64)
        });

        ty.method("target", |h: &XurHook| -> Result<i64, HookError> {
            let guard = h.0.lock().map_err(|_| HookError::LockPoisoned)?;
            Ok(guard.data.symbol_address.as_ptr() as i64)
        });
    });
}
