//! What native code can learn about Xur types at runtime, for `std::reflect`.

use xur_ast::Attribute;
use xur_resolve::{DefId, RtType};
use xur_span::Symbol;

/// A type as native code sees it: a type argument, as in `layout::<T>()`,
/// or the written type of a field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeRef {
    /// How the type was written, resolved: `u32`, `Point`, `[u8]`, or `_`
    /// if unknown.
    pub name: String,
    /// The struct, enum or native type it names, if any.
    pub def: Option<DefId>,
}

impl TypeRef {
    /// A type with a name only, such as a primitive.
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            def: None,
        }
    }

    /// `@`: record mode — values are coerced to records, and `T::new()`
    /// creates an empty record.
    pub fn record() -> Self {
        Self {
            name: "@".to_owned(),
            def: None,
        }
    }

    pub fn is_record(&self) -> bool {
        self.name == "@" && self.def.is_none()
    }
}

/// A struct or enum variant defined in Xur.
#[derive(Debug, Clone)]
pub struct AdtInfo {
    /// Its path, e.g. `Point` or `Shape::Circle`.
    pub name: String,
    /// The constructor of its values.
    pub ctor: DefId,
    /// Whether it is a variant of an enum, rather than a struct.
    pub is_variant: bool,
    /// The attributes written on it, such as `#[repr(C)]`.
    pub attrs: &'static [Attribute],
    /// Its fields, in declaration order.
    pub fields: Vec<FieldInfo>,
}

#[derive(Debug, Clone)]
pub struct FieldInfo {
    /// `None` for the fields of tuple structs and variants.
    pub name: Option<Symbol>,
    pub ty: TypeRef,
    pub attrs: &'static [Attribute],
}

impl AdtInfo {
    /// The names of the fields, or their numbers for a tuple struct.
    pub fn field_names(&self) -> Vec<String> {
        self.fields
            .iter()
            .enumerate()
            .map(|(index, field)| match field.name {
                Some(name) => name.to_string(),
                None => index.to_string(),
            })
            .collect()
    }
}

/// Reflection data for a function, closure, or constructor.
#[derive(Debug, Clone)]
pub struct FnInfo {
    pub name: String,
    pub param_count: usize,
    pub param_names: Vec<Option<String>>,
    pub param_types: Vec<Option<RtType>>,
    pub return_type: Option<RtType>,
}
