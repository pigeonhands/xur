//! `std::re`: reverse engineering — reading executables and live processes.
//!
//! Each backing library has its own place: [`pewter`] for files on disk
//! (`AnyImage` at `std::re`, the `pe` and `elf` submodules), and procodile
//! for running processes ([`proc`], at `std::re::proc`). The generic pieces
//! are the shape [`walk`] that registers a library's types from its facet
//! shapes, the [`native`] helpers for values that borrow their bytes, and
//! the bit[`flags`] helper.
//!
//! ```text
//! let image = std::re::AnyImage::open("game.dll").unwrap();
//! let pe = std::re::pe::PEFile::open("game.dll").unwrap();
//! pe.coff_header.machine                     // a field, read through facet
//!
//! let handle = std::re::proc::open(pid, "read").unwrap();
//! let bytes = handle.read(handle.modules().unwrap()[0].base, 16).unwrap();
//! ```

mod flags;
mod image;
mod native;
mod proc;
mod walk;

use xur_runtime::Registry;

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::re", |re| {
        let registry = re.registry();
        image::register(registry, re);
        proc::register(registry, re);
    });
}
