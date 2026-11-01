//! `std::stub::godot` — Godot/CoreCLR runtime for LD_PRELOAD game mods.
//!
//! Attaches to a Godot game's CoreCLR (.NET) runtime and provides method
//! resolution, managed method calling, and hooking from xur scripts.
//!
//! ```text
//! use std::stub::godot;
//!
//! let rt = godot::init(9)?;
//! let get_pid = rt.method("System.Private.CoreLib", "System.Environment", "get_ProcessId")?;
//! let pid = get_pid.call();
//! ```

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use facet::Facet;
use xur_runtime::convert::{into_value, ConvertCx};
use xur_runtime::registry::ModuleBuilder;
use xur_runtime::{Registry, RuntimeError, Value};

static RUNTIME: OnceLock<hao_runtime::Runtime> = OnceLock::new();

// ---------------------------------------------------------------------------
// Opaque wrappers
// ---------------------------------------------------------------------------

#[derive(Facet, Clone)]
#[facet(opaque)]
struct GodotRuntime;

impl std::fmt::Display for GodotRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GodotRuntime")
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

/// A captured `this` pointer — set by a passthrough hook on an instance
/// method, read from the GUI thread.
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

/// Per-hook state, stored in a global map keyed by original function address.
#[derive(Clone)]
enum HookEntry {
    Capture(&'static AtomicUsize),
    Callback {
        closure: Value,
        handle: xur_vm::SharedHandle,
    },
}

static HOOKS: OnceLock<Mutex<HashMap<usize, HookEntry>>> = OnceLock::new();

/// Use ProtectPatcher instead of the default AliasPatcher. CoreCLR's JIT
/// code pages should not be replaced with memfd-backed copies, as the
/// runtime may still need to modify other code on the same page.
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
extern "C" fn godot_hook_dispatch(
    original: usize,
    int_regs: *mut u64,
    _sse_regs: *mut [u8; 16],
    result: *mut hooking::DispatchResult,
) {
    // Clone the entry and release the mutex before calling the callback.
    // This prevents deadlocks if the callback re-enters a hooked function.
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
                    tracing::error!("godot hook callback error: {}", e.message);
                    hooking::DispatchResult::passthrough()
                }
            }
        }
    };
    unsafe { *result = dr; }
}

/// Interprets a hook callback's return value:
///   None  → passthrough (original runs unchanged)
///   Some(_) → zero non-this int arg registers, then passthrough
///             (original runs with zeroed args, e.g. Decimal amount = 0)
///   Unit  → passthrough
#[cfg(target_arch = "x86_64")]
fn interpret_hook_result(
    result: Value,
    lang: &xur_resolve::LangItems,
    int_regs: *mut u64,
) -> hooking::DispatchResult {
    if let Value::Adt(ref adt) = result {
        if adt.ctor == lang.none {
            return hooking::DispatchResult::passthrough();
        }
        if adt.ctor == lang.some {
            unsafe {
                *int_regs.add(1) = 0; // rsi
                *int_regs.add(2) = 0; // rdx
            }
            return hooking::DispatchResult::passthrough();
        }
    }
    hooking::DispatchResult::passthrough()
}


// ---------------------------------------------------------------------------
// Calling helpers
// ---------------------------------------------------------------------------

fn marshal_int_args(args: &[Value]) -> Result<Vec<u64>, RuntimeError> {
    args.iter()
        .map(|v| match v {
            Value::Int(i) => Ok(*i as u64),
            Value::Bool(b) => Ok(*b as u64),
            Value::Float(f) => Ok(*f as u64),
            Value::Native(n) => Ok(n
                .with(|o: &ManagedObject| o.0 as u64)
                .unwrap_or(0)),
            Value::Unit => Ok(0),
            _ => Err(RuntimeError::new("unsupported argument type for call")),
        })
        .collect()
}

#[cfg(target_arch = "x86_64")]
unsafe fn call_native(ptr: usize, args: &[u64]) -> u64 {
    type F0 = unsafe extern "C" fn() -> u64;
    type F1 = unsafe extern "C" fn(u64) -> u64;
    type F2 = unsafe extern "C" fn(u64, u64) -> u64;
    type F3 = unsafe extern "C" fn(u64, u64, u64) -> u64;
    type F4 = unsafe extern "C" fn(u64, u64, u64, u64) -> u64;
    type F5 = unsafe extern "C" fn(u64, u64, u64, u64, u64) -> u64;
    type F6 = unsafe extern "C" fn(u64, u64, u64, u64, u64, u64) -> u64;

    unsafe {
        match args.len() {
            0 => std::mem::transmute::<usize, F0>(ptr)(),
            1 => std::mem::transmute::<usize, F1>(ptr)(args[0]),
            2 => std::mem::transmute::<usize, F2>(ptr)(args[0], args[1]),
            3 => std::mem::transmute::<usize, F3>(ptr)(args[0], args[1], args[2]),
            4 => std::mem::transmute::<usize, F4>(ptr)(args[0], args[1], args[2], args[3]),
            5 => std::mem::transmute::<usize, F5>(ptr)(args[0], args[1], args[2], args[3], args[4]),
            6 => std::mem::transmute::<usize, F6>(ptr)(
                args[0], args[1], args[2], args[3], args[4], args[5],
            ),
            _ => panic!("too many arguments for call (max 6)"),
        }
    }
}

#[cfg(target_arch = "x86_64")]
unsafe fn call_native_float(ptr: usize, args: &[u64]) -> f64 {
    type F0 = unsafe extern "C" fn() -> f64;
    type F1 = unsafe extern "C" fn(u64) -> f64;
    type F2 = unsafe extern "C" fn(u64, u64) -> f64;
    type F3 = unsafe extern "C" fn(u64, u64, u64) -> f64;

    unsafe {
        match args.len() {
            0 => std::mem::transmute::<usize, F0>(ptr)(),
            1 => std::mem::transmute::<usize, F1>(ptr)(args[0]),
            2 => std::mem::transmute::<usize, F2>(ptr)(args[0], args[1]),
            3 => std::mem::transmute::<usize, F3>(ptr)(args[0], args[1], args[2]),
            _ => panic!("too many arguments for call_float (max 3)"),
        }
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::stub::godot", |m| {
        // --- init(major) ---
        m.raw_function("init", |cx, args| {
            let [Value::Int(major)] = args else {
                return Err(RuntimeError::new("`init` takes (major: int)"));
            };

            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            let major = *major as u16;

            hao_godot::on_clr_init(move |host| {
                // Sleep briefly so the runtime finishes bootstrap before we
                // touch managed code.
                std::thread::sleep(std::time::Duration::from_secs(5));
                let result = host.attach(major);
                tx.send(result).ok();
            });

            let rt = rx
                .recv()
                .map_err(|_| RuntimeError::new("CLR init callback was dropped"))?
                .map_err(|e| RuntimeError::new(format!("CLR attach failed: {e}")))?;

            RUNTIME.set(rt).ok();

            let lang = cx.lang();
            let v = into_value(cx, GodotRuntime)?;
            Ok(Value::adt(lang.ok, vec![v]))
        });

        // --- null() ---
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
    m.ty::<GodotRuntime>("Runtime")
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
                            godot_hook_dispatch,
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
                            godot_hook_dispatch,
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
            let addr = ptr.0 as *const u8;
            let resolved = unsafe {
                let b0 = *addr;
                let b1 = *addr.add(1);
                if b0 == 0xff && b1 == 0x25 {
                    let disp = std::ptr::read_unaligned(addr.add(2) as *const i32);
                    let target_ptr = addr.add(6).offset(disp as isize) as *const usize;
                    *target_ptr
                } else if b0 == 0xe9 {
                    let disp = std::ptr::read_unaligned(addr.add(1) as *const i32);
                    addr.add(5).offset(disp as isize) as usize
                } else {
                    ptr.0
                }
            };
            into_value(cx, MethodPtr(resolved))
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
            let addr = ptr.0 as *const u8;
            unsafe {
                let b0 = *addr;
                let b1 = *addr.add(1);
                if b0 == 0xff && b1 == 0x25 {
                    let disp = std::ptr::read_unaligned(addr.add(2) as *const i32);
                    let slot = addr.add(6).offset(disp as isize) as *mut usize;
                    *slot = new_target;
                    Ok(Value::Unit)
                } else {
                    Err(RuntimeError::new("not a precode (expected ff 25)"))
                }
            }
        })
        .raw_function("peek_bytes", |_cx, args| {
            let [this, Value::Int(count)] = args else {
                return Err(RuntimeError::new("`peek_bytes` takes (count: int)"));
            };
            let ptr = extract_ptr(this)?;
            let n = (*count).min(64).max(0) as usize;
            let bytes: Vec<u8> =
                (0..n).map(|i| unsafe { *((ptr.0 + i) as *const u8) }).collect();
            let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
            Ok(Value::str(&hex.join(" ")))
        });
}

fn register_managed_object(m: &mut ModuleBuilder<'_>) {
    m.ty::<ManagedObject>("ManagedObject")
        .method("address", |o: &ManagedObject| -> i64 { o.0 as i64 })
        .method("is_null", |o: &ManagedObject| -> bool { o.0 == 0 })
        .raw_function("read_i64", |_cx, args| {
            let [this, Value::Int(offset)] = args else {
                return Err(RuntimeError::new("`read_i64` takes (offset: int)"));
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
            let ptr = (addr as isize + *offset as isize) as *const i64;
            let val = unsafe { std::ptr::read_unaligned(ptr) };
            Ok(Value::Int(val))
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
        .raw_function("read_u8", |_cx, args| {
            let [this, Value::Int(offset)] = args else {
                return Err(RuntimeError::new("`read_u8` takes (offset: int)"));
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
            let ptr = (addr as isize + *offset as isize) as *const u8;
            let val = unsafe { std::ptr::read(ptr) };
            Ok(Value::Int(val as i64))
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
