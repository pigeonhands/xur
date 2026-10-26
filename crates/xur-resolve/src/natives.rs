//! Declaring the modules implemented in Rust, such as `std`.

use std::collections::HashMap;

use xur_span::Symbol;

use crate::create_scoped_def;
use crate::def::{DefData, DefId, DefKind, Definitions, Namespace, Res};
use crate::native::{NativeFnDecl, NativeModule};
use crate::scope::{Binding, Scope, Vis};

/// Creates definitions and scopes for `modules` and everything in them,
/// returning each top-level module by name.
pub(crate) fn declare(
    defs: &mut Definitions,
    scopes: &mut HashMap<DefId, Scope>,
    modules: &[NativeModule],
) -> HashMap<Symbol, DefId> {
    let mut declarer = Declarer { defs, scopes };
    modules
        .iter()
        .map(|module| (Symbol::intern(&module.name), declarer.module(module, None)))
        .collect()
}

struct Declarer<'a> {
    defs: &'a mut Definitions,
    scopes: &'a mut HashMap<DefId, Scope>,
}

impl Declarer<'_> {
    fn module(&mut self, module: &NativeModule, parent: Option<DefId>) -> DefId {
        let def = self.scoped(DefKind::Mod, &module.name, parent, None);
        self.functions(def, &module.functions);
        for ty in &module.types {
            let kind = DefKind::NativeType(ty.id);
            let ty_def = self.scoped(kind, &ty.name, Some(def), Some(def));
            self.functions(ty_def, &ty.functions);
            self.bind(def, Namespace::Type, &ty.name, Res::Def(kind, ty_def));
        }
        for child in &module.modules {
            let child_def = self.module(child, Some(def));
            self.bind(
                def,
                Namespace::Type,
                &child.name,
                Res::Def(DefKind::Mod, child_def),
            );
        }
        def
    }

    fn functions(&mut self, scope: DefId, functions: &[NativeFnDecl]) {
        for function in functions {
            let kind = DefKind::NativeFn(function.id);
            let def = self.defs.create(DefData {
                kind,
                name: Symbol::intern(&function.name),
                parent: Some(scope),
                span: None,
            });
            self.bind(scope, Namespace::Value, &function.name, Res::Def(kind, def));
        }
    }

    fn scoped(
        &mut self,
        kind: DefKind,
        name: &str,
        parent: Option<DefId>,
        module: Option<DefId>,
    ) -> DefId {
        let data = DefData {
            kind,
            name: Symbol::intern(name),
            parent,
            span: None,
        };
        create_scoped_def(self.defs, self.scopes, data, module)
    }

    fn bind(&mut self, scope: DefId, ns: Namespace, name: &str, res: Res) {
        let binding = Binding {
            res,
            vis: Vis::Public,
            span: None,
        };
        self.scopes
            .get_mut(&scope)
            .expect("native modules have scopes")
            .insert(ns, Symbol::intern(name), binding)
            .unwrap_or_else(|_| panic!("native name `{name}` is registered twice"));
    }
}
