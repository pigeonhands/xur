//! Declaring the modules implemented in Rust, such as `std`.

use std::collections::HashMap;

use xur_span::Symbol;

use crate::create_scoped_def;
use crate::def::{DefData, DefId, DefKind, Definitions, Namespace, Res};
use crate::native::{NativeFnDecl, NativeModule};
use crate::scope::{Binding, Scope, Vis};

/// Creates definitions and scopes for `modules` and everything in them,
/// returning each top-level module by name. Lazy modules are declared empty
/// and recorded in `lazy` with their paths, for [`populate`].
pub(crate) fn declare(
    defs: &mut Definitions,
    scopes: &mut HashMap<DefId, Scope>,
    lazy: &mut HashMap<DefId, Vec<String>>,
    modules: &[NativeModule],
) -> HashMap<Symbol, DefId> {
    let mut declarer = Declarer { defs, scopes, lazy };
    modules
        .iter()
        .map(|module| {
            let path = vec![module.name.clone()];
            let def = declarer.scoped(DefKind::Mod, &module.name, None, None);
            declarer.contents(def, module, path);
            (Symbol::intern(&module.name), def)
        })
        .collect()
}

/// Declares the contents of the lazy module `def`, at `path`, now that they
/// are known.
pub(crate) fn populate(
    defs: &mut Definitions,
    scopes: &mut HashMap<DefId, Scope>,
    lazy: &mut HashMap<DefId, Vec<String>>,
    def: DefId,
    path: Vec<String>,
    contents: &NativeModule,
) {
    Declarer { defs, scopes, lazy }.contents(def, contents, path);
}

struct Declarer<'a> {
    defs: &'a mut Definitions,
    scopes: &'a mut HashMap<DefId, Scope>,
    lazy: &'a mut HashMap<DefId, Vec<String>>,
}

impl Declarer<'_> {
    /// Declares the contents of `module`, whose definition is `def`, or
    /// records it as lazy.
    fn contents(&mut self, def: DefId, module: &NativeModule, path: Vec<String>) {
        if module.lazy {
            self.lazy.insert(def, path);
            return;
        }
        self.functions(def, &module.functions);
        for ty in &module.types {
            let kind = DefKind::NativeType(ty.id);
            let ty_def = self.scoped(kind, &ty.name, Some(def), Some(def));
            self.functions(ty_def, &ty.functions);
            self.bind(def, Namespace::Type, &ty.name, Res::Def(kind, ty_def));
        }
        for child in &module.modules {
            let child_def = self.scoped(DefKind::Mod, &child.name, Some(def), None);
            let mut child_path = path.clone();
            child_path.push(child.name.clone());
            self.contents(child_def, child, child_path);
            self.bind(
                def,
                Namespace::Type,
                &child.name,
                Res::Def(DefKind::Mod, child_def),
            );
        }
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
