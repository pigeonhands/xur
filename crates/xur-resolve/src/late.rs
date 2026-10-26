//! Resolving the names used inside items: function bodies, types,
//! patterns and generics (rustc's `late` resolution).
//!
//! Local names live in *ribs*, a stack of scopes pushed for blocks,
//! functions, closures, match arms and loops. A rib at each item boundary
//! stops a nested function from seeing the enclosing function's locals and
//! type parameters, while items declared in blocks stay visible.

use std::collections::HashMap;

use xur_ast::visit::{self, Visitor};
use xur_ast::{
    Arm, Block, Expr, ExprKind, FnRetTy, Generics, Impl, Item, ItemKind, Local, NodeId, Pat,
    PatKind, Path, StmtKind, Ty, TyKind,
};
use xur_errors::Diag;
use xur_span::{Ident, Span, Symbol};

use crate::Resolver;
use crate::def::{DefData, DefId, DefKind, Namespace, Res};
use crate::ident::{Lookup, undeclared_module};
use crate::loader::ModuleIdx;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RibKind {
    /// A block, closure, match arm or loop body.
    Normal,
    /// The boundary of an item: locals and type parameters of enclosing
    /// items are not visible past it.
    Item,
}

#[derive(Debug, Clone, Copy)]
struct RibBinding {
    res: Res,
    /// For a local initialised directly with a module (`let io = std::io;`),
    /// that module, so the local can prefix paths like `io::println`.
    module: Option<DefId>,
}

#[derive(Debug)]
struct Rib {
    kind: RibKind,
    bindings: HashMap<Symbol, RibBinding>,
}

impl Rib {
    fn new(kind: RibKind) -> Self {
        Self {
            kind,
            bindings: HashMap::new(),
        }
    }
}

impl Resolver<'_> {
    /// Resolves the names used inside every module's items.
    pub(crate) fn resolve_bodies(&mut self, modules: &[ModuleIdx]) {
        let krate = self.krate;
        for &idx in modules {
            let (module, loaded) = (self.module_def(idx), krate.module(idx));
            let mut late = LateResolver {
                r: self,
                module,
                value_ribs: Vec::new(),
                type_ribs: Vec::new(),
                pat_bindings: HashMap::new(),
                top_level: true,
            };
            late.visit_module(loaded.ast);
        }
    }

    /// Resolves `path` as written at the top level of `module`.
    pub(crate) fn resolve_module_level_path(
        &mut self,
        module: DefId,
        path: &Path,
        ns: Namespace,
    ) -> Res {
        let mut late = LateResolver {
            r: self,
            module,
            value_ribs: Vec::new(),
            type_ribs: Vec::new(),
            pat_bindings: HashMap::new(),
            top_level: true,
        };
        late.resolve_path(path, ns)
    }
}

struct LateResolver<'r, 'a> {
    r: &'r mut Resolver<'a>,
    /// The module being resolved.
    module: DefId,
    value_ribs: Vec<Rib>,
    type_ribs: Vec<Rib>,
    /// The names bound by the pattern being resolved, to reject `(a, a)`.
    pat_bindings: HashMap<Symbol, Span>,
    /// Whether the items being visited are at the top level of the module.
    top_level: bool,
}

impl LateResolver<'_, '_> {
    fn with_ribs(&mut self, kind: RibKind, f: impl FnOnce(&mut Self)) {
        self.value_ribs.push(Rib::new(kind));
        self.type_ribs.push(Rib::new(kind));
        f(self);
        self.value_ribs.pop();
        self.type_ribs.pop();
    }

    fn ribs(&self, ns: Namespace) -> &[Rib] {
        match ns {
            Namespace::Type => &self.type_ribs,
            Namespace::Value => &self.value_ribs,
        }
    }

    /// Looks a single name up: innermost rib first, then the module.
    fn resolve_ident(&self, ns: Namespace, name: Symbol) -> Option<RibBinding> {
        let mut crossed_item = false;
        for rib in self.ribs(ns).iter().rev() {
            if let Some(&binding) = rib.bindings.get(&name) {
                let is_local = matches!(binding.res, Res::Local(_) | Res::Def(DefKind::TyParam, _));
                if !(crossed_item && is_local) {
                    return Some(binding);
                }
            }
            crossed_item |= rib.kind == RibKind::Item;
        }
        let binding = self.r.lookup_in_module(self.module, ns, name)?;
        Some(RibBinding {
            res: binding.res,
            module: None,
        })
    }

    /// Resolves `path` in `ns` and records the result.
    fn resolve_path(&mut self, path: &Path, ns: Namespace) -> Res {
        let res = match self.try_resolve_path(path, ns) {
            Ok(res) => res,
            Err(diag) => {
                if let Some(diag) = diag {
                    self.r.report(diag);
                }
                Res::Err
            }
        };
        self.r.res.insert(path.id, res);
        res
    }

    fn try_resolve_path(&mut self, path: &Path, ns: Namespace) -> Lookup<Res> {
        let (last, prefix) = path.segments.split_last().expect("paths have a segment");
        let [first, rest @ ..] = prefix else {
            let binding = self
                .resolve_ident(ns, last.ident.name)
                .ok_or_else(|| Some(cannot_find(ns, last.ident)))?;
            return Ok(binding.res);
        };

        let start = self
            .resolve_path_start(first.ident.name)
            .ok_or_else(|| Some(undeclared_module(first.ident)))?;
        let mut privacy = Vec::new();
        let scope = self
            .r
            .resolve_prefix(start, first.ident, rest, self.module, &mut privacy)?;
        let binding = self
            .r
            .lookup_in_scope(scope, ns, last.ident.name)
            .ok_or_else(|| Some(self.r.not_found_in(scope, ns, last.ident)))?;
        privacy.extend(
            self.r
                .privacy_error(scope, binding, last.ident, self.module),
        );
        self.r.diags.extend(privacy);
        Ok(binding.res)
    }

    /// Resolves the first segment of a multi-segment path, which should
    /// name a module or enum, possibly through a local alias.
    fn resolve_path_start(&self, name: Symbol) -> Option<Res> {
        if let Some(binding) = self.resolve_ident(Namespace::Value, name)
            && let Res::Local(_) = binding.res
        {
            return Some(
                binding
                    .module
                    .map_or(binding.res, |module| Res::Def(DefKind::Mod, module)),
            );
        }
        self.resolve_ident(Namespace::Type, name)
            .map(|binding| binding.res)
            .or_else(|| {
                self.r
                    .lookup_path_start_in_module(self.module, name)
                    .map(|b| b.res)
            })
    }

    /// Introduces the local variable bound by pattern `pat_id` in the
    /// innermost rib.
    fn bind_local(&mut self, ident: Ident, pat_id: NodeId) {
        if let Some(&first) = self.pat_bindings.get(&ident.name) {
            self.r.report(
                Diag::error(
                    format!(
                        "identifier `{}` is bound more than once in the same pattern",
                        ident.name
                    ),
                    ident.span,
                )
                .with_label(ident.span, "used in a pattern more than once")
                .with_label(first, "first bound here"),
            );
        }
        self.pat_bindings.insert(ident.name, ident.span);

        let res = Res::Local(pat_id);
        let rib = self
            .value_ribs
            .last_mut()
            .expect("patterns are resolved inside a rib");
        rib.bindings
            .insert(ident.name, RibBinding { res, module: None });
        self.r.res.insert(pat_id, res);
        self.r.locals.insert(pat_id, ident);
    }

    /// Resolves a whole pattern, binding its variables in the innermost rib.
    fn resolve_pattern(&mut self, pat: &Pat) {
        self.pat_bindings.clear();
        self.visit_pat(pat);
        self.pat_bindings.clear();
    }

    /// Declares the type parameters of an item in the innermost type rib.
    fn declare_generics(&mut self, item: &Item, generics: &Generics) {
        let parent = self.r.node_defs.get(&item.id).copied();
        for param in &generics.params {
            let kind = DefKind::TyParam;
            let def = self.r.defs.create(DefData {
                kind,
                name: param.ident.name,
                parent,
                span: Some(param.ident.span),
            });
            self.r.node_defs.insert(param.id, def);
            let rib = self
                .type_ribs
                .last_mut()
                .expect("generics are declared inside a rib");
            let binding = RibBinding {
                res: Res::Def(kind, def),
                module: None,
            };
            if rib.bindings.insert(param.ident.name, binding).is_some() {
                self.r.report(
                    Diag::error(
                        format!(
                            "the name `{}` is already used for a type parameter in this list",
                            param.ident.name
                        ),
                        param.ident.span,
                    )
                    .with_label(param.ident.span, "already used"),
                );
            }
        }
    }

    /// Declares the items of `block` in the innermost ribs, so they are
    /// visible throughout the block.
    fn declare_block_items(&mut self, block: &Block) {
        for stmt in &block.stmts {
            let StmtKind::Item(item) = &stmt.kind else {
                continue;
            };
            for (namespaces, ident, res) in self.r.define_item(self.module, self.module, item) {
                for &ns in namespaces {
                    let rib = match ns {
                        Namespace::Type => self.type_ribs.last_mut(),
                        Namespace::Value => self.value_ribs.last_mut(),
                    };
                    let binding = RibBinding { res, module: None };
                    rib.expect("blocks have ribs")
                        .bindings
                        .insert(ident.name, binding);
                }
            }
        }
    }

    fn resolve_fn(&mut self, item: &Item, func: &xur_ast::Fn, kind: RibKind) {
        self.with_ribs(kind, |this| {
            this.declare_generics(item, &func.generics);
            for param in &func.decl.inputs {
                this.visit_ty(&param.ty);
                this.resolve_pattern(&param.pat);
            }
            if let FnRetTy::Ty(ty) = &func.decl.output {
                this.visit_ty(ty);
            }
            this.visit_block(&func.body);
        });
    }

    /// Resolves the functions of an `impl` block, with the impl's type
    /// parameters and `Self` in scope.
    fn resolve_impl(&mut self, item: &Item, imp: &Impl) {
        let self_res = self
            .r
            .impl_self_tys
            .get(&item.id)
            .copied()
            .unwrap_or(Res::Err);
        self.with_ribs(RibKind::Item, |this| {
            this.declare_generics(item, &imp.generics);
            let rib = this.type_ribs.last_mut().expect("impls have ribs");
            let binding = RibBinding {
                res: self_res,
                module: None,
            };
            rib.bindings.insert(Symbol::intern("Self"), binding);

            // The type's path was resolved when the impl was collected;
            // only its generic arguments are left.
            visit::walk_ty(this, &imp.self_ty);
            for fn_item in &imp.items {
                if let ItemKind::Fn(func) = &fn_item.kind {
                    // Functions see the impl's type parameters, so their
                    // rib does not mark an item boundary.
                    this.resolve_fn(fn_item, func, RibKind::Normal);
                }
            }
        });
    }

    /// The module `expr` statically evaluates to: a path naming a module,
    /// or `std::import` with a literal path.
    fn static_module(&self, expr: &Expr) -> Option<DefId> {
        match &expr.kind {
            ExprKind::Path(path) => match self.r.res.get(&path.id) {
                Some(&Res::Def(DefKind::Mod, module)) => Some(module),
                _ => None,
            },
            _ => self.r.static_import_target(expr),
        }
    }
}

impl<'ast> Visitor<'ast> for LateResolver<'_, '_> {
    fn visit_item(&mut self, item: &'ast Item) {
        let top_level = std::mem::replace(&mut self.top_level, false);
        match &item.kind {
            ItemKind::Use(tree) => {
                if !top_level {
                    let span = tree.span;
                    self.r.report(
                        Diag::error(
                            "`use` is only allowed at the top level of a module",
                            item.span,
                        )
                        .with_label(span, "move this to the top level of the module"),
                    );
                }
            }
            ItemKind::Const(konst) => self.with_ribs(RibKind::Item, |this| {
                if let Some(ty) = &konst.ty {
                    this.visit_ty(ty);
                }
                this.visit_expr(&konst.expr);
            }),
            ItemKind::Fn(func) => self.resolve_fn(item, func, RibKind::Item),
            ItemKind::Impl(imp) => {
                if !top_level {
                    self.r.report(
                        Diag::error(
                            "`impl` blocks are only allowed at the top level of a module",
                            item.span,
                        )
                        .with_label(
                            imp.self_ty.span,
                            "move this `impl` to the top level of the module",
                        ),
                    );
                }
                self.resolve_impl(item, imp);
            }
            ItemKind::Struct(_, generics, _) | ItemKind::Enum(_, generics, _) => {
                self.with_ribs(RibKind::Item, |this| {
                    this.declare_generics(item, generics);
                    visit::walk_item(this, item);
                });
            }
        }
        self.top_level = top_level;
    }

    fn visit_block(&mut self, block: &'ast Block) {
        self.with_ribs(RibKind::Normal, |this| {
            this.declare_block_items(block);
            visit::walk_block(this, block);
        });
    }

    fn visit_local(&mut self, local: &'ast Local) {
        if let Some(ty) = &local.ty {
            self.visit_ty(ty);
        }
        if let Some(init) = &local.init {
            self.visit_expr(init);
        }
        self.resolve_pattern(&local.pat);

        // `let io = std::io;` or `let m = std::import("m.xur");` makes the
        // local usable as a path prefix.
        if let PatKind::Ident(ident) = local.pat.kind
            && let Some(init) = &local.init
            && let Some(module) = self.static_module(init)
            && let Some(binding) = self
                .value_ribs
                .last_mut()
                .and_then(|rib| rib.bindings.get_mut(&ident.name))
        {
            binding.module = Some(module);
        }
    }

    fn visit_expr(&mut self, expr: &'ast Expr) {
        match &expr.kind {
            ExprKind::Path(path) => {
                self.resolve_path(path, Namespace::Value);
                visit::walk_path(self, path);
            }
            ExprKind::ForLoop { pat, iter, body } => {
                self.visit_expr(iter);
                self.with_ribs(RibKind::Normal, |this| {
                    this.resolve_pattern(pat);
                    this.visit_block(body);
                });
            }
            ExprKind::Closure(closure) => self.with_ribs(RibKind::Normal, |this| {
                for param in &closure.decl.inputs {
                    this.visit_ty(&param.ty);
                    this.resolve_pattern(&param.pat);
                }
                if let FnRetTy::Ty(ty) = &closure.decl.output {
                    this.visit_ty(ty);
                }
                this.visit_expr(&closure.body);
            }),
            _ => visit::walk_expr(self, expr),
        }
    }

    fn visit_arm(&mut self, arm: &'ast Arm) {
        self.with_ribs(RibKind::Normal, |this| {
            this.resolve_pattern(&arm.pat);
            this.visit_expr(&arm.body);
        });
    }

    fn visit_pat(&mut self, pat: &'ast Pat) {
        match &pat.kind {
            PatKind::Ident(ident) => {
                // As in rustc, a lone identifier that names a unit variant,
                // unit struct or constant matches it; anything else binds.
                let existing = self.resolve_ident(Namespace::Value, ident.name);
                match existing.map(|binding| binding.res) {
                    Some(
                        res @ Res::Def(DefKind::Variant | DefKind::Struct | DefKind::Const, _),
                    ) => {
                        self.r.res.insert(pat.id, res);
                    }
                    _ => self.bind_local(*ident, pat.id),
                }
            }
            PatKind::Path(path) | PatKind::TupleStruct(path, _) => {
                self.resolve_path(path, Namespace::Value);
                visit::walk_pat(self, pat);
            }
            _ => visit::walk_pat(self, pat),
        }
    }

    fn visit_ty(&mut self, ty: &'ast Ty) {
        if let TyKind::Path(path) = &ty.kind {
            self.resolve_path(path, Namespace::Type);
        }
        visit::walk_ty(self, ty);
    }
}

/// "cannot find value `x` in this scope"
fn cannot_find(ns: Namespace, ident: Ident) -> Diag {
    Diag::error(
        format!("cannot find {} `{}` in this scope", ns.descr(), ident.name),
        ident.span,
    )
    .with_label(ident.span, "not found in this scope")
}
