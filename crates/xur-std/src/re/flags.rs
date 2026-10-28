//! Naming the bits of pewter's bitflags, which come to Xur as integers.

use bitflags::{Bits, Flags};

/// The names of the flags set in `flags`.
pub(super) fn names<F: Flags + Copy>(flags: F) -> Vec<String> {
    F::FLAGS
        .iter()
        .filter(|flag| flag.value().bits() != F::Bits::EMPTY && flags.contains(*flag.value()))
        .map(|flag| flag.name().to_owned())
        .collect()
}

/// The flags named in `names`, combined.
pub(super) fn from_names<F: Flags>(names: Vec<String>) -> Result<F::Bits, String> {
    names
        .iter()
        .try_fold(F::empty(), |flags, name| {
            F::from_name(name)
                .map(|flag| flags.union(flag))
                .ok_or_else(|| format!("no flag `{name}`"))
        })
        .map(|flags| flags.bits())
}

/// Registers `<name>_names(bits)` and `<name>(names)` for the flags type
/// `F`, whose bits are `$bits`.
macro_rules! register_flags {
    ($module:expr, $name:literal, $flags:ty, $bits:ty) => {
        $module
            .function(concat!($name, "_names"), |bits: $bits| {
                $crate::re::flags::names(<$flags>::from_bits_retain(bits))
            })
            .function($name, |names: Vec<String>| {
                $crate::re::flags::from_names::<$flags>(names)
            });
    };
}

pub(super) use register_flags;

/// Collects the `(name, value)` pairs of a bitflags type for
/// [`ModuleBuilder::repr_enum`].
macro_rules! flag_variants {
    ($flags:ty, $bits:ty) => {{
        <$flags>::FLAGS
            .iter()
            .filter(|flag| flag.value().bits() != <$bits>::default())
            .map(|flag| (flag.name(), flag.value().bits() as i64))
            .collect::<Vec<(&str, i64)>>()
    }};
}

pub(super) use flag_variants;
