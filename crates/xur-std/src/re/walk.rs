//! Registering a library's types by walking its facet shapes.
//!
//! Given a library's entry-point shapes, this reaches every struct and enum
//! they refer to (through fields, variants, type arguments and containers)
//! and registers each as a Xur type, in the submodule a caller-supplied
//! mapping chooses. Nothing here knows about any particular library.

use facet::{Def, PointerType, Shape, Type, UserType};
use xur_runtime::Registry;
use xur_runtime::registry::ModuleBuilder;

/// Registers every type reachable from `roots` into `base`, each in the
/// submodule `submodule_of` gives it (`""` for `base` itself, `None` to
/// skip — a type wrapped by hand, or not this library's), unless it is
/// already registered or has no fields to show.
pub fn register(
    registry: &Registry,
    base: &mut ModuleBuilder<'_>,
    roots: &[&'static Shape],
    submodule_of: impl Fn(&Shape) -> Option<&'static str>,
    skip: &[&'static Shape],
) {
    let mut names: Vec<(&str, String)> = Vec::new();
    for shape in reachable(roots) {
        let Some(module) = submodule_of(shape) else {
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
            "" => &mut *base,
            module => {
                submodule = base.submodule(module);
                &mut submodule
            }
        };
        builder.shape_type(&name, shape);
    }
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
