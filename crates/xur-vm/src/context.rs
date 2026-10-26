//! What native functions can ask of the interpreter.

use std::path::Path;
use std::sync::{Arc, PoisonError};
use std::thread::{self, JoinHandle};

use xur_resolve::LangItems;
use xur_runtime::convert::ConvertCx;
use xur_runtime::{Context, Registry, RunResult, RuntimeError, Value, lock};
use xur_span::Span;

use crate::interp::Interp;

impl ConvertCx for Interp<'_> {
    fn lang(&self) -> &LangItems {
        &self.shared.lang
    }

    fn registry(&self) -> &Registry {
        &self.shared.registry
    }
}

impl Context for Interp<'_> {
    /// Loads a module at runtime. A relative path is relative to the file
    /// of the module whose code is running.
    fn import(&mut self, path: &str) -> RunResult<Value> {
        let path = {
            let program = self.shared.program();
            let dir = self
                .module
                .and_then(|module| program.krate.module(module).path.parent())
                .unwrap_or(Path::new(""))
                .to_owned();
            dir.join(path)
        };
        let (module, diags) = self.shared.program_mut().import(&path).map_err(|error| {
            RuntimeError::new(format!("couldn't import `{}`: {error}", path.display()))
        })?;
        if !diags.is_empty() {
            lock(&self.shared.diags).extend(diags);
            return Err(RuntimeError::new(format!(
                "couldn't import `{}` because it has errors",
                path.display()
            )));
        }
        let program = self.shared.program();
        self.shared
            .items
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .update(&program);
        Ok(Value::Module(program.resolutions().module_defs[module.0]))
    }

    fn display(&self, value: &Value) -> String {
        self.display_value(value)
    }

    fn type_args(&self) -> &[String] {
        &self.native_type_args
    }

    fn call(&mut self, callee: &Value, args: Vec<Value>) -> RunResult<Value> {
        self.call_value(callee, args, Vec::new(), Span::default())
    }

    /// Runs `f` on a new thread, with its own interpreter over the shared
    /// program.
    fn spawn(&mut self, f: Value) -> RunResult<JoinHandle<RunResult<Value>>> {
        let shared = Arc::clone(self.shared);
        let module = self.module;
        thread::Builder::new()
            .stack_size(crate::THREAD_STACK_SIZE)
            .spawn(move || {
                let mut interp = Interp::new(&shared);
                interp.module = module;
                interp.call_value(&f, Vec::new(), Vec::new(), Span::default())
            })
            .map_err(|error| RuntimeError::new(format!("couldn't spawn a thread: {error}")))
    }

    fn print(&mut self, text: &str) -> RunResult<()> {
        lock(&self.shared.stdout)
            .write_all(text.as_bytes())
            .map_err(|error| RuntimeError::new(format!("couldn't write to stdout: {error}")))
    }
}
