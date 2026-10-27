//! `std::re`: reverse engineering, reading and changing executables with
//! [pewter](https://docs.rs/pewter).
//!
//! `AnyImage::open(path)` loads a PE or ELF file and reads it by virtual
//! address, as it would be once loaded. `std::re::pe` and `std::re::elf`
//! have each format's own file types and every structure inside them,
//! registered by walking pewter's facet shapes, so headers, sections and
//! tables arrive as typed records:
//!
//! ```text
//! let image = std::re::AnyImage::open("game.dll").unwrap();
//! for export in image.exports().unwrap() { ... }
//!
//! let pe = std::re::pe::PEFile::open("game.dll").unwrap();
//! pe.coff_header.machine                     // a field, read through facet
//! std::re::pe::section_flags_names(pe.section(".text").unwrap().row.characteristics)
//! ```
//!
//! Bitflags come as integers; each set has a function naming its bits,
//! like `pe::section_flags_names`, and one making bits from names.

mod elf;
mod flags;
mod parsed;
mod pe;
mod shapes;

use facet::Facet;
use pewter::PEFile;
use pewter::elf::ElfFile;
use pewter::image::{AnyImage, ElfImage, LoadedImage, PeImage};
use xur_runtime::registry::ModuleBuilder;
use xur_runtime::{Context, Registry, RunResult, RuntimeError, Value};

use parsed::{borrow_from, keep_alive_of};
use pe::{open_bytes, open_path};

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::re", |re| {
        let registry = re.registry();
        pe::register(registry, &mut re.submodule("pe"));
        elf::register(&mut re.submodule("elf"));
        register_image(re);
        // The file types borrow their bytes, so they are registered above
        // with their Rust type (holding the bytes), not by the shape walk.
        let skip = [
            <PEFile<'static> as Facet>::SHAPE,
            <ElfFile<'static> as Facet>::SHAPE,
            <AnyImage<'static> as Facet>::SHAPE,
            <ElfImage<'static> as Facet>::SHAPE,
            <PeImage as Facet>::SHAPE,
        ];
        shapes::register(registry, re, &shapes::roots(), &skip);
    });
}

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

fn register_image(re: &mut ModuleBuilder<'_>) {
    re.ty_borrowing::<AnyImage<'static>>("AnyImage")
        .raw_function("load", |cx, args| open_bytes(cx, args, AnyImage::load))
        .raw_function("open", |cx, args| open_path(cx, args, AnyImage::load))
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
