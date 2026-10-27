//! Declaring items in scopes (rustc's `build_reduced_graph`).

use xur_ast::{Expr, Impl, Item, ItemKind, TyKind, VariantData, Visibility};
use xur_errors::Diag;
use xur_span::Ident;

use crate::def::{DefData, DefId, DefKind, Namespace, Res};
use crate::imports::Import;
use crate::loader::ModuleIdx;
use crate::loader::static_import;
use crate::scope::{Binding, Vis};
use crate::{Resolver, create_scoped_def};

impl<'a> Resolver<'a> {
    /// Declares every module's top-level items in the module's scope, and
    /// queues its `use` declarations.
    pub(crate) fn collect_items(&mut self, modules: &[ModuleIdx]) {
        let krate = self.krate;
        for &idx in modules {
            let (module, loaded) = (self.module_def(idx), krate.module(idx));
            for item in &loaded.ast.items {
                let vis = vis(item.vis);
                if let ItemKind::Use(trees) = &item.kind {
                    for tree in trees {
                        self.pending_imports.push(Import { module, tree, vis });
                    }
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
            // Imports and impls introduce no names of their own; see
            // `resolve_imports` and `collect_impls`.
            ItemKind::Use(_) | ItemKind::Impl(_) => {}
            ItemKind::Const(konst) => match self.static_import_target(&konst.expr) {
                // `const math = std::import("...")` names a module, which is
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
                let data_def = DefData {
                    kind: DefKind::Struct,
                    name: ident.name,
                    parent: Some(parent),
                    span: Some(ident.span),
                };
                let def =
                    create_scoped_def(&mut self.defs, &mut self.scopes, data_def, Some(module));
                self.node_defs.insert(item.id, def);
                let res = Res::Def(DefKind::Struct, def);
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

    /// The module `expr` loads, if it is `std::import` with a literal path.
    pub(crate) fn static_import_target(&self, expr: &Expr) -> Option<DefId> {
        let (callee, _, _) = static_import(expr)?;
        self.krate
            .modules
            .iter()
            .find_map(|module| module.imports.get(&callee))
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

impl Resolver<'_> {
    /// Declares the functions of every top-level `impl` block in `modules`
    /// in the scope of the struct or enum it is for. Runs after imports, as
    /// the type may be imported.
    pub(crate) fn collect_impls(&mut self, modules: &[ModuleIdx]) {
        let krate = self.krate;
        for &idx in modules {
            let module = self.module_def(idx);
            for item in &krate.module(idx).ast.items {
                if let ItemKind::Impl(imp) = &item.kind {
                    self.collect_impl(module, item, imp);
                }
            }
        }
    }

    fn collect_impl(&mut self, module: DefId, item: &Item, imp: &Impl) {
        let self_res = match &imp.self_ty.kind {
            TyKind::Path(path) => self.resolve_module_level_path(module, path, Namespace::Type),
            _ => Res::Err,
        };
        self.impl_self_tys.insert(item.id, self_res);
        let ty = match self_res {
            Res::Def(DefKind::Struct | DefKind::Enum, ty) => ty,
            Res::Err => return,
            _ => {
                let span = imp.self_ty.span;
                self.report(
                    Diag::error(
                        "`impl` blocks can only add functions to structs and enums",
                        span,
                    )
                    .with_label(span, format!("this is a {}", self_res.descr())),
                );
                return;
            }
        };
        for fn_item in &imp.items {
            let ItemKind::Fn(func) = &fn_item.kind else {
                continue;
            };
            let res = self.create_def(fn_item, DefKind::Fn, func.ident, ty);
            let binding = Binding {
                res,
                vis: vis(fn_item.vis),
                span: Some(func.ident.span),
            };
            self.declare(ty, Namespace::VALUE, func.ident, binding);
        }
    }
}
