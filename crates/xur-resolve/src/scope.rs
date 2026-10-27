//! The names declared in a module or enum.

use std::collections::HashMap;

use xur_span::{Span, Symbol};

use crate::def::{DefId, Namespace, Res};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vis {
    Public,
    /// Visible only inside the declaring module.
    Private,
}

/// A name declared in a scope.
#[derive(Debug, Clone, Copy)]
pub struct Binding {
    pub res: Res,
    pub vis: Vis,
    /// Where the name was declared; `None` for built-ins.
    pub span: Option<Span>,
}

/// The names declared directly in one module or enum, per namespace.
#[derive(Debug)]
pub struct Scope {
    /// The module this scope belongs to, for privacy: for an enum, the
    /// module the enum is declared in.
    pub module: DefId,
    types: HashMap<Symbol, Binding>,
    values: HashMap<Symbol, Binding>,
}

impl Scope {
    pub fn new(module: DefId) -> Self {
        Self {
            module,
            types: HashMap::new(),
            values: HashMap::new(),
        }
    }

    pub fn get(&self, ns: Namespace, name: Symbol) -> Option<Binding> {
        self.names(ns).get(&name).copied()
    }

    /// Declares `name`, or returns the existing binding if the name is
    /// already taken in `ns`.
    pub fn insert(&mut self, ns: Namespace, name: Symbol, binding: Binding) -> Result<(), Binding> {
        let names = match ns {
            Namespace::Type => &mut self.types,
            Namespace::Value => &mut self.values,
        };
        match names.get(&name) {
            Some(&existing) => Err(existing),
            None => {
                names.insert(name, binding);
                Ok(())
            }
        }
    }

    /// Every name declared in `ns`, in no particular order.
    pub fn iter(&self, ns: Namespace) -> impl Iterator<Item = (Symbol, Binding)> + '_ {
        self.names(ns)
            .iter()
            .map(|(&name, &binding)| (name, binding))
    }

    fn names(&self, ns: Namespace) -> &HashMap<Symbol, Binding> {
        match ns {
            Namespace::Type => &self.types,
            Namespace::Value => &self.values,
        }
    }
}
