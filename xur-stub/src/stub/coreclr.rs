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
use std::sync::{Arc, Mutex as StdMutex};

use xur_runtime::convert::{from_value, into_value, ConvertCx};
use xur_runtime::registry::ModuleBuilder;
use xur_runtime::{Registry, RuntimeError, Value};
use xur_std::decimal::XurDecimal;

pub(crate) static RUNTIME: OnceLock<hao_runtime::Runtime> = OnceLock::new();

// ---------------------------------------------------------------------------
// Game-thread dispatch — managed calls must run on the CLR-registered thread
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Memory probe — validate pointers before dereferencing
// ---------------------------------------------------------------------------

#[cfg(unix)]
static PROBE_FD: OnceLock<i32> = OnceLock::new();

#[cfg(unix)]
fn init_probe() {
    let fd = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY) };
    if fd >= 0 {
        PROBE_FD.set(fd).ok();
    }
}

#[cfg(not(unix))]
fn init_probe() {}

fn probe_readable(addr: usize, len: usize) -> bool {
    if addr < 0x1000 {
        return false;
    }
    #[cfg(unix)]
    {
        if let Some(&fd) = PROBE_FD.get() {
            return unsafe { libc::write(fd, addr as *const libc::c_void, len) } == len as isize;
        }
    }
    true
}

fn check_readable(addr: usize, len: usize) -> Result<(), String> {
    if addr == 0 {
        return Err("null pointer".into());
    }
    if !probe_readable(addr, len) {
        return Err(format!("stale reference at 0x{addr:x} ({len} bytes)"));
    }
    Ok(())
}

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
pub(crate) struct ManagedObject(pub(crate) usize);

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
        arg_kinds: Vec<ArgKind>,
    },
    // Argument-rewriting hook. The callback's declared parameter types drive
    // marshalling: each argument is read from its ABI slot into a typed xur value,
    // passed to the callback, and the (possibly modified) values it returns are
    // written back before the original runs. `arg_kinds[i]` is how parameter i
    // maps to the ABI. Fully generic: any arg rewrite lives in the callback.
    RewriteArgs {
        closure: Value,
        handle: xur_vm::SharedHandle,
        arg_kinds: Vec<ArgKind>,
    },
}

/// How one hooked-method parameter maps to the x86-64 SysV ABI, picked from the
/// callback parameter's declared xur type. Integer-class args consume the next
/// integer register (`rdi, rsi, rdx, rcx, r8, r9`); float-class args the next SSE
/// register (`xmm0..`); a `Decimal` is a 16-byte value spanning two int regs.
#[derive(Clone, Copy)]
enum ArgKind {
    Int,     // int reg  <-> xur int / bool / pointer
    Float32, // SSE reg  <-> xur f32
    Float64, // SSE reg  <-> xur f64
    Managed, // int reg  <-> coreclr::ManagedObject (a managed pointer)
    Decimal, // 2 int regs <-> std::decimal (System.Decimal, by value)
}

/// Maps each declared parameter of `callback` to its ABI slot, from the xur type
/// the parameter was written as. Native types are matched by registry id (robust
/// vs the type's def-path string). Shared by `hook` and `hook_args`.
fn classify_params(handle: &xur_vm::SharedHandle, callback: &Value) -> Vec<ArgKind> {
    let managed_id = handle.registry().type_id_by_name("ManagedObject");
    let decimal_id = handle.registry().type_id_by_name("Decimal");
    handle
        .callee_param_types(callback)
        .map(|types| {
            types
                .iter()
                .map(|t| match t {
                    Some(xur_vm::RtType::Float { name: "f32" }) => ArgKind::Float32,
                    Some(xur_vm::RtType::Float { .. }) => ArgKind::Float64,
                    Some(xur_vm::RtType::Native { id, .. }) if Some(*id) == managed_id => {
                        ArgKind::Managed
                    }
                    Some(xur_vm::RtType::Native { id, .. }) if Some(*id) == decimal_id => {
                        ArgKind::Decimal
                    }
                    _ => ArgKind::Int,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Reads each hooked argument from its ABI register into a typed xur value.
#[cfg(target_arch = "x86_64")]
unsafe fn marshal_in(
    arg_kinds: &[ArgKind],
    int_regs: *mut u64,
    sse_regs: *mut [u8; 16],
    handle: &xur_vm::SharedHandle,
) -> Vec<Value> {
    let read_int = |i: usize| -> u64 { if i < 6 { unsafe { *int_regs.add(i) } } else { 0 } };
    let read_f64 =
        |i: usize| -> f64 { if i < 8 { unsafe { *(sse_regs.add(i) as *const f64) } } else { 0.0 } };
    let read_f32 =
        |i: usize| -> f32 { if i < 8 { unsafe { *(sse_regs.add(i) as *const f32) } } else { 0.0 } };
    let mut int_i = 0usize;
    let mut sse_i = 0usize;
    let mut out = Vec::with_capacity(arg_kinds.len());
    for &kind in arg_kinds {
        match kind {
            ArgKind::Int => {
                out.push(Value::int(read_int(int_i) as i64));
                int_i += 1;
            }
            ArgKind::Float32 => {
                out.push(Value::float(read_f32(sse_i) as f64));
                sse_i += 1;
            }
            ArgKind::Float64 => {
                out.push(Value::float(read_f64(sse_i)));
                sse_i += 1;
            }
            ArgKind::Managed => {
                let p = read_int(int_i) as usize;
                out.push(into_value(handle, ManagedObject(p)).unwrap_or(Value::int(p as i64)));
                int_i += 1;
            }
            ArgKind::Decimal => {
                let d = XurDecimal::from_dotnet_regs([read_int(int_i), read_int(int_i + 1)]);
                out.push(into_value(handle, d).unwrap_or(Value::Unit));
                int_i += 2;
            }
        }
    }
    out
}

/// Writes the callback's returned values back into the ABI argument registers.
/// Only arguments the callback actually CHANGED are written; an argument returned
/// unchanged (the same value it was given) is left untouched, so its register
/// keeps its exact original bits rather than being round-tripped through a xur
/// type (which can be lossy, e.g. a Decimal) and corrupting the original call.
#[cfg(target_arch = "x86_64")]
unsafe fn marshal_out(
    arg_kinds: &[ArgKind],
    inputs: &[Value],
    rets: &[Value],
    int_regs: *mut u64,
    sse_regs: *mut [u8; 16],
    handle: &xur_vm::SharedHandle,
) {
    let write_int = |i: usize, v: u64| {
        if i < 6 {
            unsafe { *int_regs.add(i) = v }
        }
    };
    let write_f64 = |i: usize, v: f64| {
        if i < 8 {
            unsafe { *(sse_regs.add(i) as *mut f64) = v }
        }
    };
    let write_f32 = |i: usize, v: f32| {
        if i < 8 {
            unsafe { *(sse_regs.add(i) as *mut f32) = v }
        }
    };
    let mut int_i = 0usize;
    let mut sse_i = 0usize;
    for (i, &kind) in arg_kinds.iter().enumerate() {
        let rv = rets.get(i);
        // Unchanged (callback returned the same value it received) -> leave the
        // register alone. Native values (ManagedObject/Decimal) compare by Arc
        // identity, so an untouched arg is recognised and preserved exactly.
        let changed = match (rv, inputs.get(i)) {
            (Some(r), Some(inp)) => r != inp,
            _ => false,
        };
        match kind {
            ArgKind::Int => {
                if changed
                    && let Some(Value::Int(n)) = rv {
                        write_int(int_i, n.to_i128() as u64);
                    }
                int_i += 1;
            }
            ArgKind::Float32 => {
                if changed
                    && let Some(Value::Float(f)) = rv {
                        write_f32(sse_i, f.as_f32());
                    }
                sse_i += 1;
            }
            ArgKind::Float64 => {
                if changed
                    && let Some(Value::Float(f)) = rv {
                        write_f64(sse_i, f.as_f64());
                    }
                sse_i += 1;
            }
            ArgKind::Managed => {
                if changed
                    && let Some(v) = rv
                        && let Ok(m) = from_value::<ManagedObject>(handle, v) {
                            write_int(int_i, m.0 as u64);
                        }
                int_i += 1;
            }
            ArgKind::Decimal => {
                if changed
                    && let Some(v) = rv
                        && let Ok(d) = from_value::<XurDecimal>(handle, v) {
                            let r = d.to_dotnet_regs();
                            write_int(int_i, r[0]);
                            write_int(int_i + 1, r[1]);
                        }
                int_i += 2;
            }
        }
    }
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
    sse_regs: *mut [u8; 16],
    result: *mut hooking::DispatchResult,
) {
    // A panic must never unwind across this extern "C" boundary into the game's
    // JIT code (that is UB and corrupts control flow). Any panic in the argument
    // marshalling or the xur VM is caught here and turned into a safe passthrough,
    // and logged so we can see what failed instead of crashing the game.
    let dr = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        hook_dispatch_inner(original, int_regs, sse_regs)
    }))
    .unwrap_or_else(|e| {
        let msg = e
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| e.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic>".to_string());
        tracing::error!("coreclr hook dispatch panicked ({msg}); passing through");
        hooking::DispatchResult::passthrough()
    });
    unsafe {
        *result = dr;
    }
}

#[cfg(target_arch = "x86_64")]
fn hook_dispatch_inner(
    original: usize,
    int_regs: *mut u64,
    sse_regs: *mut [u8; 16],
) -> hooking::DispatchResult {
    let entry = {
        let map = hooks().lock().unwrap_or_else(|e| e.into_inner());
        map.get(&original).cloned()
    };
    let Some(entry) = entry else {
        return hooking::DispatchResult::passthrough();
    };
    match &entry {
        HookEntry::Capture(slot) => {
            let this_ptr = unsafe { *int_regs };
            slot.store(this_ptr as usize, Ordering::Relaxed);
            hooking::DispatchResult::passthrough()
        }
        HookEntry::Callback {
            closure,
            handle,
            arg_kinds,
        } => {
            let args = unsafe { marshal_in(arg_kinds, int_regs, sse_regs, handle) };
            match handle.call_closure(closure, &args) {
                Ok(val) => interpret_hook_result(val, handle.lang(), int_regs),
                Err(e) => {
                    tracing::error!("coreclr hook callback error: {}", e.message);
                    hooking::DispatchResult::passthrough()
                }
            }
        }
        HookEntry::RewriteArgs {
            closure,
            handle,
            arg_kinds,
        } => {
            let call_args = unsafe { marshal_in(arg_kinds, int_regs, sse_regs, handle) };
            match handle.call_closure(closure, &call_args) {
                Ok(ret) => {
                    // The callback returns the (possibly modified) values: a tuple
                    // for several params, or a single value for one.
                    let rets: Vec<Value> = match ret {
                        Value::Tuple(items) => items.to_vec(),
                        other => vec![other],
                    };
                    unsafe { marshal_out(arg_kinds, &call_args, &rets, int_regs, sse_regs, handle) };
                }
                Err(e) => tracing::error!("coreclr hook_args callback error: {}", e.message),
            }
            hooking::DispatchResult::passthrough()
        }
    }
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
                Some(Value::Int(v)) => v.to_i128() as u64,
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
            Value::Int(i) => out.push(i.to_i128() as u64),
            Value::Bool(b) => out.push(*b as u64),
            Value::Float(f) => out.push(f.as_f64() as u64),
            Value::Native(n) => {
                #[cfg(feature = "xur-decimal")]
                if let Some(regs) = n.with(|d: &xur_std::decimal::XurDecimal| d.to_dotnet_regs()) {
                    out.extend_from_slice(&regs);
                    continue;
                }
                let addr = n.with(|o: &ManagedObject| o.0 as u64).unwrap_or(0);
                if addr != 0 {
                    check_readable(addr as usize, 8)
                        .map_err(|e| RuntimeError::new(format!("call arg: {e}")))?;
                }
                out.push(addr);
            }
            Value::Unit => out.push(0),
            _ => return Err(RuntimeError::new("unsupported argument type for call")),
        }
    }
    Ok(out)
}

#[cfg(target_arch = "x86_64")]
use hao_runtime::jit::{call_native, call_native_float};

#[cfg(target_arch = "x86_64")]
fn safe_call_native(ptr: usize, args: &[u64]) -> Result<u64, RuntimeError> {
    // TODO: route through rt.invoke() once the SuppressGCTransition calli
    // is confirmed working.  Currently the [UnmanagedCallersOnly] trampoline
    // triggers SIGABRT on unregistered threads.
    Ok(unsafe { call_native(ptr, args) })
}

#[cfg(target_arch = "x86_64")]
fn safe_call_native_float(ptr: usize, args: &[u64]) -> Result<f64, RuntimeError> {
    Ok(unsafe { call_native_float(ptr, args) })
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub(crate) fn register(registry: &Registry) {
    init_probe();
    registry.lazy_module("std::stub::coreclr", |m| {
        m.raw_function("null", |cx, args| {
            if !args.is_empty() {
                return Err(RuntimeError::new("`null` takes no arguments"));
            }
            into_value(cx, ManagedObject(0))
        });

        m.named_function("from_address", &["addr"], |addr: i64| -> ManagedObject {
            ManagedObject(addr as usize)
        });

        // Materializes any managed `IEnumerable` (lazy iterators, LINQ chains,
        // dictionary views — whose layout pointer math cannot read) into a list
        // of its elements, via the CLR's own `foreach` in Hao.Loader. Must run
        // on the cooperative (game) thread, and the runtime must have warmed
        // Enumerate off that thread first (see `warm_enumerate`).
        m.named_function(
            "enumerate",
            &["obj"],
            |obj: ManagedObject| -> Result<Vec<ManagedObject>, String> {
                if obj.0 == 0 {
                    return Ok(Vec::new());
                }
                let rt = RUNTIME
                    .get()
                    .ok_or_else(|| "runtime not initialized".to_string())?;
                let items = rt.enumerate(obj.0).map_err(|e| e.to_string())?;
                Ok(items.into_iter().map(ManagedObject).collect())
            },
        );

        // JIT-compiles and caches the Enumerate helper. Call once off the game
        // thread (at init) before any `enumerate` call on the game thread.
        m.named_function("warm_enumerate", &[], || -> Result<(), String> {
            let rt = RUNTIME
                .get()
                .ok_or_else(|| "runtime not initialized".to_string())?;
            rt.warm_enumerate().map_err(|e| e.to_string())
        });

        // Injects a dynamically-created .NET assembly from its raw bytes (e.g.
        // read from a file or built elsewhere) into the running CLR. `name` is
        // the assembly name (used for identity on Mono; cosmetic on CoreCLR).
        m.named_function(
            "load_library",
            &["name", "bytes"],
            |name: String, bytes: Vec<u8>| -> Result<(), String> {
                let rt = RUNTIME
                    .get()
                    .ok_or_else(|| "runtime not initialized".to_string())?;
                rt.load_assembly_bytes(&name, &bytes)
                    .map_err(|e| e.to_string())
            },
        );

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
        .raw_function("find_method", |cx, args| {
            let [_this, Value::Str(assembly), Value::Str(type_name), Value::Str(method), Value::Int(param_count)] =
                args
            else {
                return Err(RuntimeError::new(
                    "`find_method` takes (assembly, type, method, param_count)",
                ));
            };
            let rt = RUNTIME
                .get()
                .ok_or_else(|| RuntimeError::new("runtime not initialized"))?;
            let lang = cx.lang();
            let pc = param_count.to_i64().ok_or_else(|| RuntimeError::new("param_count out of range"))? as i32;
            match rt.find_method_from_loaded(assembly, type_name, method, pc) {
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
        })
        .raw_function("type_names", |cx, args| {
            let [_this, Value::Str(assembly)] = args else {
                return Err(RuntimeError::new("`type_names` takes (assembly)"));
            };
            let rt = RUNTIME
                .get()
                .ok_or_else(|| RuntimeError::new("runtime not initialized"))?;
            let lang = cx.lang();
            match rt.type_names(assembly) {
                Ok(names) => {
                    let list: Vec<Value> = names.into_iter().map(|n| Value::str(&n)).collect();
                    Ok(Value::adt(
                        lang.ok,
                        vec![Value::List(Arc::new(StdMutex::new(list)))],
                    ))
                }
                Err(e) => Ok(Value::adt(lang.err, vec![Value::str(&e.to_string())])),
            }
        })
        .raw_function("method_names", |cx, args| {
            let [_this, Value::Str(assembly), Value::Str(type_name)] = args else {
                return Err(RuntimeError::new(
                    "`method_names` takes (assembly, type_name)",
                ));
            };
            let rt = RUNTIME
                .get()
                .ok_or_else(|| RuntimeError::new("runtime not initialized"))?;
            let lang = cx.lang();
            match rt.method_names(assembly, type_name) {
                Ok(names) => {
                    let list: Vec<Value> = names.into_iter().map(|n| Value::str(&n)).collect();
                    Ok(Value::adt(
                        lang.ok,
                        vec![Value::List(Arc::new(StdMutex::new(list)))],
                    ))
                }
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
                let result = safe_call_native(ptr.0, &call_args)?;
                Ok(Value::int(result as i64))
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
                let result = safe_call_native(ptr.0, &call_args)?;
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
                let result = safe_call_native_float(ptr.0, &call_args)?;
                Ok(Value::float(result))
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
                    hook_writer().create_save_registers(ptr.0 as *mut u8, coreclr_hook_dispatch)
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
            Err(RuntimeError::new("capture_this only supported on x86_64"))
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
            let arg_kinds = classify_params(&handle, &callback);

            #[cfg(target_arch = "x86_64")]
            {
                let mut hook = unsafe {
                    hook_writer().create_save_registers(ptr.0 as *mut u8, coreclr_hook_dispatch)
                }
                .map_err(|e| RuntimeError::new(format!("hook failed: {e:?}")))?;
                let orig = hook.original_function().as_ptr() as usize;
                hooks().lock().unwrap_or_else(|e| e.into_inner()).insert(
                    orig,
                    HookEntry::Callback {
                        closure: callback,
                        handle,
                        arg_kinds,
                    },
                );
                unsafe { hook.apply_hook() }
                    .map_err(|e| RuntimeError::new(format!("apply hook failed: {e:?}")))?;
                std::mem::forget(hook);
                Ok(Value::Unit)
            }
            #[cfg(not(target_arch = "x86_64"))]
            Err(RuntimeError::new("hook only supported on x86_64"))
        })
        // Argument-rewriting hook. The callback receives the integer argument
        // registers as a list `[rdi, rsi, rdx, rcx, r8, r9]` and returns the
        // (possibly modified) list; the modified registers are written back before
        // the original runs. Unlike `hook` (which overrides the RETURN value),
        // this rewrites ARGUMENTS and lets the original run normally — so the
        // method still returns a valid result. (A game can use it to e.g. zero a
        // damage argument for godmode.)
        .raw_function("hook_args", |cx, args| {
            let [this, callback] = args else {
                return Err(RuntimeError::new("`hook_args` takes a callback closure"));
            };
            let ptr = extract_ptr(this)?;
            let callback = callback.clone();

            let handle: Box<xur_vm::SharedHandle> = cx
                .shared_handle()
                .and_then(|h| h.downcast().ok())
                .ok_or_else(|| RuntimeError::new("hook requires VM support"))?;
            let handle = *handle;
            let arg_kinds = classify_params(&handle, &callback);

            #[cfg(target_arch = "x86_64")]
            {
                let mut hook = unsafe {
                    hook_writer().create_save_registers(ptr.0 as *mut u8, coreclr_hook_dispatch)
                }
                .map_err(|e| RuntimeError::new(format!("hook failed: {e:?}")))?;
                let orig = hook.original_function().as_ptr() as usize;
                hooks().lock().unwrap_or_else(|e| e.into_inner()).insert(
                    orig,
                    HookEntry::RewriteArgs {
                        closure: callback,
                        handle,
                        arg_kinds,
                    },
                );
                unsafe { hook.apply_hook() }
                    .map_err(|e| RuntimeError::new(format!("apply hook failed: {e:?}")))?;
                std::mem::forget(hook);
                Ok(Value::Unit)
            }
            #[cfg(not(target_arch = "x86_64"))]
            Err(RuntimeError::new("hook_args only supported on x86_64"))
        })
        .method("address", |p: &MethodPtr| -> i64 { p.0 as i64 })
        // True when the code at this address looks like a real JIT'd method body
        // (a normal prologue), not a precode or a shared CLR fixup thunk for a
        // method that hasn't been compiled yet. Those start with a jump/call
        // (e9/eb/e8/ff) or int3 padding (cc); hooking them is unsafe because the
        // thunk is shared and fires for unrelated methods. Use this to gate hook
        // installation until a method's JIT body actually exists.
        .method("looks_jitted", |p: &MethodPtr| -> bool {
            if p.0 == 0 {
                return false;
            }
            let b = unsafe { *(p.0 as *const u8) };
            !matches!(b, 0xe9 | 0xeb | 0xe8 | 0xff | 0xcc)
        })
        .raw_function("resolve_jit", |cx, args| {
            let [this] = args else {
                return Err(RuntimeError::new("`resolve_jit` takes no arguments"));
            };
            let ptr = extract_ptr(this)?;
            #[cfg(target_arch = "x86_64")]
            {
                let resolved = unsafe { hao_runtime::jit::resolve_jit(ptr.0) };
                if resolved == 0 {
                    return Err(RuntimeError::new("resolve_jit returned null"));
                }
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
            Err(RuntimeError::new(
                "patch_precode_target only supported on x86_64",
            ))
        })
        .raw_function("call_string", |_cx, args| {
            if args.is_empty() {
                return Err(RuntimeError::new("`call_string` takes at least self"));
            }
            let ptr = extract_ptr(&args[0])?;
            let call_args = marshal_int_args(&args[1..])?;
            #[cfg(target_arch = "x86_64")]
            {
                let result = safe_call_native(ptr.0, &call_args)?;
                let s = read_clr_string(result as usize)
                    .map_err(|e| RuntimeError::new(format!("call_string: {e}")))?;
                Ok(Value::str(&s))
            }
            #[cfg(not(target_arch = "x86_64"))]
            Err(RuntimeError::new("call_string only supported on x86_64"))
        })
        .method("peek_bytes", |p: &MethodPtr, count: i64| -> Result<String, String> {
            if p.0 == 0 {
                return Err("peek_bytes on null MethodPtr".into());
            }
            let n = count.max(0) as usize;
            Ok(unsafe { hao_runtime::jit::peek_bytes(p.0, n) })
        })
        .method("read_bytes", |p: &MethodPtr, count: i64| -> Result<Vec<u8>, String> {
            if p.0 == 0 {
                return Err("read_bytes on null MethodPtr".into());
            }
            let n = count.max(0) as usize;
            Ok(unsafe { std::slice::from_raw_parts(p.0 as *const u8, n).to_vec() })
        });
}

fn register_managed_object(m: &mut ModuleBuilder<'_>) {
    m.ty::<ManagedObject>("ManagedObject")
        .method("address", |o: &ManagedObject| -> i64 { o.0 as i64 })
        .method("is_null", |o: &ManagedObject| -> bool { o.0 == 0 })
        // Null-safe accessor: `Some(obj)` when the pointer is non-null, `None`
        // otherwise. Prefer this over returning a null ManagedObject sentinel so
        // callers get a compiler-checked `Option` instead of an easy-to-forget
        // `.is_null()` check.
        .method("non_null", |o: &ManagedObject| -> Option<ManagedObject> {
            if o.0 == 0 { None } else { Some(*o) }
        })
        .method("read_i64", |o: &ManagedObject, offset: i64| -> Result<i64, String> {
            let addr = (o.0 as isize + offset as isize) as usize;
            check_readable(addr, 8)?;
            Ok(unsafe { std::ptr::read_unaligned(addr as *const i64) })
        })
        .raw_function("deref", |cx, args| {
            let [this, Value::Int(offset)] = args else {
                return Err(RuntimeError::new("`deref` takes (offset: int)"));
            };
            let Value::Native(obj) = this else {
                return Err(RuntimeError::new("expected a ManagedObject"));
            };
            let base = obj
                .with(|o: &ManagedObject| o.0)
                .ok_or_else(|| RuntimeError::new("expected a ManagedObject"))?;
            let addr = (base as isize + offset.to_i64().ok_or_else(|| RuntimeError::new(format!("{offset} is not a valid offset")))? as isize) as usize;
            check_readable(addr, 8)
                .map_err(|e| RuntimeError::new(format!("deref: {e}")))?;
            let val = unsafe { std::ptr::read_unaligned(addr as *const usize) };
            into_value(cx, ManagedObject(val))
        })
        .method("read_u8", |o: &ManagedObject, offset: i64| -> Result<i64, String> {
            let addr = (o.0 as isize + offset as isize) as usize;
            check_readable(addr, 1)?;
            Ok(unsafe { std::ptr::read(addr as *const u8) } as i64)
        })
        .method("read_i32", |o: &ManagedObject, offset: i64| -> Result<i64, String> {
            let addr = (o.0 as isize + offset as isize) as usize;
            check_readable(addr, 4)?;
            Ok(unsafe { std::ptr::read_unaligned(addr as *const i32) } as i64)
        })
        .method("write_i32", |o: &ManagedObject, offset: i64, value: i64| -> Result<(), String> {
            let addr = (o.0 as isize + offset as isize) as usize;
            check_readable(addr, 4)?;
            unsafe { std::ptr::write(addr as *mut i32, value as i32) };
            Ok(())
        })
        .method("read_string", |o: &ManagedObject| -> Result<String, String> {
            read_clr_string(o.0)
        })
        .method("read_bytes", |o: &ManagedObject, offset: i64, count: i64| -> Result<Vec<u8>, String> {
            let n = count.max(0) as usize;
            let addr = (o.0 as isize + offset as isize) as usize;
            check_readable(addr, n.max(1))?;
            Ok(unsafe { std::slice::from_raw_parts(addr as *const u8, n).to_vec() })
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

/// Reads a CoreCLR System.String object: `[MethodTable:8][length:i32][UTF-16 chars...]`
pub(crate) fn read_clr_string(addr: usize) -> Result<String, String> {
    if addr == 0 {
        return Err("read_string on null".into());
    }
    check_readable(addr + 8, 4)?;
    let len = unsafe { std::ptr::read_unaligned((addr + 8) as *const i32) };
    if len <= 0 || len > 1_000_000 {
        return Ok(String::new());
    }
    let chars_addr = addr + 12;
    let byte_len = len as usize * 2;
    check_readable(chars_addr, byte_len)?;
    let slice = unsafe { std::slice::from_raw_parts(chars_addr as *const u16, len as usize) };
    Ok(String::from_utf16_lossy(slice))
}

fn extract_ptr(val: &Value) -> Result<MethodPtr, RuntimeError> {
    let Value::Native(obj) = val else {
        return Err(RuntimeError::new("expected a MethodPtr"));
    };
    let ptr = obj
        .with(|p: &MethodPtr| *p)
        .ok_or_else(|| RuntimeError::new("expected a MethodPtr"))?;
    if ptr.0 == 0 {
        return Err(RuntimeError::new("null MethodPtr"));
    }
    Ok(ptr)
}
