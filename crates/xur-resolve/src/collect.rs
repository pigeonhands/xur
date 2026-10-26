//! Declaring items in scopes (rustc's `build_reduced_graph`).

use xur_ast::{Item, ItemKind, VariantData, Visibility};
use xur_errors::Diag;
use xur_span::Ident;

use crate::def::{DefData, DefId, DefKind, Namespace, Res};
use crate::imports::Import;
use crate::scope::{Binding, Vis};
use crate::{Resolver, create_scoped_def};

impl<'a> Resolver<'a> {
    /// Declares every module's top-level items in the module's scope, and
    /// queues its `use` declarations.
    pub(crate) fn collect_items(&mut self) {
        let krate = self.krate;
        for (module, loaded) in self.module_defs.clone().into_iter().zip(&krate.modules) {
            for item in &loaded.ast.items {
                let vis = vis(item.vis);
                if let ItemKind::Use(tree) = &item.kind {
                    self.pending_imports.push(Import { module, tree, vis });
                    continue;
                }
                for (namespaces, ident, res) in self.define_item(module, module, item) {
                    let binding = Binding {
                        res,
                        vis,
                        span: Some(ident.span),
                    };
                    self.declare(module, namespaces, ident, binding);
                }
            }
        }
    }

    /// Creates the definitions for `item`, whose parent is `parent` and
    /// which is in `module`, and returns the names it introduces. `use`
    /// declarations introduce no names here; see [`Resolver::resolve_imports`].
    pub(crate) fn define_item(
        &mut self,
        parent: DefId,
        module: DefId,
        item: &Item,
    ) -> Vec<(&'static [Namespace], Ident, Res)> {
        let mut names = Vec::new();
        match &item.kind {
            ItemKind::Use(_) => {}
            ItemKind::Const(konst) => match self.import_target(item) {
                // `const math = std::import(...)` names a module, which is
                // both a value and a path prefix.
                Some(target) => {
                    let res = Res::Def(DefKind::Mod, target);
                    names.push((Namespace::BOTH, konst.ident, res));
                }
                None => {
                    let res = self.create_def(item, DefKind::Const, konst.ident, parent);
                    names.push((Namespace::VALUE, konst.ident, res));
                }
            },
            ItemKind::Fn(func) => {
                let res = self.create_def(item, DefKind::Fn, func.ident, parent);
                names.push((Namespace::VALUE, func.ident, res));
            }
            ItemKind::Struct(ident, _, data) => {
                let res = self.create_def(item, DefKind::Struct, *ident, parent);
                // Tuple and unit structs also name their constructor.
                let namespaces = match data {
                    VariantData::Struct(_) => Namespace::TYPE,
                    VariantData::Tuple(_) | VariantData::Unit => Namespace::BOTH,
                };
                names.push((namespaces, *ident, res));
            }
            ItemKind::Enum(ident, _, def) => {
                let data = DefData {
                    kind: DefKind::Enum,
                    name: ident.name,
                    parent: Some(parent),
                    span: Some(ident.span),
                };
                let enum_def =
                    create_scoped_def(&mut self.defs, &mut self.scopes, data, Some(module));
                self.node_defs.insert(item.id, enum_def);
                names.push((Namespace::TYPE, *ident, Res::Def(DefKind::Enum, enum_def)));

                for variant in &def.variants {
                    let def = self.defs.create(DefData {
                        kind: DefKind::Variant,
                        name: variant.ident.name,
                        parent: Some(enum_def),
                        span: Some(variant.ident.span),
                    });
                    self.node_defs.insert(variant.id, def);
                    let binding = Binding {
                        res: Res::Def(DefKind::Variant, def),
                        vis: Vis::Public,
                        span: Some(variant.ident.span),
                    };
                    // Variants are both constructors and patterns.
                    self.declare(enum_def, Namespace::BOTH, variant.ident, binding);
                }
            }
        }
        names
    }

    fn create_def(&mut self, item: &Item, kind: DefKind, ident: Ident, parent: DefId) -> Res {
        let def = self.defs.create(DefData {
            kind,
            name: ident.name,
            parent: Some(parent),
            span: Some(ident.span),
        });
        self.node_defs.insert(item.id, def);
        Res::Def(kind, def)
    }

    /// The module that a `const name = std::import(...)` item loads.
    fn import_target(&self, item: &Item) -> Option<DefId> {
        self.krate
            .modules
            .iter()
            .find_map(|module| module.imports.get(&item.id))
            .map(|&idx| self.module_def(idx))
    }

    /// Declares `ident` in `namespaces` of the scope of `scope`. A clash is
    /// reported once, however many namespaces it occurs in.
    pub(crate) fn declare(
        &mut self,
        scope: DefId,
        namespaces: &[Namespace],
        ident: Ident,
        binding: Binding,
    ) {
        let scope_data = self.scopes.get_mut(&scope).expect("declaring in a scope");
        let clash = namespaces
            .iter()
            .filter_map(|&ns| {
                scope_data
                    .insert(ns, ident.name, binding)
                    .err()
                    .map(|existing| (ns, existing))
            })
            .collect::<Vec<_>>();
        if let Some(&(ns, existing)) = clash.first() {
            self.report(duplicate_definition(ident, existing, ns));
        }
    }
}

fn vis(vis: Visibility) -> Vis {
    match vis {
        Visibility::Public(_) => Vis::Public,
        Visibility::Inherited => Vis::Private,
    }
}

fn duplicate_definition(ident: Ident, existing: Binding, ns: Namespace) -> Diag {
    let name = ident.name;
    let mut diag = Diag::error(
        format!("the name `{name}` is defined multiple times"),
        ident.span,
    )
    .with_label(ident.span, format!("`{name}` redefined here"));
    if let Some(span) = existing.span {
        let descr = existing.res.descr();
        diag = diag.with_label(
            span,
            format!("previous definition of the {descr} `{name}` here"),
        );
    }
    diag.with_note(format!(
        "`{name}` must be defined only once in the {} namespace of this module",
        ns.descr()
    ))
}
