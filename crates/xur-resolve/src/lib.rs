//! Loading modules and resolving names, after `rustc_resolve`.
//!
//! A [`Program`] is loaded from its root file and can grow at runtime as
//! further modules are imported. Each batch of new modules is resolved in
//! phases:
//! 1. The modules reachable through literal-path `std::import`s are parsed.
//! 2. Every module's items are declared in its [`Scope`](scope::Scope).
//! 3. `use` declarations are resolved, repeatedly until no more make
//!    progress, since one import may depend on another.
//! 4. The functions of `impl` blocks are declared in their type's scope.
//! 5. The names used inside items (bodies, types, patterns) are resolved
//!    against local scopes and then the module.
//!
//! Names only ever resolve statically. Members reached through a value
//! (`math.add`, `point.x`) are not names and are left for later stages.

mod collect;
mod def;
mod ident;
mod imports;
mod lang;
mod late;
mod loader;
mod names;
mod native;
mod natives;
mod program;
mod scope;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::ops::Range;

use xur_ast::NodeId;
use xur_errors::Diag;
use xur_span::{Ident, Symbol};

pub use def::{DefData, DefId, DefKind, Definitions, Namespace, PrimTy, Res};
pub use lang::LangItems;
pub use loader::{Crate, LoadedModule, ModuleIdx};
pub use names::{NameRef, names};
pub use native::{
    NativeFnDecl, NativeFnId, NativeModule, NativeProvider, NativeTypeDecl, NativeTypeId,
};
pub use program::Program;

use imports::Import;
use scope::Scope;

/// The results of name resolution, keyed by AST node.
#[derive(Debug)]
pub struct Resolutions {
    pub defs: Definitions,
    /// What each path and each identifier pattern refers to. A pattern that
    /// binds a variable resolves to [`Res::Local`] of itself.
    pub res: HashMap<NodeId, Res>,
    /// The name of each local variable, by the pattern that binds it.
    pub locals: HashMap<NodeId, Ident>,
    /// The definition created for each item, variant and generic parameter.
    pub node_defs: HashMap<NodeId, DefId>,
    /// The module definition of each loaded file, by [`ModuleIdx`].
    pub module_defs: Vec<DefId>,
    /// File modules whose name is left out of [`Resolutions::def_path`]: the
    /// root module and the prelude.
    unnamed_modules: Vec<DefId>,
    /// The names declared in each module and enum.
    scopes: HashMap<DefId, Scope>,
    /// The top-level native modules, such as `std`, by name.
    native_roots: HashMap<Symbol, DefId>,
    /// Lazy native modules not yet expanded, with their paths.
    lazy_natives: HashMap<DefId, Vec<String>>,
}

impl Resolutions {
    /// Resolutions with nothing but the native modules `natives`.
    fn new(natives: &[NativeModule]) -> Self {
        let mut defs = Definitions::default();
        let mut scopes = HashMap::new();
        let mut lazy_natives = HashMap::new();
        let native_roots = natives::declare(&mut defs, &mut scopes, &mut lazy_natives, natives);
        Self {
            defs,
            res: HashMap::new(),
            locals: HashMap::new(),
            node_defs: HashMap::new(),
            module_defs: Vec::new(),
            unnamed_modules: Vec::new(),
            scopes,
            native_roots,
            lazy_natives,
        }
    }

    /// The path a definition is known by, e.g. `std::io::println`,
    /// `math::div` or `Result::Ok`. Items of the root module and the prelude
    /// are written without a module prefix.
    pub fn def_path(&self, id: DefId) -> String {
        format_def_path(&self.defs, &self.unnamed_modules, id)
    }

    /// The binding of `name` in namespace `ns` declared directly in the
    /// module or type `scope`.
    pub fn lookup(&self, scope: DefId, ns: Namespace, name: Symbol) -> Option<Res> {
        Some(self.scopes.get(&scope)?.get(ns, name)?.res)
    }

    /// The function `name` of the struct, enum or native type `ty`: from an
    /// `impl` block, or registered with a native type. For dispatching
    /// `value.name()` at runtime.
    pub fn assoc_fn(&self, ty: DefId, name: Symbol) -> Option<Res> {
        match self.scopes.get(&ty)?.get(Namespace::Value, name)?.res {
            res @ Res::Def(DefKind::Fn | DefKind::NativeFn(_), _) => Some(res),
            _ => None,
        }
    }
}

/// Resolves the modules `new` of `krate`, which were added since
/// `resolutions` was last updated.
fn resolve_modules(
    krate: &Crate,
    natives: &dyn NativeProvider,
    resolutions: Resolutions,
    new: Range<usize>,
) -> (Resolutions, Vec<Diag>) {
    let mut resolver = Resolver::resume(krate, natives, resolutions);
    let new: Vec<_> = new.map(ModuleIdx).collect();
    resolver.declare_modules(&new);
    resolver.collect_items(&new);
    resolver.resolve_imports();
    resolver.collect_impls(&new);
    resolver.resolve_bodies(&new);
    resolver.suspend()
}

struct Resolver<'a> {
    krate: &'a Crate,
    natives: &'a dyn NativeProvider,
    defs: Definitions,
    /// The names declared in each module and enum.
    scopes: HashMap<DefId, Scope>,
    res: HashMap<NodeId, Res>,
    locals: HashMap<NodeId, Ident>,
    node_defs: HashMap<NodeId, DefId>,
    module_defs: Vec<DefId>,
    native_roots: HashMap<Symbol, DefId>,
    lazy_natives: HashMap<DefId, Vec<String>>,
    /// `use` declarations not yet resolved.
    pending_imports: Vec<Import<'a>>,
    /// What the type of each `impl` block resolved to, by the impl item.
    impl_self_tys: HashMap<NodeId, Res>,
    diags: Vec<Diag>,
}

impl<'a> Resolver<'a> {
    /// Picks up resolution where `resolutions` left off.
    fn resume(krate: &'a Crate, natives: &'a dyn NativeProvider, resolutions: Resolutions) -> Self {
        let Resolutions {
            defs,
            res,
            locals,
            node_defs,
            module_defs,
            unnamed_modules: _,
            scopes,
            native_roots,
            lazy_natives,
        } = resolutions;
        Self {
            krate,
            natives,
            defs,
            scopes,
            res,
            locals,
            node_defs,
            module_defs,
            native_roots,
            lazy_natives,
            pending_imports: Vec::new(),
            impl_self_tys: HashMap::new(),
            diags: Vec::new(),
        }
    }

    /// Creates the module definitions of the newly loaded modules `new`.
    fn declare_modules(&mut self, new: &[ModuleIdx]) {
        for &idx in new {
            debug_assert_eq!(
                idx.0,
                self.module_defs.len(),
                "modules are resolved in order"
            );
            let module = self.krate.module(idx);
            let data = DefData {
                kind: DefKind::Mod,
                name: module.name,
                parent: None,
                span: Some(module.ast.span),
            };
            let def = create_scoped_def(&mut self.defs, &mut self.scopes, data, None);
            self.module_defs.push(def);
        }
    }

    fn module_def(&self, idx: ModuleIdx) -> DefId {
        self.module_defs[idx.0]
    }

    fn prelude(&self) -> DefId {
        self.module_def(self.krate.prelude)
    }

    fn report(&mut self, diag: Diag) {
        self.diags.push(diag);
    }

    fn scope(&self, def: DefId) -> &Scope {
        &self.scopes[&def]
    }

    /// Declares the contents of `scope` if it is a lazy native module not
    /// yet expanded. Called before looking names up inside a scope.
    fn expand_lazy(&mut self, scope: DefId) {
        if let Some(path) = self.lazy_natives.remove(&scope) {
            let contents = self.natives.expand(&path);
            natives::populate(
                &mut self.defs,
                &mut self.scopes,
                &mut self.lazy_natives,
                scope,
                path,
                &contents,
            );
        }
    }

    /// Hands the accumulated state back, with the diagnostics of this batch.
    fn suspend(self) -> (Resolutions, Vec<Diag>) {
        let unnamed_modules = [self.krate.root, self.krate.prelude]
            .into_iter()
            .filter_map(|idx| self.module_defs.get(idx.0).copied())
            .collect();
        let resolutions = Resolutions {
            defs: self.defs,
            res: self.res,
            locals: self.locals,
            node_defs: self.node_defs,
            module_defs: self.module_defs,
            unnamed_modules,
            scopes: self.scopes,
            native_roots: self.native_roots,
            lazy_natives: self.lazy_natives,
        };
        (resolutions, self.diags)
    }
}

/// Joins the names of `id` and its parents with `::`, leaving out the name
/// of an `unnamed` outermost module unless it is `id` itself.
fn format_def_path(defs: &Definitions, unnamed: &[DefId], id: DefId) -> String {
    let lineage = defs.lineage(id);
    let skip = usize::from(lineage.len() > 1 && unnamed.contains(&lineage[0]));
    let names: Vec<_> = lineage[skip..]
        .iter()
        .map(|&id| defs.get(id).name.as_str())
        .collect();
    names.join("::")
}

/// Creates a module or enum definition along with its (empty) scope.
/// `module` is the module an enum is declared in; `None` for a module
/// itself.
fn create_scoped_def(
    defs: &mut Definitions,
    scopes: &mut HashMap<DefId, Scope>,
    data: DefData,
    module: Option<DefId>,
) -> DefId {
    let def = defs.create(data);
    scopes.insert(def, Scope::new(module.unwrap_or(def)));
    def
}
