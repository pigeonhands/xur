//! The built-in `std` module tree.

use std::collections::HashMap;

use xur_span::Symbol;

use crate::create_scoped_def;
use crate::def::{Builtin, DefData, DefId, DefKind, Definitions, Namespace, Res};
use crate::scope::{Binding, Scope, Vis};

/// Every built-in function, by its path within `std`.
const ITEMS: &[(&str, Builtin)] = &[
    ("import", Builtin::Import),
    ("io::print", Builtin::Print),
    ("io::println", Builtin::Println),
    ("any::type_name", Builtin::TypeName),
];

/// Creates `std` and its submodules, returning the `std` module.
pub(crate) fn build(defs: &mut Definitions, scopes: &mut HashMap<DefId, Scope>) -> DefId {
    let std = create_module(defs, scopes, Symbol::intern("std"), None);
    for &(path, builtin) in ITEMS {
        let mut names: Vec<_> = path.split("::").map(Symbol::intern).collect();
        let name = names.pop().expect("builtin paths are non-empty");
        let module = names
            .into_iter()
            .fold(std, |parent, name| submodule(defs, scopes, parent, name));

        let kind = DefKind::Builtin(builtin);
        let def = defs.create(DefData {
            kind,
            name,
            parent: Some(module),
            span: None,
        });
        declare(scopes, module, Namespace::Value, name, Res::Def(kind, def));
    }
    std
}

/// The submodule `name` of `parent`, created on first use.
fn submodule(
    defs: &mut Definitions,
    scopes: &mut HashMap<DefId, Scope>,
    parent: DefId,
    name: Symbol,
) -> DefId {
    if let Some(existing) = scopes[&parent].get(Namespace::Type, name) {
        return existing
            .res
            .scope()
            .expect("std contains only modules and functions");
    }
    let module = create_module(defs, scopes, name, Some(parent));
    declare(
        scopes,
        parent,
        Namespace::Type,
        name,
        Res::Def(DefKind::Mod, module),
    );
    module
}

fn create_module(
    defs: &mut Definitions,
    scopes: &mut HashMap<DefId, Scope>,
    name: Symbol,
    parent: Option<DefId>,
) -> DefId {
    let data = DefData {
        kind: DefKind::Mod,
        name,
        parent,
        span: None,
    };
    create_scoped_def(defs, scopes, data, None)
}

fn declare(
    scopes: &mut HashMap<DefId, Scope>,
    scope: DefId,
    ns: Namespace,
    name: Symbol,
    res: Res,
) {
    let binding = Binding {
        res,
        vis: Vis::Public,
        span: None,
    };
    scopes
        .get_mut(&scope)
        .expect("std modules have scopes")
        .insert(ns, name, binding)
        .expect("std names are unique");
}
