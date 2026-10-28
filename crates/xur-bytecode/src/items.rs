//! Finding the syntax behind definitions.

use std::collections::HashMap;

use xur_ast::visit::{self, Visitor};
use xur_ast::{Attribute, ConstItem, FieldDef, Fn, Item, ItemKind, NodeId, Variant, VariantData};
use xur_resolve::{DefId, ModuleIdx, Program, Resolutions};

/// What lowering needs to know about a definition.
#[derive(Debug, Clone, Copy)]
pub enum ItemRef {
    Fn {
        item: &'static Fn,
        attrs: &'static [Attribute],
        module: ModuleIdx,
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

    /// Every indexed definition.
    pub fn iter(&self) -> impl Iterator<Item = (DefId, ItemRef)> + '_ {
        self.items.iter().map(|(&def, &item)| (def, item))
    }
}

struct Indexer<'a> {
    resolutions: &'a Resolutions,
    module: ModuleIdx,
    items: &'a mut HashMap<DefId, ItemRef>,
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
            ItemKind::Fn(func) => self.add(item.id, ItemRef::Fn { item: func, attrs: &item.attrs, module }),
            ItemKind::Const(konst) => self.add(
                item.id,
                ItemRef::Const {
                    item: konst,
                    module,
                },
            ),
            ItemKind::Struct(_, _, data) => {
                let attrs = &item.attrs;
                self.add(item.id, ItemRef::Ctor { data, attrs });
            }
            ItemKind::Enum(..) => {
                self.add(item.id, ItemRef::Enum { attrs: &item.attrs });
            }
            ItemKind::Impl(_) | ItemKind::Use(_) => {}
        }
        visit::walk_item(self, item);
    }

    fn visit_variant(&mut self, variant: &'static Variant) {
        let (data, attrs) = (&variant.data, &variant.attrs);
        self.add(variant.id, ItemRef::Ctor { data, attrs });
        visit::walk_variant(self, variant);
    }
}
