//! `std::re`: reverse engineering — reading executables, live processes and
//! machine code.
//!
//! Each backing library has its own place: [`pewter`] for files on disk
//! (`AnyImage` at `std::re`, the `pe` and `elf` submodules), procodile for
//! running processes ([`proc`], at `std::re::proc`), and iced-x86 for x86
//! disassembly and assembly ([`asm::x86`]), and hao for .NET assemblies
//! ([`dotnet`], at `std::re::dotnet`). The generic pieces are the shape
//! [`walk`] that registers a library's types from its facet shapes, the
//! [`native`] helpers for values that borrow their bytes, and the
//! bit[`flags`] helper.
//!
//! ```text
//! let image = std::re::AnyImage::open("game.dll").unwrap();
//! let pe = std::re::pe::PEFile::open("game.dll").unwrap();
//! pe.coff_header.machine                     // a field, read through facet
//!
//! let handle = std::re::proc::open(pid, "read").unwrap();
//! let bytes = handle.read(handle.modules().unwrap()[0].base, 16).unwrap();
//!
//! let insns = std::re::asm::x86::decode(64, bytes, 0x1000);
//!
//! let module = std::re::dotnet::open("Game.dll").unwrap();
//! for ty in module.all_types() { ... }
//! ```

#[cfg(feature = "re-asm")]
mod asm;
#[cfg(feature = "re-dotnet")]
mod dotnet;
#[allow(unused)]
mod flags;
#[cfg(feature = "re-image")]
mod image;
#[cfg_attr(not(any(feature = "re-image", feature = "re-proc")), allow(unused))]
mod native;
#[cfg(feature = "re-proc")]
mod proc;
pub mod walk;

use xur_runtime::Registry;

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::re", |re| {
        let registry = re.registry();
        let _ = &registry;
        #[cfg(feature = "re-image")]
        image::register(registry, re);
        #[cfg(feature = "re-proc")]
        proc::register(registry, re);
        #[cfg(feature = "re-asm")]
        asm::register(registry, re);
        #[cfg(feature = "re-dotnet")]
        dotnet::register(registry, re);
    });
}
