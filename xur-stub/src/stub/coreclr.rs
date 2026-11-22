//! `std::stub::coreclr` — CoreCLR runtime types for calling and hooking
//! managed methods from xur scripts.
//!
//! Provides `Runtime`, `MethodPtr`, `ManagedObject`, and `CapturedRef` types
//! for resolving, calling, and hooking .NET methods. The runtime is attached
//! by an engine-specific module (e.g. `std::stub::godot`).
//!
//! ```text
//! use std::stub::godot;
//! use std::stub::coreclr;
//!
//! let rt = godot::init(9)?;
//! let method = rt.method("sts2", "Some.Type", "get_Value")?;
//! let val = method.call(obj);
//! ```

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use facet::Facet;
use xur_runtime::convert::{into_value, ConvertCx};
use xur_runtime::registry::ModuleBuilder;
use xur_runtime::{Registry, RuntimeError, Value};

pub(crate) static RUNTIME: OnceLock<hao_runtime::Runtime> = OnceLock::new();

// ---------------------------------------------------------------------------
// Opaque wrappers
// ---------------------------------------------------------------------------

#[derive(Facet, Clone)]
#[facet(opaque)]
pub(crate) struct CoreClrRuntime;

impl std::fmt::Display for CoreClrRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Runtime")
    }
}

#[derive(Facet, Clone, Copy)]
#[facet(opaque)]
struct MethodPtr(usize);

unsafe impl Send for MethodPtr {}
unsafe impl Sync for MethodPtr {}

impl std::fmt::Display for MethodPtr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MethodPtr(0x{:x})", self.0)
    }
}

#[derive(Facet, Clone, Copy)]
#[facet(opaque)]
struct ManagedObject(usize);

unsafe impl Send for ManagedObject {}
unsafe impl Sync for ManagedObject {}

impl std::fmt::Display for ManagedObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0 == 0 {
            write!(f, "ManagedObject(null)")
        } else {
            write!(f, "ManagedObject(0x{:x})", self.0)
        }
    }
}

#[derive(Facet, Clone)]
#[facet(opaque)]
struct CapturedRef(&'static AtomicUsize);

unsafe impl Send for CapturedRef {}
unsafe impl Sync for CapturedRef {}

impl std::fmt::Display for CapturedRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let val = self.0.load(Ordering::Relaxed);
        if val == 0 {
            write!(f, "CapturedRef(null)")
        } else {
            write!(f, "CapturedRef(0x{val:x})")
        }
    }
}

// ---------------------------------------------------------------------------
// Hook dispatch infrastructure
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum HookEntry {
    Capture(&'static AtomicUsize),
    Callback {
        closure: Value,
        handle: xur_vm::SharedHandle,
    },
}

static HOOKS: OnceLock<Mutex<HashMap<usize, HookEntry>>> = OnceLock::new();

#[cfg(target_arch = "x86_64")]
static PROTECT_PATCHER: hooking::patch::ProtectPatcher = hooking::patch::ProtectPatcher::new();

#[cfg(target_arch = "x86_64")]
fn hook_writer() -> hooking::HookWriter<'static> {
    hooking::HookWriter::from_static().with_patcher(&PROTECT_PATCHER)
}

fn hooks() -> &'static Mutex<HashMap<usize, HookEntry>> {
    HOOKS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(target_arch = "x86_64")]
extern "C" fn coreclr_hook_dispatch(
    original: usize,
    int_regs: *mut u64,
    _sse_regs: *mut [u8; 16],
    result: *mut hooking::DispatchResult,
) {
    let entry = {
        let map = hooks().lock().unwrap_or_else(|e| e.into_inner());
        map.get(&original).cloned()
    };
    let Some(entry) = entry else {
        unsafe { *result = hooking::DispatchResult::passthrough(); }
        return;
    };
    let dr = match &entry {
        HookEntry::Capture(slot) => {
            let this_ptr = unsafe { *int_regs };
            slot.store(this_ptr as usize, Ordering::Relaxed);
            hooking::DispatchResult::passthrough()
        }
        HookEntry::Callback { closure, handle } => {
            let this_addr = unsafe { *int_regs } as i64;
            match handle.call_closure(closure, &[Value::Int(this_addr)]) {
                Ok(val) => interpret_hook_result(val, handle.lang(), int_regs),
                Err(e) => {
                    tracing::error!("coreclr hook callback error: {}", e.message);
                    hooking::DispatchResult::passthrough()
                }
            }
        }
    };
    unsafe { *result = dr; }
}

#[cfg(target_arch = "x86_64")]
fn interpret_hook_result(
    result: Value,
    lang: &xur_resolve::LangItems,
    _int_regs: *mut u64,
) -> hooking::DispatchResult {
    if let Value::Adt(ref adt) = result {
        if adt.ctor == lang.none {
            return hooking::DispatchResult::passthrough();
        }
        if adt.ctor == lang.some {
            let val = match adt.fields.first() {
                Some(Value::Int(v)) => *v as u64,
                _ => 0,
            };
            return hooking::DispatchResult::override_int(val);
        }
    }
    hooking::DispatchResult::passthrough()
}

// ---------------------------------------------------------------------------
// Calling helpers
// ---------------------------------------------------------------------------

fn marshal_int_args(args: &[Value]) -> Result<Vec<u64>, RuntimeError> {
    let mut out = Vec::with_capacity(args.len());
    for v in args {
        match v {
            Value::Int(i) => out.push(*i as u64),
            Value::Bool(b) => out.push(*b as u64),
            Value::Float(f) => out.push(*f as u64),
            Value::Native(n) => {
                #[cfg(feature = "xur-decimal")]
                if let Some(regs) = n.with(|d: &xur_std::decimal::XurDecimal| d.to_dotnet_regs()) {
                    out.extend_from_slice(&regs);
                    continue;
                }
                out.push(n.with(|o: &ManagedObject| o.0 as u64).unwrap_or(0));
            }
            Value::Unit => out.push(0),
            _ => return Err(RuntimeError::new("unsupported argument type for call")),
        }
    }
    Ok(out)
}

#[cfg(target_arch = "x86_64")]
use hao_runtime::jit::{call_native, call_native_float};

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::stub::coreclr", |m| {
        m.raw_function("null", |cx, args| {
            if !args.is_empty() {
                return Err(RuntimeError::new("`null` takes no arguments"));
            }
            into_value(cx, ManagedObject(0))
        });

        register_runtime(m);
        register_method_ptr(m);
        register_managed_object(m);
        register_captured_ref(m);
    });
}

fn register_runtime(m: &mut ModuleBuilder<'_>) {
    m.ty::<CoreClrRuntime>("Runtime")
        .raw_function("method", |cx, args| {
            let [_this, Value::Str(assembly), Value::Str(type_name), Value::Str(method)] = args
            else {
                return Err(RuntimeError::new(
                    "`method` takes (assembly, type, method)",
                ));
            };
            let rt = RUNTIME
                .get()
                .ok_or_else(|| RuntimeError::new("runtime not initialized"))?;
            let lang = cx.lang();
            let result = rt
                .method_pointer_from_loaded(assembly, type_name, method)
                .or_else(|_| rt.method_pointer(assembly, type_name, method));
            match result {
                Ok(ptr) => {
                    let v = into_value(cx, MethodPtr(ptr as usize))?;
                    Ok(Value::adt(lang.ok, vec![v]))
                }
                Err(e) => Ok(Value::adt(lang.err, vec![Value::str(&e.to_string())])),
            }
        })
        .raw_function("load_assembly", |cx, args| {
            let [_this, Value::Str(path)] = args else {
                return Err(RuntimeError::new("`load_assembly` takes a path"));
            };
            let rt = RUNTIME
                .get()
                .ok_or_else(|| RuntimeError::new("runtime not initialized"))?;
            let lang = cx.lang();
            match rt.load_assembly(path.as_ref()) {
                Ok(()) => Ok(Value::adt(lang.ok, vec![Value::Unit])),
                Err(e) => Ok(Value::adt(lang.err, vec![Value::str(&e.to_string())])),
            }
        });
}

fn register_method_ptr(m: &mut ModuleBuilder<'_>) {
    m.ty::<MethodPtr>("MethodPtr")
        .raw_function("call", |_cx, args| {
            if args.is_empty() {
                return Err(RuntimeError::new("`call` takes at least self"));
            }
            let ptr = extract_ptr(&args[0])?;
            let call_args = marshal_int_args(&args[1..])?;
            #[cfg(target_arch = "x86_64")]
            {
                let result = unsafe { call_native(ptr.0, &call_args) };
                Ok(Value::Int(result as i64))
            }
            #[cfg(not(target_arch = "x86_64"))]
            Err(RuntimeError::new("call only supported on x86_64"))
        })
        .raw_function("call_ptr", |cx, args| {
            if args.is_empty() {
                return Err(RuntimeError::new("`call_ptr` takes at least self"));
            }
            let ptr = extract_ptr(&args[0])?;
            let call_args = marshal_int_args(&args[1..])?;
            #[cfg(target_arch = "x86_64")]
            {
                let result = unsafe { call_native(ptr.0, &call_args) };
                into_value(cx, ManagedObject(result as usize))
            }
            #[cfg(not(target_arch = "x86_64"))]
            Err(RuntimeError::new("call_ptr only supported on x86_64"))
        })
        .raw_function("call_float", |_cx, args| {
            if args.is_empty() {
                return Err(RuntimeError::new("`call_float` takes at least self"));
            }
            let ptr = extract_ptr(&args[0])?;
            let call_args = marshal_int_args(&args[1..])?;
            #[cfg(target_arch = "x86_64")]
            {
                let result = unsafe { call_native_float(ptr.0, &call_args) };
                Ok(Value::Float(result))
            }
            #[cfg(not(target_arch = "x86_64"))]
            Err(RuntimeError::new("call_float only supported on x86_64"))
        })
        .raw_function("capture_this", |cx, args| {
            let [this] = args else {
                return Err(RuntimeError::new("`capture_this` takes no arguments"));
            };
            let ptr = extract_ptr(this)?;
            #[cfg(target_arch = "x86_64")]
            {
                let slot: &'static AtomicUsize = Box::leak(Box::new(AtomicUsize::new(0)));
                let mut hook = unsafe {
                    hook_writer()
                        .create_save_registers(
                            ptr.0 as *mut u8,
                            coreclr_hook_dispatch,
                        )
                }
                .map_err(|e| RuntimeError::new(format!("hook failed: {e:?}")))?;
                let orig = hook.original_function().as_ptr() as usize;
                hooks()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(orig, HookEntry::Capture(slot));
                unsafe { hook.apply_hook() }
                    .map_err(|e| RuntimeError::new(format!("apply hook failed: {e:?}")))?;
                std::mem::forget(hook);
                into_value(cx, CapturedRef(slot))
            }
            #[cfg(not(target_arch = "x86_64"))]
            Err(RuntimeError::new(
                "capture_this only supported on x86_64",
            ))
        })
        .raw_function("hook", |cx, args| {
            let [this, callback] = args else {
                return Err(RuntimeError::new("`hook` takes a callback closure"));
            };
            let ptr = extract_ptr(this)?;
            let callback = callback.clone();

            let handle: Box<xur_vm::SharedHandle> = cx
                .shared_handle()
                .and_then(|h| h.downcast().ok())
                .ok_or_else(|| RuntimeError::new("hook requires VM support"))?;
            let handle = *handle;

            #[cfg(target_arch = "x86_64")]
            {
                let mut hook = unsafe {
                    hook_writer()
                        .create_save_registers(
                            ptr.0 as *mut u8,
                            coreclr_hook_dispatch,
                        )
                }
                .map_err(|e| RuntimeError::new(format!("hook failed: {e:?}")))?;
                let orig = hook.original_function().as_ptr() as usize;
                hooks()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(orig, HookEntry::Callback { closure: callback, handle });
                unsafe { hook.apply_hook() }
                    .map_err(|e| RuntimeError::new(format!("apply hook failed: {e:?}")))?;
                std::mem::forget(hook);
                Ok(Value::Unit)
            }
            #[cfg(not(target_arch = "x86_64"))]
            Err(RuntimeError::new("hook only supported on x86_64"))
        })
        .method("address", |p: &MethodPtr| -> i64 { p.0 as i64 })
        .raw_function("resolve_jit", |cx, args| {
            let [this] = args else {
                return Err(RuntimeError::new("`resolve_jit` takes no arguments"));
            };
            let ptr = extract_ptr(this)?;
            #[cfg(target_arch = "x86_64")]
            {
                let resolved = unsafe { hao_runtime::jit::resolve_jit(ptr.0) };
                into_value(cx, MethodPtr(resolved))
            }
            #[cfg(not(target_arch = "x86_64"))]
            Err(RuntimeError::new("resolve_jit only supported on x86_64"))
        })
        .raw_function("patch_precode_target", |_cx, args| {
            let [this, Value::Native(target_obj)] = args else {
                return Err(RuntimeError::new(
                    "`patch_precode_target` takes (new_target: MethodPtr)",
                ));
            };
            let ptr = extract_ptr(this)?;
            let new_target = target_obj
                .with(|o: &MethodPtr| o.0)
                .ok_or_else(|| RuntimeError::new("expected a MethodPtr"))?;
            #[cfg(target_arch = "x86_64")]
            {
                unsafe { hao_runtime::jit::patch_precode_target(ptr.0, new_target) }
                    .map_err(|e| RuntimeError::new(e.to_string()))?;
                Ok(Value::Unit)
            }
            #[cfg(not(target_arch = "x86_64"))]
            Err(RuntimeError::new("patch_precode_target only supported on x86_64"))
        })
        .method("peek_bytes", |p: &MethodPtr, count: i64| -> String {
            let n = count.max(0) as usize;
            unsafe { hao_runtime::jit::peek_bytes(p.0, n) }
        })
        .method("read_bytes", |p: &MethodPtr, count: i64| -> Vec<u8> {
            let n = count.max(0) as usize;
            unsafe { std::slice::from_raw_parts(p.0 as *const u8, n).to_vec() }
        });
}

fn register_managed_object(m: &mut ModuleBuilder<'_>) {
    m.ty::<ManagedObject>("ManagedObject")
        .method("address", |o: &ManagedObject| -> i64 { o.0 as i64 })
        .method("is_null", |o: &ManagedObject| -> bool { o.0 == 0 })
        .method("read_i64", |o: &ManagedObject, offset: i64| -> i64 {
            assert!(o.0 != 0, "null pointer");
            let ptr = (o.0 as isize + offset as isize) as *const i64;
            unsafe { std::ptr::read_unaligned(ptr) }
        })
        .raw_function("deref", |cx, args| {
            let [this, Value::Int(offset)] = args else {
                return Err(RuntimeError::new("`deref` takes (offset: int)"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a ManagedObject"));
            };
            let addr = obj
                .with(|o: &ManagedObject| o.0)
                .ok_or_else(|| RuntimeError::new("expected a ManagedObject"))?;
            if addr == 0 {
                return Err(RuntimeError::new("null pointer"));
            }
            let ptr = (addr as isize + *offset as isize) as *const usize;
            let val = unsafe { std::ptr::read_unaligned(ptr) };
            into_value(cx, ManagedObject(val))
        })
        .method("read_u8", |o: &ManagedObject, offset: i64| -> i64 {
            assert!(o.0 != 0, "null pointer");
            let ptr = (o.0 as isize + offset as isize) as *const u8;
            (unsafe { std::ptr::read(ptr) }) as i64
        })
        .method("read_i32", |o: &ManagedObject, offset: i64| -> i64 {
            assert!(o.0 != 0, "null pointer");
            let ptr = (o.0 as isize + offset as isize) as *const i32;
            (unsafe { std::ptr::read_unaligned(ptr) }) as i64
        })
        .method("write_i32", |o: &ManagedObject, offset: i64, value: i64| {
            assert!(o.0 != 0, "null pointer");
            let ptr = (o.0 as isize + offset as isize) as *mut i32;
            unsafe { std::ptr::write(ptr, value as i32) };
        });
}

fn register_captured_ref(m: &mut ModuleBuilder<'_>) {
    m.ty::<CapturedRef>("CapturedRef")
        .raw_function("get", |cx, args| {
            let [this] = args else {
                return Err(RuntimeError::new("`get` takes no arguments"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a CapturedRef"));
            };
            let val = obj
                .with(|c: &CapturedRef| c.0.load(Ordering::Relaxed))
                .ok_or_else(|| RuntimeError::new("expected a CapturedRef"))?;
            into_value(cx, ManagedObject(val))
        });
}

// ---------------------------------------------------------------------------
// Extractors
// ---------------------------------------------------------------------------

fn extract_ptr(val: &Value) -> Result<MethodPtr, RuntimeError> {
    let Value::Native(obj) = val else {
        return Err(RuntimeError::new("expected a MethodPtr"));
    };
    obj.with(|p: &MethodPtr| *p)
        .ok_or_else(|| RuntimeError::new("expected a MethodPtr"))
}
