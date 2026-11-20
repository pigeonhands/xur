//! `std::re::proc`: live process memory, with
//! [procodile](https://docs.rs/procodile).
//!
//! `proc::processes()` lists running processes; `process.open(access)` or
//! `proc::open(pid, access)` gives a handle through which memory is read and
//! written by address, and the loaded modules and mapped regions enumerated.
//!
//! ```text
//! let handle = std::re::proc::open(pid, proc::Access::Read).unwrap();
//! for module in handle.modules().unwrap() { ... }
//! let bytes = handle.read(module.base, 16).unwrap();
//! ```
//!
//! Addresses are plain integers. `Process` and the handle are opaque OS
//! objects; everything else (modules, regions) arrives as records.

use std::sync::Arc;

use facet::{Facet, Peek};
use procodile::{Access, Address, LoadMethod, Module, Process, ProcessHandle, Protection, Region};
use xur_runtime::convert::{ConvertCx, reflect_error, to_value};
use xur_runtime::registry::ModuleBuilder;
use xur_runtime::{Record, Registry, RunResult, RuntimeError, Value};
use xur_span::Symbol;

use crate::{error_text, result};

/// A process snapshot. `Process` is an opaque OS object; it is held behind
/// an `Arc` so the wrapper is cheap to clone, and lives in Xur as a native
/// value.
#[derive(Facet, Clone)]
#[facet(opaque)]
struct Proc(Arc<Process>);

/// An open handle to a process's memory, likewise shared.
#[derive(Facet, Clone)]
#[facet(opaque)]
struct Handle(Arc<ProcessHandle>);

const ACCESS_READ: i64 = 0;
const ACCESS_READ_WRITE: i64 = 1;
const ACCESS_ALL: i64 = 2;

fn access_from_str(name: &str) -> RunResult<Access> {
    match name.to_ascii_lowercase().replace('_', "").as_str() {
        "read" => Ok(Access::Read),
        "readwrite" | "write" => Ok(Access::ReadWrite),
        "all" => Ok(Access::All),
        _ => Err(RuntimeError::new(format!(
            "unknown access `{name}`; use Read, ReadWrite, or All"
        ))),
    }
}

fn access_from_int(v: i64) -> RunResult<Access> {
    match v {
        ACCESS_READ => Ok(Access::Read),
        ACCESS_READ_WRITE => Ok(Access::ReadWrite),
        ACCESS_ALL => Ok(Access::All),
        _ => Err(RuntimeError::new(format!("unknown access value {v}"))),
    }
}

fn access_arg(cx: &mut dyn xur_runtime::Context, val: &Value) -> RunResult<Access> {
    match val {
        Value::Str(s) => access_from_str(s),
        Value::Int(v) => access_from_int(*v),
        Value::Adt(adt) => {
            let v = cx.repr_value(adt.ctor)
                .ok_or_else(|| RuntimeError::new("expected an Access enum value"))?;
            access_from_int(v)
        }
        _ => Err(RuntimeError::new("expected an Access or access string")),
    }
}

/// The names of the protection bits set in `bits`, which is how a region's
/// `protection` field arrives (procodile's `Protection` is an opaque set of
/// flag constants).
fn protection_names(bits: u8) -> Vec<String> {
    const FLAGS: [(Protection, &str); 6] = [
        (Protection::READ, "READ"),
        (Protection::WRITE, "WRITE"),
        (Protection::EXECUTE, "EXECUTE"),
        (Protection::COPY_ON_WRITE, "COPY_ON_WRITE"),
        (Protection::GUARD, "GUARD"),
        (Protection::NO_CACHE, "NO_CACHE"),
    ];
    FLAGS
        .iter()
        .filter(|(flag, _)| bits & flag.bits() != 0)
        .map(|(_, name)| (*name).to_owned())
        .collect()
}

fn wrap_proc(process: Process) -> Proc {
    Proc(Arc::new(process))
}

fn wrap_handle(handle: ProcessHandle) -> Handle {
    Handle(Arc::new(handle))
}

fn addr(value: u64) -> Address {
    Address::new(value as usize)
}

pub(super) fn register(registry: &Registry, re: &mut ModuleBuilder<'_>) {
    let mut proc = re.submodule("proc");
    register_in(registry, &mut proc);
    // procodile's remaining types become records in `std::re::proc`. Region,
    // Module and Address are converted by hand (opaque, or a bare address).
    let skip = [Region::SHAPE, Module::SHAPE, Address::SHAPE];
    super::walk::register(
        registry,
        &mut proc,
        procodile::reflect::roots(),
        submodule_of,
        &skip,
    );
}

/// Every procodile type goes directly in `std::re::proc`.
fn submodule_of(shape: &facet::Shape) -> Option<&'static str> {
    shape.module_path?.starts_with("procodile").then_some("")
}

fn register_in(registry: &Registry, proc: &mut ModuleBuilder<'_>) {
    // A region borrows nothing, but facet marks it opaque (its `path` is a
    // `PathBuf`), so its fields are turned into a record by hand.
    registry.converter(Region::SHAPE, region_record);
    registry.converter(Module::SHAPE, module_record);
    registry.converter(Address::SHAPE, |_, peek| {
        let address: &Address = peek.get().map_err(reflect_error)?;
        Ok(Value::Int(address.get() as i64))
    });

    proc.repr_enum("Access", &[
        ("Read", ACCESS_READ),
        ("ReadWrite", ACCESS_READ_WRITE),
        ("All", ACCESS_ALL),
    ]);

    proc.function("processes", || {
        procodile::process::processes()
            .map(|ps| ps.map(wrap_proc).collect::<Vec<_>>())
            .map_err(error_text)
    })
    .function("by_pid", |pid: u32| {
        Process::by_pid(pid).map(wrap_proc).map_err(error_text)
    })
    .function("by_name", |name: String| {
        Process::by_name(&name)
            .map(|ps| ps.map(wrap_proc).collect::<Vec<_>>())
            .map_err(error_text)
    })
    .function("current_pid", Process::current_pid)
    .raw_function_returns::<Handle>("open", |cx, args| open(cx, args))
    .function("open_current", || {
        ProcessHandle::open_current()
            .map(wrap_handle)
            .map_err(error_text)
    })
    .function("protection_names", protection_names);

    proc.ty::<Proc>("Process")
        .method("pid", |p: &Proc| p.0.pid())
        .method("parent_pid", |p: &Proc| p.0.parent_pid())
        .method("name", |p: &Proc| p.0.name().to_string_lossy().into_owned())
        .method("exe_path", |p: &Proc| {
            p.0.exe_path()
                .map(|path| path.display().to_string())
                .map_err(error_text)
        })
        .raw_function_returns::<Handle>("open", |cx, args| open_process(cx, args));

    proc.ty::<Handle>("Handle")
        .method("pid", |h: &Handle| h.0.pid())
        .method("is_alive", |h: &Handle| h.0.is_alive())
        .method("suspend", |h: &Handle| h.0.suspend().map_err(error_text))
        .method("resume", |h: &Handle| h.0.resume().map_err(error_text))
        .method("pointer_width", |h: &Handle| {
            h.0.pointer_width().map_err(error_text)
        })
        .method("read", |h: &Handle, address: u64, len: usize| {
            h.0.read_vec(addr(address), len).map_err(error_text)
        })
        .method("write", |h: &Handle, address: u64, bytes: Vec<u8>| {
            h.0.write_bytes(addr(address), &bytes).map_err(error_text)
        })
        .method("read_pointer", |h: &Handle, address: u64, width: usize| {
            h.0.read_pointer(addr(address), width)
                .map(|a| a.get() as u64)
                .map_err(error_text)
        })
        .method(
            "resolve_pointer_chain",
            |h: &Handle, base: u64, offsets: Vec<i64>| {
                let offsets: Vec<isize> = offsets.into_iter().map(|o| o as isize).collect();
                h.0.resolve_pointer_chain(addr(base), &offsets)
                    .map(|a| a.get() as u64)
                    .map_err(error_text)
            },
        )
        .method("regions", |h: &Handle| h.0.regions().map_err(error_text))
        .method("region_at", |h: &Handle, address: u64| {
            h.0.region_at(addr(address)).map_err(error_text)
        })
        .method("modules", |h: &Handle| h.0.modules().map_err(error_text))
        .method("module", |h: &Handle, name: String| {
            h.0.module(&name).map_err(error_text)
        })
        .method("module_at", |h: &Handle, address: u64| {
            h.0.module_at(addr(address)).map_err(error_text)
        })
        // `method` is a `LoadMethod` value: `LoadMethod::Loader` (the OS
        // loader) or `LoadMethod::ManualMap` (mapped directly, without the
        // OS loader's involvement or module-list entry).
        .method(
            "load_library",
            |h: &Handle, path: String, method: LoadMethod| {
                h.0.load_library(path, method).map_err(error_text)
            },
        )
        .method("map_image", |h: &Handle, bytes: Vec<u8>| {
            h.0.map_image(&bytes).map_err(error_text)
        })
        .method("inject_bytes", |h: &Handle, bytes: Vec<u8>| {
            h.0.inject_bytes(&bytes)
                .map(|a| a.get() as u64)
                .map_err(error_text)
        })
        .method("execute_remote", |h: &Handle, address: u64, arg: u64| {
            h.0.execute_remote(addr(address), arg).map_err(error_text)
        })
        .method(
            "execute_remote_detached",
            |h: &Handle, address: u64, arg: u64| {
                h.0.execute_remote_detached(addr(address), arg)
                    .map_err(error_text)
            },
        );
}

/// `proc::open(pid, access)`: a handle, as a Xur `Result`.
fn open(cx: &mut dyn xur_runtime::Context, args: &[Value]) -> RunResult<Value> {
    let (pid, access_val) = match args {
        [Value::Int(pid), val] => (*pid, val),
        _ => return Err(RuntimeError::new("`open` takes (pid, access)")),
    };
    let pid = u32::try_from(pid).map_err(|_| RuntimeError::new("invalid pid"))?;
    let access = access_arg(cx, access_val)?;
    handle_result(cx, ProcessHandle::open(pid, access))
}

/// `process.open(access)`: a handle to this process.
fn open_process(cx: &mut dyn xur_runtime::Context, args: &[Value]) -> RunResult<Value> {
    let (this, access_val) = match args {
        [this, val] => (this, val),
        _ => return Err(RuntimeError::new("`open` takes an access")),
    };
    let access = access_arg(cx, access_val)?;
    let Value::Native(object) = this else {
        return Err(RuntimeError::new("expected a `Process`"));
    };
    let handle = object
        .with(|p: &Proc| p.0.open(access))
        .ok_or_else(|| RuntimeError::new("expected a `Process`"))?;
    handle_result(cx, handle)
}

/// Converts an opened-handle result to a Xur `Result<Handle, str>`.
fn handle_result(
    cx: &dyn xur_runtime::Context,
    handle: procodile::Result<ProcessHandle>,
) -> RunResult<Value> {
    match handle {
        Ok(h) => Ok(result(cx, Ok(to_value(cx, &wrap_handle(h))?))),
        Err(error) => Ok(result(cx, Err(error_text(error)))),
    }
}

/// The fields of an opaque [`Region`], as a record.
fn region_record(cx: &dyn ConvertCx, peek: Peek<'_, 'static>) -> RunResult<Value> {
    let region: &Region = peek.get().map_err(reflect_error)?;
    let path = match &region.path {
        Some(path) => Value::str(&path.display().to_string()),
        None => Value::Unit,
    };
    let fields = [
        ("base", Value::Ptr(region.base.get() as u64)),
        ("size", Value::Int(region.size as i64)),
        ("protection", Value::Int(region.protection.bits().into())),
        ("kind", to_value(cx, &region.kind)?),
        ("path", path),
    ];
    let record = Record {
        ty: cx.registry().record_type_of_shape(peek.shape()),
        variant: None,
        fields: fields
            .into_iter()
            .map(|(name, value)| (Symbol::intern(name), value))
            .collect(),
    };
    Ok(Value::Record(record.into()))
}

/// The fields of an opaque [`Module`], as a record.
fn module_record(cx: &dyn ConvertCx, peek: Peek<'_, 'static>) -> RunResult<Value> {
    let module: &Module = peek.get().map_err(reflect_error)?;
    let fields = [
        ("name", Value::str(&module.name.to_string_lossy())),
        ("path", Value::str(&module.path.display().to_string())),
        ("base", Value::Ptr(module.base.get() as u64)),
        ("size", Value::Int(module.size as i64)),
    ];
    let record = Record {
        ty: cx.registry().record_type_of_shape(peek.shape()),
        variant: None,
        fields: fields
            .into_iter()
            .map(|(name, value)| (Symbol::intern(name), value))
            .collect(),
    };
    Ok(Value::Record(record.into()))
}
