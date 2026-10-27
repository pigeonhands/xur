//! Registering every type of pewter by walking its facet shapes.
//!
//! Each struct and enum reachable from `pewter::reflect::roots()` becomes a
//! Xur type in the module matching where pewter defines it: `pewter::pe::*`
//! in `std::re::pe`, `pewter::elf::*` in `std::re::elf`, everything else in
//! `std::re`. Their values come to Xur as typed records.

use facet::{Def, PointerType, Shape, Type, UserType};
use xur_runtime::Registry;
use xur_runtime::registry::ModuleBuilder;

/// The shapes to start from: pewter's roots, and types that only its
/// functions return.
pub(super) fn roots() -> Vec<&'static Shape> {
    let mut roots = pewter::reflect::roots().to_vec();
    roots.extend([
        <pewter::image::Export as facet::Facet>::SHAPE,
        <pewter::elf::Symbol<'static> as facet::Facet>::SHAPE,
        <pewter::pe::sections::edata::ExportTableDataDirectory as facet::Facet>::SHAPE,
        <pewter::pe::sections::idata::ImportTableDataDirectory as facet::Facet>::SHAPE,
    ]);
    roots
}

/// Registers every pewter type reachable from `roots` that is not already
/// registered, except those in `skip`, which are wrapped by hand.
pub(super) fn register(
    registry: &Registry,
    re: &mut ModuleBuilder<'_>,
    roots: &[&'static Shape],
    skip: &[&'static Shape],
) {
    let mut names: Vec<(&str, String)> = Vec::new();
    for shape in reachable(roots) {
        let Some(module) = pewter_module(shape) else {
            continue;
        };
        if skip.contains(&shape) || registry.has_shape(shape) || !is_registrable(registry, shape) {
            continue;
        }
        // Generic types are named with their arguments when the plain name
        // is taken, as `OptionalHeaderWindowsSpecificFieldsPe32Plus`.
        let mut name = shape.type_identifier.to_owned();
        if names.iter().any(|(m, n)| *m == module && *n == name) {
            for param in shape.type_params {
                name.push_str(param.shape.type_identifier);
            }
        }
        names.push((module, name.clone()));
        let mut submodule;
        let builder = match module {
            "" => &mut *re,
            module => {
                submodule = re.submodule(module);
                &mut submodule
            }
        };
        builder.shape_type(&name, shape);
    }
}

/// Which submodule of `std::re` a pewter type goes in (`""` for `std::re`
/// itself), or `None` if it isn't pewter's.
fn pewter_module(shape: &Shape) -> Option<&'static str> {
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

/// Structs and enums, and opaque types converted by hand (the others'
/// values are just bits).
fn is_registrable(registry: &Registry, shape: &'static Shape) -> bool {
    match shape.ty {
        Type::User(UserType::Struct(_) | UserType::Enum(_)) => true,
        Type::User(UserType::Opaque) => registry.has_converter(shape),
        _ => false,
    }
}

/// Every shape reachable from `roots` through fields, variants, type
/// arguments and the items of containers, in the order found.
fn reachable(roots: &[&'static Shape]) -> Vec<&'static Shape> {
    let mut found: Vec<&'static Shape> = Vec::new();
    let mut stack: Vec<&'static Shape> = roots.iter().rev().copied().collect();
    while let Some(shape) = stack.pop() {
        if found.contains(&shape) {
            continue;
        }
        found.push(shape);
        let mut children = children(shape);
        children.reverse();
        stack.extend(children);
    }
    found
}

fn children(shape: &'static Shape) -> Vec<&'static Shape> {
    let mut children: Vec<&'static Shape> = shape.type_params.iter().map(|p| p.shape).collect();
    children.extend(shape.inner);
    match shape.def {
        Def::Option(def) => children.push(def.t()),
        Def::List(def) => children.push(def.t()),
        Def::Array(def) => children.push(def.t()),
        Def::Slice(def) => children.push(def.t()),
        Def::Result(def) => children.extend([def.t(), def.e()]),
        _ => {}
    }
    match shape.ty {
        Type::User(UserType::Struct(st)) => {
            children.extend(st.fields.iter().map(|field| field.shape()));
        }
        Type::User(UserType::Enum(en)) => {
            for variant in en.variants {
                children.extend(variant.data.fields.iter().map(|field| field.shape()));
            }
        }
        Type::Pointer(PointerType::Reference(ptr) | PointerType::Raw(ptr)) => {
            children.push(ptr.target);
        }
        _ => {}
    }
    children
}
