//! `std::re`: reverse engineering, reading and changing executables with
//! [pewter](https://docs.rs/pewter).
//!
//! `Image::open(path)` loads a PE or ELF file and reads it by virtual
//! address, as it would be once loaded. `std::re::pe` and `std::re::elf`
//! have each format's own file types and every structure inside them,
//! registered by walking pewter's facet shapes, so headers, sections and
//! tables arrive as typed records:
//!
//! ```text
//! let image = std::re::Image::open("game.dll").unwrap();
//! for export in image.exports().unwrap() { ... }
//!
//! let pe = std::re::pe::File::open("game.dll").unwrap();
//! pe.coff_header().machine                  // ImageFileMachine::Amd64
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

use std::sync::Arc;

use facet::Facet;
use pewter::image::{AnyImage, ElfImage, LoadedImage, PeImage};
use xur_runtime::Registry;
use xur_runtime::registry::ModuleBuilder;

use crate::error_text;
use parsed::Parsed;

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::re", |re| {
        let registry = re.registry();
        pe::register(registry, &mut re.submodule("pe"));
        elf::register(&mut re.submodule("elf"));
        register_image(re);
        let skip = [
            <pewter::PEFile<'static> as Facet>::SHAPE,
            <pewter::elf::ElfFile<'static> as Facet>::SHAPE,
            <AnyImage<'static> as Facet>::SHAPE,
            <ElfImage<'static> as Facet>::SHAPE,
            <PeImage as Facet>::SHAPE,
        ];
        shapes::register(registry, re, &shapes::roots(), &skip);
    });
}

/// A PE or ELF file, read by virtual address.
#[derive(Facet, Clone)]
#[facet(opaque)]
struct Image(Arc<Parsed<AnyImage<'static>>>);

impl Image {
    fn parse(data: Vec<u8>) -> Result<Self, String> {
        let parsed = Parsed::new(data, AnyImage::load).map_err(error_text)?;
        Ok(Self(Arc::new(parsed)))
    }

    fn get(&self) -> &AnyImage<'static> {
        self.0.get()
    }

    fn format(&self) -> &'static str {
        match self.get() {
            AnyImage::Pe(_) => "pe",
            AnyImage::Elf(_) => "elf",
        }
    }
}

impl std::fmt::Display for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let bits = if self.get().is_64() { 64 } else { 32 };
        write!(f, "Image({}, {bits}-bit)", self.format())
    }
}

fn register_image(re: &mut ModuleBuilder<'_>) {
    re.ty::<Image>("Image")
        .function("parse", Image::parse)
        .function("open", |path: String| {
            Image::parse(std::fs::read(&path).map_err(error_text)?)
        })
        .method("format", |i: &Image| i.format().to_owned())
        .method("is_64", |i: &Image| i.get().is_64())
        .method("pointer_size", |i: &Image| i.get().pointer_size())
        .method("machine", |i: &Image| i.get().machine())
        .method("image_base", |i: &Image| {
            i.get().as_pe().map(PeImage::image_base)
        })
        .method("bytes", |i: &Image, vaddr: u64, len: usize| {
            i.get().bytes(vaddr, len).map(<[u8]>::to_vec)
        })
        .method("u8", |i: &Image, vaddr: u64| i.get().u8(vaddr))
        .method("u32", |i: &Image, vaddr: u64| i.get().u32(vaddr))
        .method("pointer", |i: &Image, vaddr: u64| i.get().pointer(vaddr))
        .method("pointer_at", |i: &Image, base: u64, index: u64| {
            i.get().pointer_at(base, index)
        })
        .method("pointers", |i: &Image, vaddr: u64, count: u64| {
            i.get().pointers(vaddr, count)
        })
        .method("c_str", |i: &Image, vaddr: u64| i.get().c_str(vaddr))
        .method("is_mapped", |i: &Image, vaddr: u64| {
            i.get().is_mapped(vaddr)
        })
        .method("data_segments", |i: &Image| i.get().data_segments())
        .method("exports", |i: &Image| i.get().exports().map_err(error_text))
        .method("pe", |i: &Image| {
            i.get().as_pe().map(|pe| pe.file().clone())
        })
        .method("elf", |i: &Image| {
            i.get().as_elf()?;
            Some(elf::File(Arc::new(i.0.derive(|image| match image {
                AnyImage::Elf(elf) => elf.elf().clone(),
                AnyImage::Pe(_) => unreachable!("checked above"),
            }))))
        });
}
