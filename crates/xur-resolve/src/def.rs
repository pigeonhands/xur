//! What names resolve to, after `rustc_hir::def`.

use std::fmt;

use xur_ast::NodeId;

use crate::native::{NativeFnId, NativeTypeId};
use xur_span::{Span, Symbol};

/// Identifies a definition: a module, item, enum variant, type parameter
/// or built-in.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DefId(u32);

impl fmt::Debug for DefId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DefId({})", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefKind {
    Mod,
    Fn,
    Const,
    Struct,
    Enum,
    Variant,
    TyParam,
    /// A function implemented in Rust.
    NativeFn(NativeFnId),
    /// A type implemented in Rust; its functions are in its scope.
    NativeType(NativeTypeId),
    /// A variant of an enum registered from Rust, of the native type.
    NativeVariant(NativeTypeId),
}

impl DefKind {
    /// How the kind is named in messages, e.g. "function".
    pub fn descr(self) -> &'static str {
        match self {
            Self::Mod => "module",
            Self::Fn => "function",
            Self::Const => "constant",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Variant => "variant",
            Self::TyParam => "type parameter",
            Self::NativeFn(_) => "native function",
            Self::NativeType(_) => "native type",
            Self::NativeVariant(_) => "variant",
        }
    }

    /// Whether names can be looked up inside a definition of this kind, as
    /// in `math::div`, `Shape::Rect` or `Point::new`.
    pub fn has_scope(self) -> bool {
        matches!(
            self,
            Self::Mod | Self::Enum | Self::Struct | Self::NativeType(_)
        )
    }
}

/// The built-in primitive types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimTy {
    Bool,
    Str,
    Bytes,
    U8,
    U16,
    U32,
    U64,
    Usize,
    I8,
    I16,
    I32,
    I64,
    Isize,
    F32,
    F64,
    /// The metatype: accepts values that are themselves types.
    Type,
}

impl PrimTy {
    pub const ALL: [PrimTy; 16] = [
        Self::Bool,
        Self::Str,
        Self::Bytes,
        Self::U8,
        Self::U16,
        Self::U32,
        Self::U64,
        Self::Usize,
        Self::I8,
        Self::I16,
        Self::I32,
        Self::I64,
        Self::Isize,
        Self::F32,
        Self::F64,
        Self::Type,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::Str => "str",
            Self::Bytes => "bytes",
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
            Self::Usize => "usize",
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::Isize => "isize",
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::Type => "type",
        }
    }

    pub fn from_name(name: Symbol) -> Option<Self> {
        Self::ALL.into_iter().find(|ty| ty.name() == name.as_str())
    }
}

/// What a name resolved to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Res {
    Def(DefKind, DefId),
    /// A local variable, identified by the pattern that binds it.
    Local(NodeId),
    PrimTy(PrimTy),
    /// `T::name` where `T` is a type parameter: resolved at runtime.
    TyParamAssoc(DefId, Symbol),
    /// Resolution failed; the error has been reported.
    Err,
}

impl Res {
    pub fn descr(self) -> &'static str {
        match self {
            Self::Def(kind, _) => kind.descr(),
            Self::Local(_) => "local variable",
            Self::PrimTy(_) => "builtin type",
            Self::TyParamAssoc(..) => "type parameter associated function",
            Self::Err => "unresolved item",
        }
    }

    /// The definition to look further names up in, for `a::b` paths.
    pub fn scope(self) -> Option<DefId> {
        match self {
            Self::Def(kind, id) if kind.has_scope() => Some(id),
            _ => None,
        }
    }
}

/// The two namespaces of names (rustc also has one for macros). A type and
/// a value may share a name; a tuple struct or variant is in both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Namespace {
    Type,
    Value,
}

impl Namespace {
    pub const TYPE: &[Namespace] = &[Namespace::Type];
    pub const VALUE: &[Namespace] = &[Namespace::Value];
    /// For names that are both a type and a value, like tuple structs.
    pub const BOTH: &[Namespace] = &[Namespace::Type, Namespace::Value];

    pub fn descr(self) -> &'static str {
        match self {
            Self::Type => "type",
            Self::Value => "value",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DefData {
    pub kind: DefKind,
    pub name: Symbol,
    /// The module or enum the definition is in. `None` for file modules
    /// and the `std` root.
    pub parent: Option<DefId>,
    /// Where the definition's name was written; `None` for built-ins.
    pub span: Option<Span>,
}

/// Every definition in the program.
#[derive(Debug, Default)]
pub struct Definitions {
    defs: Vec<DefData>,
}

impl Definitions {
    pub fn create(&mut self, data: DefData) -> DefId {
        let id = DefId(u32::try_from(self.defs.len()).expect("too many definitions"));
        self.defs.push(data);
        id
    }

    pub fn get(&self, id: DefId) -> &DefData {
        &self.defs[id.0 as usize]
    }

    pub fn iter(&self) -> impl Iterator<Item = (DefId, &DefData)> {
        self.defs
            .iter()
            .enumerate()
            .map(|(i, d)| (DefId(i as u32), d))
    }

    /// `id` and its parents, outermost first: for `std::io::println`, the
    /// definitions of `std`, `io` and `println`.
    pub fn lineage(&self, id: DefId) -> Vec<DefId> {
        let mut lineage = Vec::new();
        let mut current = Some(id);
        while let Some(id) = current {
            lineage.push(id);
            current = self.get(id).parent;
        }
        lineage.reverse();
        lineage
    }
}
