//! `std::re` image parsing, backed by [pewter](https://docs.rs/pewter): the
//! `AnyImage` loaded view at `std::re`, the `pe` and `elf` submodules, and
//! every type inside them, registered by walking pewter's facet shapes.

mod any;
mod elf;
mod pe;

use facet::{Facet, Shape};
use pewter::PEFile;
use pewter::elf::ElfFile;
use pewter::image::{AnyImage, ElfImage, PeImage};
use xur_runtime::Registry;
use xur_runtime::registry::ModuleBuilder;

use super::walk;

pub(super) fn register(registry: &Registry, re: &mut ModuleBuilder<'_>) {
    any::register(re);
    pe::register(registry, &mut re.submodule("pe"));
    elf::register(&mut re.submodule("elf"));
    // The file types borrow their bytes, so they are registered above with
    // their Rust type (holding the bytes), not by the shape walk.
    let skip = [
        <PEFile<'static> as Facet>::SHAPE,
        <ElfFile<'static> as Facet>::SHAPE,
        <AnyImage<'static> as Facet>::SHAPE,
        <ElfImage<'static> as Facet>::SHAPE,
        <PeImage as Facet>::SHAPE,
    ];
    walk::register(registry, re, &roots(), submodule_of, &skip);
}

/// pewter's shapes to start the walk from: its roots, and types that only
/// its functions return.
fn roots() -> Vec<&'static Shape> {
    let mut roots = pewter::reflect::roots().to_vec();
    roots.extend([
        <pewter::image::Export as Facet>::SHAPE,
        <pewter::elf::Symbol<'static> as Facet>::SHAPE,
        <pewter::pe::sections::edata::ExportTableDataDirectory as Facet>::SHAPE,
        <pewter::pe::sections::idata::ImportTableDataDirectory as Facet>::SHAPE,
    ]);
    roots
}

/// Which submodule of `std::re` a pewter type goes in (`""` for `std::re`
/// itself), or `None` if it isn't pewter's.
fn submodule_of(shape: &Shape) -> Option<&'static str> {
    let path = shape.module_path?;
    let mut segments = path.split("::");
    if segments.next() != Some("pewter") {
        return None;
    }
    Some(match segments.next() {
        Some("pe") => "pe",
        Some("elf") => "elf",
        _ => "",
    })
}
