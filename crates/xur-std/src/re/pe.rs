//! `std::re::pe`: Windows PE files (`.exe`, `.dll`, `.sys`).
//!
//! `pe::PEFile` reads a file as it is, its fields (`coff_header`,
//! `optional_header`, `sections`, ...) read straight through facet;
//! `pe::ImageDef` is an image that can be changed and written back out.

use facet::{Facet, Peek};
use pewter::PEFile;
use pewter::pe::coff::ImageFileCharacteristics;
use pewter::pe::file::definition::{PEImageDef, SectionHeap};
use pewter::pe::optional_header::{
    ImageDllCharacteristics, OptVariant, OptionalHeaderWindowsSpecificFields, Pe32, Pe32Plus,
};
use pewter::pe::sections::edata::ExportDef;
use pewter::pe::sections::{SectionFlags, SectionRow};
use xur_runtime::convert::{ConvertCx, to_value};
use xur_runtime::registry::ModuleBuilder;
use xur_runtime::{Context, Record, Registry, RunResult, Value};
use xur_span::Symbol;

use super::flags::register_flags;
use super::parsed::parse_object;
use crate::error_text;

/// A section's name, without the padding of its 8-byte field.
fn section_name(row: &SectionRow<'_>) -> String {
    let name = &row.row.name;
    let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    String::from_utf8_lossy(&name[..end]).into_owned()
}

pub(super) fn register(registry: &Registry, pe: &mut ModuleBuilder<'_>) {
    // Generic and opaque, so named here rather than by the shape walk.
    let pe32 = <OptionalHeaderWindowsSpecificFields<Pe32> as Facet>::SHAPE;
    let pe32_plus = <OptionalHeaderWindowsSpecificFields<Pe32Plus> as Facet>::SHAPE;
    registry.converter(pe32, windows_fields::<Pe32>);
    registry.converter(pe32_plus, windows_fields::<Pe32Plus>);
    pe.shape_type("OptionalHeaderWindowsSpecificFields32", pe32);
    pe.shape_type("OptionalHeaderWindowsSpecificFields64", pe32_plus);

    // pewter's own `PEFile`, holding the bytes it borrows. Its fields are
    // read through facet; only the computed tables are methods.
    pe.ty::<PEFile<'static>>("PEFile")
        .raw_function("parse", |cx, args| open_bytes(cx, args, PEFile::parse))
        .raw_function("open", |cx, args| open_path(cx, args, PEFile::parse))
        .method("section_names", |f: &PEFile<'static>| {
            f.sections.0.0.iter().map(section_name).collect::<Vec<_>>()
        })
        .method("section", |f: &PEFile<'static>, name: String| {
            f.sections
                .0
                .0
                .iter()
                .find(|row| section_name(row) == name)
                .cloned()
        })
        .method("exports", |f: &PEFile<'static>| {
            f.read_export_table().map_err(error_text)
        })
        .method("imports", |f: &PEFile<'static>| {
            f.read_import_table().map_err(error_text)
        })
        .method("resources", |f: &PEFile<'static>| {
            f.read_resource_directory().map_err(error_text)
        })
        .method("relocations", |f: &PEFile<'static>| {
            f.read_base_relocation_table().map_err(error_text)
        })
        .method("exceptions", |f: &PEFile<'static>| {
            f.read_exeption_table().map_err(error_text)
        })
        .method("certificates", |f: &PEFile<'static>| {
            f.read_certificate_table().map_err(error_text)
        })
        .method("clr_header", |f: &PEFile<'static>| {
            f.read_clr_runtime_header().map_err(error_text)
        })
        .method("to_image_def", |f: &PEFile<'static>| {
            PEImageDef::from_pe_file(f.clone())
        });

    // An editable image. Its fields are read through facet too; the methods
    // change it or compute things facet cannot.
    pe.ty::<PEImageDef>("ImageDef")
        .function("new_dll", PEImageDef::new_dll)
        .function("parse", |data: Vec<u8>| {
            PEImageDef::parse(&data).map_err(error_text)
        })
        .function("open", |path: String| {
            let data = std::fs::read(&path).map_err(error_text)?;
            PEImageDef::parse(&data).map_err(error_text)
        })
        .method("section", |def: &PEImageDef, name: String| {
            def.sections.by_name(&name).cloned()
        })
        .method("read_at_rva", |def: &PEImageDef, rva: u32, len: usize| {
            def.read_at_rva(rva, len).map(<[u8]>::to_vec)
        })
        .method("exports", |def: &PEImageDef| {
            def.read_export_table().map_err(error_text)
        })
        .method("entry_point", |def: &PEImageDef| {
            def.optional_header.standard_fields.address_of_entry_point
        })
        .method("image_base", |def: &PEImageDef| {
            def.optional_header.windows_specific_fields.image_base()
        })
        .method_mut("set_entry_point", |def: &mut PEImageDef, rva: u32| {
            def.optional_header.standard_fields.address_of_entry_point = rva;
        })
        .method_mut(
            "write_at_rva",
            |def: &mut PEImageDef, rva: u32, data: Vec<u8>| {
                def.write_at_rva(rva, &data).map_err(error_text)
            },
        )
        .method_mut(
            "add_section",
            |def: &mut PEImageDef, name: String, flags: u32, data: Vec<u8>| {
                let section: &mut SectionHeap =
                    def.new_section(name, SectionFlags::from_bits_retain(flags));
                section.data = data;
                section.virtual_address
            },
        )
        .method_mut(
            "add_exports",
            |def: &mut PEImageDef, dll_name: String, exports: Vec<(String, u32)>| {
                let exports: Vec<_> = exports
                    .into_iter()
                    .map(|(name, rva)| ExportDef::rva(&name, rva))
                    .collect();
                def.add_export_table(&dll_name, 1, &exports)
                    .map_err(error_text)
            },
        )
        .method_mut("strip_certificate", |def: &mut PEImageDef| {
            def.strip_certificate()
        })
        .method_mut("write", |def: &mut PEImageDef| {
            def.write_file().map_err(error_text)
        });

    register_flags!(pe, "section_flags", SectionFlags, u32);
    register_flags!(pe, "file_characteristics", ImageFileCharacteristics, u16);
    register_flags!(pe, "dll_characteristics", ImageDllCharacteristics, u16);
}

/// A `parse(bytes)` raw function for a type that borrows its bytes.
pub(super) fn open_bytes<T, E>(
    cx: &mut dyn Context,
    args: &[Value],
    parse: impl FnOnce(&'static [u8]) -> Result<T, E>,
) -> RunResult<Value>
where
    T: Facet<'static> + Send + 'static,
    E: std::fmt::Display,
{
    let [Value::Bytes(bytes)] = args else {
        return Err(xur_runtime::RuntimeError::new("expected bytes"));
    };
    let data = xur_runtime::lock(bytes).clone();
    parse_object(cx, data, parse)
}

/// An `open(path)` raw function for a type that borrows its bytes.
pub(super) fn open_path<T, E>(
    cx: &mut dyn Context,
    args: &[Value],
    parse: impl FnOnce(&'static [u8]) -> Result<T, E>,
) -> RunResult<Value>
where
    T: Facet<'static> + Send + 'static,
    E: std::fmt::Display,
{
    let [Value::Str(path)] = args else {
        return Err(xur_runtime::RuntimeError::new("expected a path string"));
    };
    match std::fs::read(&**path) {
        Ok(data) => parse_object(cx, data, parse),
        Err(error) => Ok(crate::result(cx, Err(error_text(error)))),
    }
}

/// The Windows-specific fields of the optional header, which facet sees as
/// opaque because their size depends on PE32 or PE32+.
fn windows_fields<A>(cx: &dyn ConvertCx, peek: Peek<'_, 'static>) -> RunResult<Value>
where
    A: OptVariant + 'static,
    A::Addr: Into<u64>,
    OptionalHeaderWindowsSpecificFields<A>: Facet<'static>,
{
    let f: &OptionalHeaderWindowsSpecificFields<A> =
        peek.get().map_err(xur_runtime::convert::reflect_error)?;
    let addr = |a: A::Addr| -> RunResult<Value> { to_value(cx, &a.into()) };
    let fields = [
        ("image_base", addr(f.image_base)?),
        ("section_alignment", Value::Int(f.section_alignment.into())),
        ("file_alignment", Value::Int(f.file_alignment.into())),
        (
            "major_operating_system_version",
            Value::Int(f.major_operating_system_version.into()),
        ),
        (
            "minor_operating_system_version",
            Value::Int(f.minor_operating_system_version.into()),
        ),
        (
            "major_image_version",
            Value::Int(f.major_image_version.into()),
        ),
        (
            "minor_image_version",
            Value::Int(f.minor_image_version.into()),
        ),
        (
            "major_subsystem_version",
            Value::Int(f.major_subsystem_version.into()),
        ),
        (
            "minor_subsystem_version",
            Value::Int(f.minor_subsystem_version.into()),
        ),
        (
            "win32_version_value",
            Value::Int(f.win32_version_value.into()),
        ),
        ("size_of_image", Value::Int(f.size_of_image.into())),
        ("size_of_headers", Value::Int(f.size_of_headers.into())),
        ("check_sum", Value::Int(f.check_sum.into())),
        ("subsystem", to_value(cx, &f.subsystem)?),
        (
            "dll_characteristics",
            Value::Int(f.dll_characteristics.bits().into()),
        ),
        ("size_of_stack_reserve", addr(f.size_of_stack_reserve)?),
        ("size_of_stack_commit", addr(f.size_of_stack_commit)?),
        ("size_of_heap_reserve", addr(f.size_of_heap_reserve)?),
        ("size_of_heap_commit", addr(f.size_of_heap_commit)?),
        ("loader_flags", Value::Int(f.loader_flags.into())),
        (
            "number_of_rva_and_sizes",
            Value::Int(f.number_of_rva_and_sizes.into()),
        ),
    ];
    let record = Record {
        ty: cx.registry().record_type_of_shape(peek.shape()),
        variant: None,
        fields: fields
            .into_iter()
            .map(|(name, value)| (Symbol::intern(name), value))
            .collect(),
    };
    Ok(Value::Record(record.into()))
}
