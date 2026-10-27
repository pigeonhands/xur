//! `std::re::pe`: Windows PE files (`.exe`, `.dll`, `.sys`).
//!
//! `pe::File` reads a file as it is; `pe::ImageDef` is an image that can be
//! changed and written back out.

use std::sync::Arc;

use facet::{Facet, Peek};
use pewter::PEFile;
use pewter::pe::coff::{ImageFileCharacteristics, ImageFileMachine};
use pewter::pe::file::definition::{PEImageDef, SectionHeap};
use pewter::pe::optional_header::{
    ImageDllCharacteristics, OptVariant, OptionalHeaderWindowsSpecificFields, Pe32, Pe32Plus,
};
use pewter::pe::sections::edata::ExportDef;
use pewter::pe::sections::{SectionFlags, SectionRow};
use xur_runtime::convert::{ConvertCx, from_value, into_value, to_value};
use xur_runtime::registry::ModuleBuilder;
use xur_runtime::{Record, Registry, RunResult, RuntimeError, Value};
use xur_span::Symbol;

use super::flags::register_flags;
use super::parsed::Parsed;
use crate::error_text;

/// A PE file as read, borrowing its bytes.
#[derive(Facet, Clone)]
#[facet(opaque)]
pub(super) struct File(Arc<Parsed<PEFile<'static>>>);

impl File {
    fn parse(data: Vec<u8>) -> Result<Self, String> {
        let parsed = Parsed::new(data, PEFile::parse).map_err(error_text)?;
        Ok(Self(Arc::new(parsed)))
    }

    fn get(&self) -> &PEFile<'static> {
        self.0.get()
    }

    fn sections(&self) -> &[SectionRow<'static>] {
        &self.get().sections.0.0
    }
}

impl std::fmt::Display for File {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let file = self.get();
        write!(
            f,
            "pe::File({:?}, {} section(s))",
            file.coff_header.machine,
            self.sections().len()
        )
    }
}

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

    pe.ty::<File>("File")
        .function("parse", File::parse)
        .function("open", |path: String| {
            File::parse(std::fs::read(&path).map_err(error_text)?)
        })
        .method("dos_header", |f: &File| f.get().dos_header.clone())
        .method("coff_header", |f: &File| f.get().coff_header.clone())
        .method("optional_header", |f: &File| {
            f.get().optional_header.clone()
        })
        .method("sections", |f: &File| f.sections().to_vec())
        .method("section_names", |f: &File| {
            f.sections().iter().map(section_name).collect::<Vec<_>>()
        })
        .method("section", |f: &File, name: String| {
            f.sections()
                .iter()
                .find(|row| section_name(row) == name)
                .cloned()
        })
        .method("exports", |f: &File| {
            f.get().read_export_table().map_err(error_text)
        })
        .method("imports", |f: &File| {
            f.get().read_import_table().map_err(error_text)
        })
        .method("resources", |f: &File| {
            f.get().read_resource_directory().map_err(error_text)
        })
        .method("relocations", |f: &File| {
            f.get().read_base_relocation_table().map_err(error_text)
        })
        .method("exceptions", |f: &File| {
            f.get().read_exeption_table().map_err(error_text)
        })
        .method("certificates", |f: &File| {
            f.get().read_certificate_table().map_err(error_text)
        })
        .method("clr_header", |f: &File| {
            f.get().read_clr_runtime_header().map_err(error_text)
        })
        .method("to_image_def", |f: &File| {
            PEImageDef::from_pe_file(f.get().clone())
        });

    pe.ty::<PEImageDef>("ImageDef")
        .raw_function("new_dll", |cx, args| {
            let machine = match args {
                // A machine by its variant's name, like `"Amd64"`.
                [Value::Str(name)] => Value::Record(
                    Record {
                        ty: None,
                        variant: Some(Symbol::intern(name)),
                        fields: Vec::new(),
                    }
                    .into(),
                ),
                [machine @ Value::Record(_)] => machine.clone(),
                _ => {
                    return Err(RuntimeError::new(
                        "`new_dll` takes a machine, such as \"Amd64\"",
                    ));
                }
            };
            let machine: ImageFileMachine = from_value(cx, &machine)
                .map_err(|_| RuntimeError::new("unknown machine; try \"Amd64\" or \"I386\""))?;
            into_value(cx, PEImageDef::new_dll(machine))
        })
        .function("parse", |data: Vec<u8>| {
            PEImageDef::parse(&data).map_err(error_text)
        })
        .function("open", |path: String| {
            let data = std::fs::read(&path).map_err(error_text)?;
            PEImageDef::parse(&data).map_err(error_text)
        })
        .method("section_names", |def: &PEImageDef| {
            def.sections
                .iter()
                .map(|s| s.name.clone())
                .collect::<Vec<_>>()
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
