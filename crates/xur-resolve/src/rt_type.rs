//! Runtime type representation, used for runtime type checks and reflection.

use crate::def::DefId;
use crate::native::NativeTypeId;

/// A type written in the source, as checked at runtime. Xur is checked
/// dynamically: where a type is written it is enforced when the value
/// arrives, and where it isn't (or is generic) anything goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RtType {
    /// Not checked: no type, `_`, or a type parameter.
    Any,
    Unit,
    Bool,
    Str,
    Bytes,
    Float { name: &'static str },
    /// An integer type, with the range its values must be in.
    Int {
        name: &'static str,
        min: i64,
        max: i64,
    },
    Tuple(Vec<RtType>),
    /// A list. Only the list itself is checked, not each element, so a
    /// check doesn't take time proportional to the list's length.
    List,
    /// A map (`#{}`). Like List, only the container is checked.
    Map,
    /// A struct or enum defined in Xur, by its definition.
    Adt {
        def: DefId,
        name: String,
    },
    Native {
        id: NativeTypeId,
        name: String,
    },
    /// `*T`, a raw pointer.
    Ptr,
    /// A function type `fn(A, B) -> C`.
    Fn {
        params: Vec<RtType>,
        ret: Box<RtType>,
    },
    /// The metatype: a value that is itself a type.
    Type,
}

impl std::fmt::Display for RtType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Any => f.write_str("_"),
            Self::Unit => f.write_str("()"),
            Self::Bool => f.write_str("bool"),
            Self::Str => f.write_str("str"),
            Self::Bytes => f.write_str("bytes"),
            Self::Float { name } => f.write_str(name),
            Self::Int { name, .. } => f.write_str(name),
            Self::Tuple(tys) => {
                let tys: Vec<_> = tys.iter().map(ToString::to_string).collect();
                write!(f, "({})", tys.join(", "))
            }
            Self::Ptr => f.write_str("*_"),
            Self::List => f.write_str("[_]"),
            Self::Map => f.write_str("map"),
            Self::Adt { name, .. } | Self::Native { name, .. } => f.write_str(name),
            Self::Fn { params, ret } => {
                let params: Vec<_> = params.iter().map(ToString::to_string).collect();
                write!(f, "fn({}) -> {}", params.join(", "), ret)
            }
            Self::Type => f.write_str("type"),
        }
    }
}
