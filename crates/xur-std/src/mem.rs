//! `std::mem`: sizes, alignment and byte-level conversion of structs.

use std::sync::Arc;

use xur_runtime::{BuiltinType, Context, Registry, RunResult, RuntimeError, Value, lock};
use xur_runtime::registry::XurType;

use crate::reflect::layout::{layout_of_def, type_arg_layout};

pub(crate) fn register(registry: &Registry) {
    registry
        .module("std::mem")
        .repr_enum("Protection", &[
            ("Read", 1),
            ("Write", 2),
            ("Execute", 4),
        ])
        .named_raw_function_returns::<i64>("size_of", &[], |cx, args| {
            let layout = type_arg_layout(cx, args, "size_of")?;
            Ok(Value::Int(layout.byte_size() as i64))
        })
        .named_raw_function_returns::<i64>("align_of", &[], |cx, args| {
            let layout = type_arg_layout(cx, args, "align_of")?;
            Ok(Value::Int(layout.alignment() as i64))
        })
        .named_raw_function("from_bytes", &["data"], |cx, args| {
            let [Value::Bytes(data)] = args else {
                return Err(RuntimeError::new(
                    "`from_bytes` takes bytes: `from_bytes::<T>(data)`",
                ));
            };
            let layout = type_arg_layout(cx, &[], "from_bytes")?;
            layout.read(&lock(data), 0)
        })
        .named_raw_function_returns::<i64>("size_of_val", &["value"], |cx, args| {
            let [Value::Adt(adt)] = args else {
                return Err(RuntimeError::new("`size_of_val` takes a struct"));
            };
            Ok(Value::Int(layout_of_def(cx, adt.ctor, 0)?.byte_size() as i64))
        })
        .named_raw_function_returns::<Vec<u8>>("to_bytes", &["value"], struct_to_bytes)
        .named_raw_function_returns_xur("alloc_page", &["size"], XurType::ptr::<u8>(), alloc_page)
        .named_raw_function_returns_xur("protect", &["addr", "size", "mode"], XurType::UNIT, protect_page)
        .named_raw_function_returns_xur("write_bytes", &["addr", "data"], XurType::UNIT, write_bytes)
        .named_raw_function_returns_xur("free_page", &["addr", "size"], XurType::UNIT, free_page);
    registry
        .builtin(BuiltinType::Struct)
        .raw_function_returns::<Vec<u8>>("to_bytes", struct_to_bytes)
        .raw_function_returns::<i64>("byte_size", |cx, args| {
            let [Value::Adt(adt)] = args else {
                return Err(RuntimeError::new("`byte_size` takes no arguments"));
            };
            Ok(Value::Int(layout_of_def(cx, adt.ctor, 0)?.byte_size() as i64))
        });
}

/// `mem::alloc_page(size)` → `*u8`: allocates `size` bytes of read-write memory.
fn alloc_page(_cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [Value::Int(size)] = args else {
        return Err(RuntimeError::new("`alloc_page` takes a size in bytes"));
    };
    let size = *size as usize;
    let addr = platform::alloc_rw(size).map_err(RuntimeError::new)?;
    Ok(Value::Ptr(addr as u64))
}

fn protect_page(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let (addr, size, prot) = match args {
        [Value::Ptr(a), Value::Int(s), Value::Str(mode)] => {
            (*a, *s, mode_to_prot(mode)?)
        }
        [Value::Ptr(a), Value::Int(s), Value::Int(flags)] => {
            (*a, *s, *flags as i32)
        }
        [Value::Ptr(a), Value::Int(s), Value::Adt(adt)] => {
            let v = cx.repr_value(adt.ctor)
                .ok_or_else(|| RuntimeError::new("expected a Protection enum value"))?;
            (*a, *s, v as i32)
        }
        _ => {
            return Err(RuntimeError::new(
                "`protect` takes (ptr, size, mode) where mode is a Protection flag or string",
            ));
        }
    };
    platform::protect(addr as usize, size as usize, prot)
        .map_err(RuntimeError::new)?;
    Ok(Value::Unit)
}

fn mode_to_prot(mode: &str) -> RunResult<i32> {
    match mode {
        "r" => Ok(1),
        "rw" => Ok(1 | 2),
        "rx" => Ok(1 | 4),
        "rwx" => Ok(1 | 2 | 4),
        _ => Err(RuntimeError::new(format!("unknown protection mode `{mode}`"))),
    }
}

/// `mem::write_bytes(addr, data)` — copies `data` into memory at `addr`.
fn write_bytes(_cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [Value::Ptr(addr), Value::Bytes(data)] = args else {
        return Err(RuntimeError::new("`write_bytes` takes (ptr, bytes)"));
    };
    let data = lock(data);
    let dest = *addr as usize as *mut u8;
    unsafe {
        std::ptr::copy_nonoverlapping(data.as_ptr(), dest, data.len());
    }
    Ok(Value::Unit)
}

/// `mem::free_page(addr, size)` — frees a page allocated with `alloc_page`.
fn free_page(_cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [Value::Ptr(addr), Value::Int(size)] = args else {
        return Err(RuntimeError::new("`free_page` takes (ptr, size)"));
    };
    platform::free(*addr as usize, *size as usize).map_err(RuntimeError::new)?;
    Ok(Value::Unit)
}

#[cfg(unix)]
mod platform {
    use std::ffi::c_void;

    pub fn alloc_rw(size: usize) -> Result<usize, String> {
        let addr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_ANONYMOUS | libc::MAP_PRIVATE,
                -1,
                0,
            )
        };
        if addr == libc::MAP_FAILED {
            Err(format!("mmap failed: {}", std::io::Error::last_os_error()))
        } else {
            Ok(addr as usize)
        }
    }

    pub fn protect(addr: usize, size: usize, flags: i32) -> Result<(), String> {
        let mut prot = 0;
        if flags & 1 != 0 { prot |= libc::PROT_READ; }
        if flags & 2 != 0 { prot |= libc::PROT_WRITE; }
        if flags & 4 != 0 { prot |= libc::PROT_EXEC; }
        let ret = unsafe { libc::mprotect(addr as *mut c_void, size, prot) };
        if ret != 0 {
            Err(format!("mprotect failed: {}", std::io::Error::last_os_error()))
        } else {
            Ok(())
        }
    }

    pub fn free(addr: usize, size: usize) -> Result<(), String> {
        let ret = unsafe { libc::munmap(addr as *mut c_void, size) };
        if ret != 0 {
            Err(format!("munmap failed: {}", std::io::Error::last_os_error()))
        } else {
            Ok(())
        }
    }
}

#[cfg(windows)]
mod platform {
    pub fn alloc_rw(size: usize) -> Result<usize, String> {
        use windows_sys::Win32::System::Memory::*;
        let addr = unsafe {
            VirtualAlloc(
                std::ptr::null(),
                size,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        };
        if addr.is_null() {
            Err(format!("VirtualAlloc failed: {}", std::io::Error::last_os_error()))
        } else {
            Ok(addr as usize)
        }
    }

    pub fn protect(addr: usize, size: usize, flags: i32) -> Result<(), String> {
        use windows_sys::Win32::System::Memory::*;
        let prot = match (flags & 1 != 0, flags & 2 != 0, flags & 4 != 0) {
            (true, false, false) => PAGE_READONLY,
            (true, true, false) => PAGE_READWRITE,
            (true, false, true) => PAGE_EXECUTE_READ,
            (true, true, true) => PAGE_EXECUTE_READWRITE,
            (false, false, true) => PAGE_EXECUTE,
            _ => return Err(format!("unsupported protection flags {flags}")),
        };
        let mut old = 0u32;
        let ret = unsafe {
            VirtualProtect(addr as *const _, size, prot, &mut old)
        };
        if ret == 0 {
            Err(format!("VirtualProtect failed: {}", std::io::Error::last_os_error()))
        } else {
            Ok(())
        }
    }

    pub fn free(addr: usize, _size: usize) -> Result<(), String> {
        use windows_sys::Win32::System::Memory::*;
        let ret = unsafe { VirtualFree(addr as *mut _, 0, MEM_RELEASE) };
        if ret == 0 {
            Err(format!("VirtualFree failed: {}", std::io::Error::last_os_error()))
        } else {
            Ok(())
        }
    }
}

fn struct_to_bytes(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [value @ Value::Adt(adt)] = args else {
        return Err(RuntimeError::new("`to_bytes` takes a struct"));
    };
    let layout = layout_of_def(cx, adt.ctor, 0)?;
    let mut buffer = vec![0; layout.byte_size()];
    layout.write(cx, &mut buffer, 0, value)?;
    Ok(Value::Bytes(Arc::new(buffer.into())))
}
