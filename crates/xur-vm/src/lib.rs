//! Running Xur programs: an interpreter for the bytecode of
//! [`xur_bytecode`].
//!
//! Functions are lowered to bytecode the first time they are called and
//! cached. Each thread of a program runs its own [`Interp`] over the
//! state in [`Shared`]; the bytecode is the same a JIT would compile.

mod context;
mod dispatch;
mod display;
mod interp;
mod ops;
mod reflect;
mod types;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use xur_bytecode::{Function, ItemIndex};
use xur_errors::Diag;
use xur_resolve::{DefId, DefKind, LangItems, Namespace, Program, Res};
use xur_runtime::{Registry, RunResult, RuntimeError, Value, lock};
use xur_span::Symbol;

use interp::Interp;

/// The stack of threads a program spawns. Deep recursion in Xur recurses
/// in the interpreter too.
pub const THREAD_STACK_SIZE: usize = 64 << 20;

/// A loaded program, ready to run.
pub struct Vm {
    shared: Arc<Shared>,
}

/// What every thread of a running program shares.
struct Shared {
    program: RwLock<Program>,
    items: RwLock<ItemIndex>,
    registry: Arc<Registry>,
    lang: LangItems,
    /// Functions and constants lowered so far.
    code: RwLock<HashMap<DefId, Arc<Function>>>,
    /// The values of constants evaluated so far.
    consts: Mutex<HashMap<DefId, Value>>,
    stdout: Mutex<Box<dyn Write + Send>>,
    /// Diagnostics for modules imported while running.
    diags: Mutex<Vec<Diag>>,
}

impl Vm {
    /// Prepares to run `program`, whose native modules come from `registry`
    /// (the same registry it was loaded with), printing to `stdout`.
    pub fn new(program: Program, registry: Arc<Registry>, stdout: Box<dyn Write + Send>) -> Self {
        let items = ItemIndex::new(&program);
        let lang = program.lang_items();
        let shared = Shared {
            program: RwLock::new(program),
            items: RwLock::new(items),
            registry,
            lang,
            code: RwLock::new(HashMap::new()),
            consts: Mutex::new(HashMap::new()),
            stdout: Mutex::new(stdout),
            diags: Mutex::new(Vec::new()),
        };
        Self {
            shared: Arc::new(shared),
        }
    }

    /// Runs the root module's `main` function.
    pub fn run_main(&self) -> RunResult<Value> {
        let main = {
            let program = self.shared.program();
            let resolutions = program.resolutions();
            let root = resolutions.module_defs[program.krate.root.0];
            match resolutions.lookup(root, Namespace::Value, Symbol::intern("main")) {
                Some(Res::Def(DefKind::Fn, main)) => main,
                _ => return Err(RuntimeError::new("the program has no `main` function")),
            }
        };
        Interp::new(&self.shared).call_fn(main, Vec::new(), Vec::new())
    }

    /// Diagnostics for modules imported while the program ran.
    pub fn take_diagnostics(&self) -> Vec<Diag> {
        std::mem::take(&mut lock(&self.shared.diags))
    }

    /// Reads the program, e.g. to render diagnostics with its source map.
    pub fn program(&self) -> RwLockReadGuard<'_, Program> {
        self.shared.program()
    }
}

impl Shared {
    fn program(&self) -> RwLockReadGuard<'_, Program> {
        self.program.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn program_mut(&self) -> RwLockWriteGuard<'_, Program> {
        self.program.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// The field names of the struct or variant `ctor`, if it has named
    /// fields.
    fn field_names(&self, ctor: DefId) -> Option<Vec<Symbol>> {
        let items = self.items.read().unwrap_or_else(PoisonError::into_inner);
        let fields = items.named_fields(ctor)?;
        Some(
            fields
                .iter()
                .filter_map(|field| field.ident)
                .map(|i| i.name)
                .collect(),
        )
    }

    /// The bytecode of the function or constant `def`, lowered on first use.
    fn code(&self, def: DefId) -> RunResult<Arc<Function>> {
        if let Some(function) = self
            .code
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&def)
        {
            return Ok(Arc::clone(function));
        }
        let function = {
            let program = self.program();
            let items = self.items.read().unwrap_or_else(PoisonError::into_inner);
            xur_bytecode::lower(&program, &items, def)
                .ok_or_else(|| RuntimeError::new("this definition has no code"))?
        };
        let function = Arc::new(function);
        let mut code = self.code.write().unwrap_or_else(PoisonError::into_inner);
        Ok(Arc::clone(code.entry(def).or_insert(function)))
    }
}
