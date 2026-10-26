//! Looking names up in scopes, and walking `a::b::c` paths.

use xur_ast::PathSegment;
use xur_errors::Diag;
use xur_span::{Ident, Symbol};

use crate::def::{DefId, DefKind, Namespace, PrimTy, Res};
use crate::scope::{Binding, Vis};
use crate::{Resolver, format_def_path};

/// The result of a lookup that can fail. `Err(None)` means the failure was
/// already reported, e.g. the path started with a name that failed to
/// resolve, so nothing more should be said about it.
pub(crate) type Lookup<T> = Result<T, Option<Diag>>;

impl Resolver<'_> {
    /// Looks `name` up as written at the top level of `module`: the module's
    /// own items, then the prelude, then built-in names (`std` and the
    /// primitive types).
    pub(crate) fn lookup_in_module(
        &self,
        module: DefId,
        ns: Namespace,
        name: Symbol,
    ) -> Option<Binding> {
        self.lookup_ns_in_module(module, ns, name).or_else(|| {
            self.module_as_value(ns, || {
                self.lookup_ns_in_module(module, Namespace::Type, name)
            })
        })
    }

    /// Looks up the first segment of a multi-segment path at the top level
    /// of `module`. It should name a module or enum, but anything found is
    /// returned so that a misuse can be reported precisely.
    pub(crate) fn lookup_path_start_in_module(
        &self,
        module: DefId,
        name: Symbol,
    ) -> Option<Binding> {
        self.lookup_in_module(module, Namespace::Type, name)
            .or_else(|| self.lookup_in_module(module, Namespace::Value, name))
    }

    fn lookup_ns_in_module(&self, module: DefId, ns: Namespace, name: Symbol) -> Option<Binding> {
        if let Some(binding) = self.scope(module).get(ns, name) {
            return Some(binding);
        }
        let prelude = self.prelude();
        if module != prelude
            && let Some(binding) = self.scope(prelude).get(ns, name)
            && binding.vis == Vis::Public
        {
            return Some(binding);
        }
        if ns != Namespace::Type {
            return None;
        }
        let builtin = |res| Binding {
            res,
            vis: Vis::Public,
            span: None,
        };
        if name.as_str() == "std" {
            return Some(builtin(Res::Def(DefKind::Mod, self.std)));
        }
        PrimTy::from_name(name).map(|ty| builtin(Res::PrimTy(ty)))
    }

    /// Looks `name` up directly in `scope`, a module or enum.
    pub(crate) fn lookup_in_scope(
        &self,
        scope: DefId,
        ns: Namespace,
        name: Symbol,
    ) -> Option<Binding> {
        let scope = self.scope(scope);
        scope
            .get(ns, name)
            .or_else(|| self.module_as_value(ns, || scope.get(Namespace::Type, name)))
    }

    /// Modules are values: a value lookup that finds nothing may find a
    /// module in the type namespace instead.
    fn module_as_value(
        &self,
        ns: Namespace,
        lookup_type: impl FnOnce() -> Option<Binding>,
    ) -> Option<Binding> {
        if ns != Namespace::Value {
            return None;
        }
        lookup_type().filter(|binding| matches!(binding.res, Res::Def(DefKind::Mod, _)))
    }

    /// Walks the module path `rest` from `start`, what the path's first
    /// segment `first` resolved to, and returns the scope to look the
    /// path's last segment up in. Privacy errors along the way are added to
    /// `privacy` rather than reported, for the caller to report on success.
    pub(crate) fn resolve_prefix(
        &self,
        start: Res,
        first: Ident,
        rest: &[PathSegment],
        from: DefId,
        privacy: &mut Vec<Diag>,
    ) -> Lookup<DefId> {
        let mut scope = expect_scope(start, first)?;
        for segment in rest {
            let ident = segment.ident;
            let binding = self
                .lookup_in_scope(scope, Namespace::Type, ident.name)
                .ok_or_else(|| Some(self.not_found_in(scope, Namespace::Type, ident)))?;
            privacy.extend(self.privacy_error(scope, binding, ident, from));
            scope = expect_scope(binding.res, ident)?;
        }
        Ok(scope)
    }

    /// An error if `binding`, found in `scope`, may not be used from module
    /// `from`.
    pub(crate) fn privacy_error(
        &self,
        scope: DefId,
        binding: Binding,
        ident: Ident,
        from: DefId,
    ) -> Option<Diag> {
        if binding.vis == Vis::Public || self.scope(scope).module == from {
            return None;
        }
        let descr = binding.res.descr();
        let mut diag = Diag::error(format!("{descr} `{}` is private", ident.name), ident.span)
            .with_label(ident.span, format!("private {descr}"));
        if let Some(span) = binding.span {
            diag = diag.with_label(
                span,
                format!("`{}` is declared here without `pub`", ident.name),
            );
        }
        Some(diag)
    }

    /// "cannot find value `x` in module `m`"
    pub(crate) fn not_found_in(&self, scope: DefId, ns: Namespace, ident: Ident) -> Diag {
        let scope_descr = self.defs.get(scope).kind.descr();
        let scope_path = self.def_path(scope);
        Diag::error(
            format!(
                "cannot find {} `{}` in {scope_descr} `{scope_path}`",
                ns.descr(),
                ident.name
            ),
            ident.span,
        )
        .with_label(ident.span, format!("not found in `{scope_path}`"))
    }

    pub(crate) fn def_path(&self, def: DefId) -> String {
        let unnamed = [self.module_def(self.krate.root), self.prelude()];
        format_def_path(&self.defs, &unnamed, def)
    }
}

/// "failed to resolve: use of undeclared module `x`"
pub(crate) fn undeclared_module(ident: Ident) -> Diag {
    let message = format!("use of undeclared module `{}`", ident.name);
    Diag::error(format!("failed to resolve: {message}"), ident.span).with_label(ident.span, message)
}

/// The scope that `res` names, for looking further path segments up in.
fn expect_scope(res: Res, ident: Ident) -> Lookup<DefId> {
    match res {
        Res::Err => Err(None),
        _ => res.scope().ok_or_else(|| {
            let message = format!("`{}` is a {}, not a module", ident.name, res.descr());
            Some(
                Diag::error(format!("failed to resolve: {message}"), ident.span)
                    .with_label(ident.span, message),
            )
        }),
    }
}
