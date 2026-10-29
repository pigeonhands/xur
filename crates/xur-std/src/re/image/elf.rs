//! `std::re::elf`: ELF files (Linux executables and `.so` libraries).
//!
//! `elf::ElfFile` reads a file; its fields (`header`, `sections`,
//! `program_headers`) are read straight through facet, and its methods are
//! the things pewter computes.

use pewter::elf::{ElfFile, SectionFlags, SegmentFlags};
use xur_runtime::registry::ModuleBuilder;

use crate::error_text;
use crate::re::flags::register_flags;
use crate::re::native::{open_bytes, open_path};

pub(super) fn register(elf: &mut ModuleBuilder<'_>) {
    elf.ty::<ElfFile<'static>>("ElfFile")
        .raw_function("parse", |cx, args| open_bytes(cx, args, ElfFile::parse)).returns::<ElfFile<'static>>()
        .raw_function("open", |cx, args| open_path(cx, args, ElfFile::parse)).returns::<ElfFile<'static>>()
        .method("is_64", |f: &ElfFile<'static>| f.is_64())
        .method("section_names", |f: &ElfFile<'static>| {
            f.sections
                .iter()
                .map(|s| s.name.to_owned())
                .collect::<Vec<_>>()
        })
        .method("section", |f: &ElfFile<'static>, name: String| {
            f.section_by_name(&name).cloned()
        })
        .method("symbols", |f: &ElfFile<'static>| {
            f.symbols().map_err(error_text)
        })
        .method("find_symbol", |f: &ElfFile<'static>, name: String| {
            f.find_symbol(&name).map_err(error_text)
        })
        .method("relocations", |f: &ElfFile<'static>| {
            f.relocations().map_err(error_text)
        })
        .method("vaddr_to_offset", |f: &ElfFile<'static>, vaddr: u64| {
            f.vaddr_to_offset(vaddr)
        })
        .method("offset_to_vaddr", |f: &ElfFile<'static>, offset: usize| {
            f.offset_to_vaddr(offset)
        })
        .method("read", |f: &ElfFile<'static>, vaddr: u64, len: usize| {
            f.data_at_vaddr(vaddr)
                .and_then(|data| data.get(..len))
                .map(<[u8]>::to_vec)
        });

    register_flags!(elf, "section_flags", SectionFlags, u64);
    register_flags!(elf, "segment_flags", SegmentFlags, u32);
}
