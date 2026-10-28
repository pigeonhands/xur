//! `std::mem`: sizes, alignment and byte-level conversion of structs.

use std::sync::Arc;

use xur_runtime::{BuiltinType, Context, Registry, RunResult, RuntimeError, Value, lock};

use crate::reflect::layout::{layout_of_def, type_arg_layout};

pub(crate) fn register(registry: &Registry) {
    registry
        .module("std::mem")
        .named_raw_function("size_of", &[], |cx, args| {
            let layout = type_arg_layout(cx, args, "size_of")?;
            Ok(Value::Int(layout.byte_size() as i64))
        })
        .named_raw_function("align_of", &[], |cx, args| {
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
        .named_raw_function("size_of_val", &["value"], |cx, args| {
            let [Value::Adt(adt)] = args else {
                return Err(RuntimeError::new("`size_of_val` takes a struct"));
            };
            Ok(Value::Int(layout_of_def(cx, adt.ctor, 0)?.byte_size() as i64))
        })
        .named_raw_function("to_bytes", &["value"], struct_to_bytes)
        .named_raw_function("alloc_page", &["size"], alloc_page)
        .named_raw_function("protect", &["addr", "size", "mode"], protect_page)
        .named_raw_function("write_bytes", &["addr", "data"], write_bytes)
        .named_raw_function("free_page", &["addr", "size"], free_page);
    registry
        .builtin(BuiltinType::Struct)
        .raw_function("to_bytes", struct_to_bytes)
        .raw_function("byte_size", |cx, args| {
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

/// `mem::protect(addr, size, mode)` — changes page protection.
/// `mode` is `"rx"`, `"rw"`, `"rwx"`, or `"r"`.
fn protect_page(_cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [Value::Ptr(addr), Value::Int(size), Value::Str(mode)] = args else {
        return Err(RuntimeError::new(
            "`protect` takes (ptr, size, mode) where mode is \"rx\", \"rw\", \"rwx\", or \"r\"",
        ));
    };
    platform::protect(*addr as usize, *size as usize, mode)
        .map_err(RuntimeError::new)?;
    Ok(Value::Unit)
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

    pub fn protect(addr: usize, size: usize, mode: &str) -> Result<(), String> {
        let prot = match mode {
            "r" => libc::PROT_READ,
            "rw" => libc::PROT_READ | libc::PROT_WRITE,
            "rx" => libc::PROT_READ | libc::PROT_EXEC,
            "rwx" => libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
            _ => return Err(format!("unknown protection mode `{mode}`")),
        };
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

    pub fn protect(addr: usize, size: usize, mode: &str) -> Result<(), String> {
        use windows_sys::Win32::System::Memory::*;
        let prot = match mode {
            "r" => PAGE_READONLY,
            "rw" => PAGE_READWRITE,
            "rx" => PAGE_EXECUTE_READ,
            "rwx" => PAGE_EXECUTE_READWRITE,
            _ => return Err(format!("unknown protection mode `{mode}`")),
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
