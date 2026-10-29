//! `std::re::AnyImage`: a PE or ELF file read by virtual address, as it
//! would be once a loader mapped it.

use pewter::image::{AnyImage, LoadedImage, PeImage};
use xur_runtime::registry::ModuleBuilder;
use xur_runtime::{Context, RunResult, RuntimeError, Value};

use crate::re::native::{borrow_from, keep_alive_of, open_bytes, open_path};

/// The `AnyImage` behind a receiver value, for a method that reads it.
fn image<R>(value: &Value, f: impl FnOnce(&AnyImage<'static>) -> R) -> RunResult<R> {
    match value {
        Value::Native(object) => object
            .with(f)
            .ok_or_else(|| RuntimeError::new("expected an `AnyImage`")),
        _ => Err(RuntimeError::new("expected an `AnyImage`")),
    }
}

fn format(image: &AnyImage<'_>) -> &'static str {
    match image {
        AnyImage::Pe(_) => "pe",
        AnyImage::Elf(_) => "elf",
    }
}

pub(super) fn register(re: &mut ModuleBuilder<'_>) {
    re.ty_borrowing::<AnyImage<'static>>("AnyImage")
        .raw_function_returns::<AnyImage<'static>>("load", |cx, args| open_bytes(cx, args, AnyImage::load))
        .raw_function_returns::<AnyImage<'static>>("open", |cx, args| open_path(cx, args, AnyImage::load))
        .method("format", |i: &AnyImage<'static>| format(i).to_owned())
        .method("is_64", |i: &AnyImage<'static>| i.is_64())
        .method("pointer_size", |i: &AnyImage<'static>| i.pointer_size())
        .method("machine", |i: &AnyImage<'static>| i.machine())
        .method("image_base", |i: &AnyImage<'static>| {
            i.as_pe().map(PeImage::image_base)
        })
        .method("bytes", |i: &AnyImage<'static>, vaddr: u64, len: usize| {
            i.bytes(vaddr, len).map(<[u8]>::to_vec)
        })
        .method("u8", |i: &AnyImage<'static>, vaddr: u64| i.u8(vaddr))
        .method("u32", |i: &AnyImage<'static>, vaddr: u64| i.u32(vaddr))
        .method("pointer", |i: &AnyImage<'static>, vaddr: u64| {
            i.pointer(vaddr)
        })
        .method(
            "pointer_at",
            |i: &AnyImage<'static>, base: u64, index: u64| i.pointer_at(base, index),
        )
        .method(
            "pointers",
            |i: &AnyImage<'static>, vaddr: u64, count: u64| i.pointers(vaddr, count),
        )
        .method("c_str", |i: &AnyImage<'static>, vaddr: u64| i.c_str(vaddr))
        .method("is_mapped", |i: &AnyImage<'static>, vaddr: u64| {
            i.is_mapped(vaddr)
        })
        .method("data_segments", |i: &AnyImage<'static>| i.data_segments())
        .method("exports", |i: &AnyImage<'static>| {
            i.exports().map_err(crate::error_text)
        })
        // The owned PE image definition; no borrow of the file's bytes.
        .method("as_pe", |i: &AnyImage<'static>| {
            i.as_pe().map(|pe| pe.file().clone())
        })
        // The ELF inside, sharing this image's bytes.
        .raw_function("as_elf", |cx, args| {
            let [this, ..] = args else {
                return Err(RuntimeError::new("`as_elf` takes no arguments"));
            };
            let Some(elf) = image(this, |i| i.as_elf().map(|e| e.elf().clone()))? else {
                return Ok(none(cx));
            };
            let value = borrow_from(cx, elf, keep_alive_of(this)?)?;
            Ok(some(cx, value))
        });
}

fn none(cx: &dyn Context) -> Value {
    Value::adt(cx.lang().none, Vec::new())
}

fn some(cx: &dyn Context, value: Value) -> Value {
    Value::adt(cx.lang().some, vec![value])
}
