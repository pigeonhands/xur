//! Parsing a binary into a native object that owns the bytes it borrows.
//!
//! pewter's file types (`PEFile<'a>`, `ElfFile<'a>`, `AnyImage<'a>`) borrow
//! the bytes they were parsed from. They are registered with their real
//! Rust type, so facet exposes every field, and each parsed value is kept
//! together with its bytes in one native object: the value borrows the
//! bytes for `'static`, and the bytes live exactly as long as the object.

use std::sync::Arc;

use facet::Facet;
use xur_runtime::{Context, RunResult, RuntimeError, Value};

use crate::{error_text, result};

/// Parses `data` with `parse` and returns, as a Xur `Result`, a native
/// object of the registered type `T` holding the bytes so the value's
/// borrows stay valid, or the parse error's message.
pub(super) fn parse_object<T, E>(
    cx: &dyn Context,
    data: Vec<u8>,
    parse: impl FnOnce(&'static [u8]) -> Result<T, E>,
) -> RunResult<Value>
where
    T: Facet<'static> + Send + 'static,
    E: std::fmt::Display,
{
    let data: Arc<Vec<u8>> = Arc::new(data);
    // SAFETY: the bytes live behind an `Arc` that is handed to the native
    // object as its keep-alive, so they never move or change and outlive
    // the parsed value, whose `'static` borrows of them stay valid.
    let bytes: &'static [u8] = unsafe { &*(data.as_slice() as *const [u8]) };
    let parsed = match parse(bytes) {
        Ok(value) => cx.registry().borrowing_object(value, data)?,
        Err(error) => return Ok(result(cx, Err(error_text(error)))),
    };
    Ok(result(cx, Ok(parsed)))
}

/// Builds a native object from `value`, which borrows the same bytes as
/// `keep_alive` (another parsed object's backing). For a part of a parsed
/// file, such as the ELF inside an `AnyImage`.
pub(super) fn borrow_from<T>(
    cx: &dyn Context,
    value: T,
    keep_alive: Arc<dyn std::any::Any + Send + Sync>,
) -> RunResult<Value>
where
    T: Facet<'static> + Send + 'static,
{
    cx.registry().borrowing_object(value, keep_alive)
}

/// The bytes a native object is keeping alive, if it has any, for handing a
/// part of it a share of the same backing.
pub(super) fn keep_alive_of(value: &Value) -> RunResult<Arc<dyn std::any::Any + Send + Sync>> {
    match value {
        Value::Native(object) => object
            .keep_alive()
            .ok_or_else(|| RuntimeError::new("this value does not own any bytes")),
        _ => Err(RuntimeError::new("expected a parsed file")),
    }
}
