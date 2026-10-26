//! Loading modules and resolving names, after `rustc_resolve`.
//!
//! Resolution runs in phases:
//! 1. [`load_crate`] parses every module reachable through `std::import`.
//! 2. Every module's items are declared in its [`Scope`](scope::Scope).
//! 3. `use` declarations are resolved, repeatedly until no more make
//!    progress, since one import may depend on another.
//!
//! Names only ever resolve statically. Members reached through a value
//! (`math.add`, `point.x`) are not names and are left for later stages.

mod collect;
mod def;
mod ident;
mod imports;
mod loader;
mod scope;
mod std_lib;

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use xur_ast::NodeId;
use xur_errors::Diag;

pub use def::{Builtin, DefData, DefId, DefKind, Definitions, Namespace, PrimTy, Res};
pub use loader::{Crate, LoadedModule, ModuleIdx, load_crate};

use imports::Import;
use scope::Scope;

/// The results of name resolution, keyed by AST node.
#[derive(Debug)]
pub struct Resolutions {
    pub defs: Definitions,
    /// What each resolved path (and, later, binding pattern) refers to.
    pub res: HashMap<NodeId, Res>,
    /// The definition created for each item, variant and generic parameter.
    pub node_defs: HashMap<NodeId, DefId>,
    /// The module definition of each loaded file, by [`ModuleIdx`].
    pub module_defs: Vec<DefId>,
    /// File modules whose name is left out of [`Resolutions::def_path`]: the
    /// root module and the prelude.
    unnamed_modules: [DefId; 2],
}

impl Resolutions {
    /// The path a definition is known by, e.g. `std::io::println`,
    /// `math::div` or `Result::Ok`. Items of the root module and the prelude
    /// are written without a module prefix.
    pub fn def_path(&self, id: DefId) -> String {
        format_def_path(&self.defs, self.unnamed_modules, id)
    }
}

/// Resolves every name in `krate`.
pub fn resolve(krate: &Crate) -> (Resolutions, Vec<Diag>) {
    let mut resolver = Resolver::new(krate);
    resolver.collect_items();
    resolver.resolve_imports();
    resolver.finish()
}

struct Resolver<'a> {
    krate: &'a Crate,
    defs: Definitions,
    /// The names declared in each module and enum.
    scopes: HashMap<DefId, Scope>,
    res: HashMap<NodeId, Res>,
    node_defs: HashMap<NodeId, DefId>,
    module_defs: Vec<DefId>,
    std: DefId,
    /// `use` declarations not yet resolved.
    pending_imports: Vec<Import<'a>>,
    diags: Vec<Diag>,
}

impl<'a> Resolver<'a> {
    fn new(krate: &'a Crate) -> Self {
        let mut defs = Definitions::default();
        let mut scopes = HashMap::new();
        let std = std_lib::build(&mut defs, &mut scopes);
        let module_defs = krate
            .modules
            .iter()
            .map(|module| {
                let data = DefData {
                    kind: DefKind::Mod,
                    name: module.name,
                    parent: None,
                    span: Some(module.ast.span),
                };
                create_scoped_def(&mut defs, &mut scopes, data, None)
            })
            .collect();
        Self {
            krate,
            defs,
            scopes,
            res: HashMap::new(),
            node_defs: HashMap::new(),
            module_defs,
            std,
            pending_imports: Vec::new(),
            diags: Vec::new(),
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

    fn finish(self) -> (Resolutions, Vec<Diag>) {
        let unnamed_modules = [
            self.module_def(self.krate.root),
            self.module_def(self.krate.prelude),
        ];
        let resolutions = Resolutions {
            defs: self.defs,
            res: self.res,
            node_defs: self.node_defs,
            module_defs: self.module_defs,
            unnamed_modules,
        };
        (resolutions, self.diags)
    }
}

/// Joins the names of `id` and its parents with `::`, leaving out the name
/// of an `unnamed` outermost module unless it is `id` itself.
fn format_def_path(defs: &Definitions, unnamed: [DefId; 2], id: DefId) -> String {
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
