//! Finding the syntax behind definitions.

use std::collections::HashMap;

use xur_ast::visit::{self, Visitor};
use xur_ast::{
    Attribute, ConstItem, FieldDef, Fn, Generics, Item, ItemKind, LitKind, MetaItemKind, NodeId,
    Variant, VariantData,
};
use xur_resolve::{DefId, ModuleIdx, Program, Resolutions};

/// What lowering needs to know about a definition.
#[derive(Debug, Clone, Copy)]
pub enum ItemRef {
    Fn {
        item: &'static Fn,
        attrs: &'static [Attribute],
        module: ModuleIdx,
        impl_generics: Option<&'static Generics>,
    },
    Const {
        item: &'static ConstItem,
        module: ModuleIdx,
    },
    /// A struct or enum variant: the shape of its constructor, and the
    /// attributes written on it.
    Ctor {
        data: &'static VariantData,
        attrs: &'static [Attribute],
        /// For `#[repr(inttype)]` enum variants, the integer discriminant.
        repr_value: Option<i64>,
    },
    /// An enum definition: its attributes (e.g. `#[try(unwrap = Ok)]`).
    Enum {
        attrs: &'static [Attribute],
    },
}

/// Every function, constant and constructor of the loaded modules, by
/// definition. Updated as modules are imported.
#[derive(Debug, Default)]
pub struct ItemIndex {
    items: HashMap<DefId, ItemRef>,
    /// How many of the program's modules have been indexed.
    indexed: usize,
}

impl ItemIndex {
    pub fn new(program: &Program) -> Self {
        let mut index = Self::default();
        index.update(program);
        index
    }

    /// Indexes the modules loaded since the last update.
    pub fn update(&mut self, program: &Program) {
        let modules = &program.krate.modules;
        for (i, module) in modules.iter().enumerate().skip(self.indexed) {
            let mut indexer = Indexer {
                resolutions: program.resolutions(),
                module: ModuleIdx(i),
                items: &mut self.items,
                current_impl: None,
                current_repr: None,
            };
            indexer.visit_module(module.ast);
        }
        self.indexed = modules.len();
    }

    pub fn get(&self, def: DefId) -> Option<ItemRef> {
        self.items.get(&def).copied()
    }

    /// The fields of the struct or variant `def`, named or positional, in
    /// declaration order, or `None` if `def` has no fields.
    pub fn fields(&self, def: DefId) -> Option<&'static [FieldDef]> {
        match self.get(def)? {
            ItemRef::Ctor {
                data: VariantData::Struct(fields) | VariantData::Tuple(fields),
                ..
            } => Some(fields),
            _ => None,
        }
    }

    /// The named fields of the struct or variant `def`, in declaration
    /// order, or `None` if it has no named fields.
    pub fn named_fields(&self, def: DefId) -> Option<&'static [FieldDef]> {
        match self.get(def)? {
            ItemRef::Ctor {
                data: VariantData::Struct(fields),
                ..
            } => Some(fields),
            ItemRef::Ctor {
                data: VariantData::Unit,
                ..
            } => Some(&[]),
            _ => None,
        }
    }

    /// The attributes on the struct or variant `def`.
    pub fn attrs(&self, def: DefId) -> Option<&'static [Attribute]> {
        match self.get(def)? {
            ItemRef::Ctor { attrs, .. } => Some(attrs),
            _ => None,
        }
    }

    /// The repr discriminant of a `#[repr(inttype)]` variant, if any.
    pub fn repr_value(&self, def: DefId) -> Option<i64> {
        match self.get(def)? {
            ItemRef::Ctor { repr_value, .. } => repr_value,
            _ => None,
        }
    }

    /// Every indexed definition.
    pub fn iter(&self) -> impl Iterator<Item = (DefId, ItemRef)> + '_ {
        self.items.iter().map(|(&def, &item)| (def, item))
    }
}

struct Indexer<'a> {
    resolutions: &'a Resolutions,
    module: ModuleIdx,
    items: &'a mut HashMap<DefId, ItemRef>,
    current_impl: Option<&'static Generics>,
    /// When inside a `#[repr(inttype)]` enum, the next auto-assigned
    /// discriminant value.
    current_repr: Option<i64>,
}

impl Indexer<'_> {
    fn add(&mut self, node: NodeId, item: ItemRef) {
        if let Some(&def) = self.resolutions.node_defs.get(&node) {
            self.items.insert(def, item);
        }
    }
}

/// Modules' ASTs live for the rest of the process, so the index can keep
/// references into them.
impl Visitor<'static> for Indexer<'_> {
    fn visit_item(&mut self, item: &'static Item) {
        let module = self.module;
        match &item.kind {
            ItemKind::Fn(func) => {
                let impl_generics = self.current_impl;
                self.add(item.id, ItemRef::Fn { item: func, attrs: &item.attrs, module, impl_generics });
            }
            ItemKind::Const(konst) => self.add(
                item.id,
                ItemRef::Const {
                    item: konst,
                    module,
                },
            ),
            ItemKind::Struct(_, _, data) => {
                let attrs = &item.attrs;
                self.add(item.id, ItemRef::Ctor { data, attrs, repr_value: None });
            }
            ItemKind::Enum(..) => {
                self.add(item.id, ItemRef::Enum { attrs: &item.attrs });
                self.current_repr = find_repr_attr(&item.attrs);
                visit::walk_item(self, item);
                self.current_repr = None;
                return;
            }
            ItemKind::Impl(imp) => {
                let saved = self.current_impl;
                self.current_impl = if imp.generics.params.is_empty() {
                    None
                } else {
                    Some(&imp.generics)
                };
                visit::walk_item(self, item);
                self.current_impl = saved;
                return;
            }
            ItemKind::Use(_) => {}
        }
        visit::walk_item(self, item);
    }

    fn visit_variant(&mut self, variant: &'static Variant) {
        let (data, attrs) = (&variant.data, &variant.attrs);
        let repr_value = if let Some(next) = &mut self.current_repr {
            let value = if let Some(disc) = &variant.discriminant {
                eval_discriminant(disc)
            } else {
                Some(*next)
            };
            if let Some(v) = value {
                *next = v + 1;
            }
            value
        } else {
            None
        };
        self.add(variant.id, ItemRef::Ctor { data, attrs, repr_value });
        visit::walk_variant(self, variant);
    }
}

/// If `attrs` contains `#[repr(u8)]`, `#[repr(u16)]`, `#[repr(u32)]` or
/// `#[repr(u64)]`, returns `Some(0)` as the initial discriminant.
fn find_repr_attr(attrs: &[Attribute]) -> Option<i64> {
    for attr in attrs {
        let segments = &attr.meta.path.segments;
        if segments.len() != 1 || segments[0].ident.name.as_str() != "repr" {
            continue;
        }
        if let MetaItemKind::List(items) = &attr.meta.kind {
            for item in items {
                if let xur_ast::MetaItemInner::MetaItem(meta) = item {
                    let name = meta.path.segments.last().map(|s| s.ident.name.as_str());
                    if matches!(name, Some("u8" | "u16" | "u32" | "u64" | "i8" | "i16" | "i32" | "i64")) {
                        return Some(0);
                    }
                }
            }
        }
    }
    None
}

/// Evaluates a discriminant expression to an integer. Only integer literals
/// are supported.
fn eval_discriminant(expr: &xur_ast::Expr) -> Option<i64> {
    match &expr.kind {
        xur_ast::ExprKind::Lit(lit) => match lit.kind {
            LitKind::Int(i) => i64::try_from(i).ok(),
            _ => None,
        },
        xur_ast::ExprKind::Unary(xur_ast::UnOp::Neg, inner) => {
            eval_discriminant(inner).map(|v| -v)
        }
        _ => None,
    }
}
