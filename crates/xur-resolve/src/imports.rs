//! Resolving `use` declarations.
//!
//! An import may depend on another (`use a::b;` where `a` is itself
//! imported), so imports are retried until a pass makes no progress, and
//! only then are the remaining failures reported.

use xur_ast::UseTree;
use xur_errors::Diag;

use crate::Resolver;
use crate::def::{DefId, Namespace, Res};
use crate::ident::{Lookup, undeclared_module};
use crate::scope::{Binding, Vis};

pub(crate) struct Import<'a> {
    /// The module the `use` is in.
    pub module: DefId,
    pub tree: &'a UseTree,
    pub vis: Vis,
}

impl Resolver<'_> {
    pub(crate) fn resolve_imports(&mut self) {
        loop {
            let pending = std::mem::take(&mut self.pending_imports);
            let before = pending.len();
            for import in pending {
                if self.try_resolve_import(&import).is_err() {
                    self.pending_imports.push(import);
                }
            }
            if self.pending_imports.len() == before {
                break;
            }
        }

        for import in std::mem::take(&mut self.pending_imports) {
            if let Err(Some(diag)) = self.try_resolve_import(&import) {
                self.report(diag);
            }
            // Declare the name anyway, unless it is taken, so that its uses
            // aren't reported again.
            let path = &import.tree.path;
            let last = path.segments.last().expect("paths have a segment").ident;
            let binding = Binding {
                res: Res::Err,
                vis: import.vis,
                span: Some(last.span),
            };
            let scope = self
                .scopes
                .get_mut(&import.module)
                .expect("modules have scopes");
            for ns in [Namespace::Type, Namespace::Value] {
                let _already_declared = scope.insert(ns, last.name, binding);
            }
            self.res.insert(path.id, Res::Err);
        }
    }

    /// Resolves `import` and declares the name it introduces, in every
    /// namespace the name is found in. On failure nothing is changed.
    fn try_resolve_import(&mut self, import: &Import<'_>) -> Lookup<()> {
        let path = &import.tree.path;
        let (last, prefix) = path.segments.split_last().expect("paths have a segment");
        let mut privacy = Vec::new();
        let found: Vec<(Namespace, Binding)> = if let [first, rest @ ..] = prefix {
            let start = self
                .lookup_path_start_in_module(import.module, first.ident.name)
                .ok_or_else(|| Some(undeclared_module(first.ident)))?;
            let scope =
                self.resolve_prefix(start.res, first.ident, rest, import.module, &mut privacy)?;
            let found: Vec<_> = namespaces(|ns| self.lookup_in_scope(scope, ns, last.ident.name));
            if let Some(&(_, binding)) = found.first() {
                privacy.extend(self.privacy_error(scope, binding, last.ident, import.module));
            } else {
                let scope_path = self.def_path(scope);
                return Err(Some(unresolved_import(
                    &import.tree.path,
                    last.ident,
                    format!("no `{}` in `{scope_path}`", last.ident.name),
                )));
            }
            found
        } else {
            let found = namespaces(|ns| self.lookup_in_module(import.module, ns, last.ident.name));
            if found.is_empty() {
                let label = format!("no `{}` in scope", last.ident.name);
                return Err(Some(unresolved_import(
                    &import.tree.path,
                    last.ident,
                    label,
                )));
            }
            found
        };

        self.diags.extend(privacy);
        let namespaces: Vec<_> = found.iter().map(|&(ns, _)| ns).collect();
        let binding = Binding {
            res: found[0].1.res,
            vis: import.vis,
            span: Some(last.ident.span),
        };
        self.declare(import.module, &namespaces, last.ident, binding);
        self.res.insert(path.id, found[0].1.res);
        Ok(())
    }
}

/// The namespaces in which `lookup` finds a binding, type first.
fn namespaces(mut lookup: impl FnMut(Namespace) -> Option<Binding>) -> Vec<(Namespace, Binding)> {
    [Namespace::Type, Namespace::Value]
        .into_iter()
        .filter_map(|ns| lookup(ns).map(|binding| (ns, binding)))
        .collect()
}

fn unresolved_import(path: &xur_ast::Path, last: xur_span::Ident, label: String) -> Diag {
    let names: Vec<_> = path
        .segments
        .iter()
        .map(|s| s.ident.name.as_str())
        .collect();
    Diag::error(
        format!("unresolved import `{}`", names.join("::")),
        last.span,
    )
    .with_label(last.span, label)
}
