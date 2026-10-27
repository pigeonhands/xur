//! `std::re::elf`: ELF files (Linux executables and `.so` libraries).

use std::sync::Arc;

use facet::Facet;
use pewter::elf::{SectionFlags, SegmentFlags};
use xur_runtime::registry::ModuleBuilder;

use super::flags::register_flags;
use super::parsed::Parsed;
use crate::error_text;

/// An ELF file, borrowing its bytes.
#[derive(Facet, Clone)]
#[facet(opaque)]
pub(super) struct File(pub(super) Arc<Parsed<pewter::elf::ElfFile<'static>>>);

impl File {
    fn parse(data: Vec<u8>) -> Result<Self, String> {
        let parsed = Parsed::new(data, pewter::elf::ElfFile::parse).map_err(error_text)?;
        Ok(Self(Arc::new(parsed)))
    }

    fn get(&self) -> &pewter::elf::ElfFile<'static> {
        self.0.get()
    }
}

impl std::fmt::Display for File {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let elf = self.get();
        let bits = if elf.is_64() { 64 } else { 32 };
        write!(
            f,
            "elf::File({bits}-bit, {} section(s))",
            elf.sections.len()
        )
    }
}

pub(super) fn register(elf: &mut ModuleBuilder<'_>) {
    elf.ty::<File>("File")
        .function("parse", File::parse)
        .function("open", |path: String| {
            File::parse(std::fs::read(&path).map_err(error_text)?)
        })
        .method("is_64", |f: &File| f.get().is_64())
        .method("header", |f: &File| f.get().header)
        .method("program_headers", |f: &File| {
            f.get().program_headers.clone()
        })
        .method("sections", |f: &File| f.get().sections.clone())
        .method("section_names", |f: &File| {
            f.get()
                .sections
                .iter()
                .map(|s| s.name.to_owned())
                .collect::<Vec<_>>()
        })
        .method("section", |f: &File, name: String| {
            f.get().section_by_name(&name).cloned()
        })
        .method("symbols", |f: &File| f.get().symbols().map_err(error_text))
        .method("find_symbol", |f: &File, name: String| {
            f.get().find_symbol(&name).map_err(error_text)
        })
        .method("relocations", |f: &File| {
            f.get().relocations().map_err(error_text)
        })
        .method("vaddr_to_offset", |f: &File, vaddr: u64| {
            f.get().vaddr_to_offset(vaddr)
        })
        .method("offset_to_vaddr", |f: &File, offset: usize| {
            f.get().offset_to_vaddr(offset)
        })
        .method("read", |f: &File, vaddr: u64, len: usize| {
            let data = f.get().data_at_vaddr(vaddr)?;
            data.get(..len).map(<[u8]>::to_vec)
        });

    register_flags!(elf, "section_flags", SectionFlags, u64);
    register_flags!(elf, "segment_flags", SegmentFlags, u32);
}
