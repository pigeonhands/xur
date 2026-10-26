//! A stand-in for the interpreter, for unit-testing native functions
//! without running a program. Enabled by the `testing` feature.

use std::io::Write;
use std::path::Path;

use facet::Shape;
use xur_resolve::{NativeFnId, NativeModule, NativeTypeId, Program};
use xur_span::MemoryFileLoader;

use crate::convert::ConvertCx;
use crate::registry::Context;
use crate::{LangItems, Registry, RunResult, RuntimeError, Value};

pub struct TestCx<'r> {
    pub registry: &'r Registry,
    pub lang: LangItems,
    pub type_args: Vec<String>,
    pub stdout: Vec<u8>,
}

impl<'r> TestCx<'r> {
    pub fn new(registry: &'r Registry) -> Self {
        let loader = MemoryFileLoader::new([("main.xur", "")]);
        let (program, _) = Program::load(Path::new("main.xur"), Box::new(loader)).unwrap();
        Self {
            registry,
            lang: LangItems::new(&program),
            type_args: Vec::new(),
            stdout: Vec::new(),
        }
    }

    pub fn call(&mut self, id: NativeFnId, args: &[Value]) -> RunResult<Value> {
        let registry = self.registry;
        registry.function(id).call(self, args)
    }

    /// Everything written to standard output so far.
    pub fn output(&self) -> &str {
        std::str::from_utf8(&self.stdout).unwrap()
    }
}

impl ConvertCx for TestCx<'_> {
    fn lang(&self) -> &LangItems {
        &self.lang
    }

    fn native_type(&self, shape: &'static Shape) -> Option<NativeTypeId> {
        self.registry.type_of_shape(shape)
    }
}

impl Context for TestCx<'_> {
    fn import(&mut self, path: &str) -> RunResult<Value> {
        Err(RuntimeError::new(format!(
            "cannot import `{path}` in tests"
        )))
    }

    fn display(&self, value: &Value) -> String {
        match value {
            Value::Str(s) => s.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => f.to_string(),
            other => format!("{other:?}"),
        }
    }

    fn type_args(&self) -> &[String] {
        &self.type_args
    }

    fn stdout(&mut self) -> &mut dyn Write {
        &mut self.stdout
    }
}

/// The id of the function at `path`, e.g. `std::io::println` or
/// `geo::Point::new`.
pub fn fn_id(registry: &Registry, path: &str) -> NativeFnId {
    fn collect(module: &NativeModule, prefix: &str, out: &mut Vec<(String, NativeFnId)>) {
        let prefix = format!("{prefix}{}::", module.name);
        for f in &module.functions {
            out.push((format!("{prefix}{}", f.name), f.id));
        }
        for ty in &module.types {
            for f in &ty.functions {
                out.push((format!("{prefix}{}::{}", ty.name, f.name), f.id));
            }
        }
        for child in &module.modules {
            collect(child, &prefix, out);
        }
    }
    let mut all = Vec::new();
    for module in registry.decls() {
        collect(&module, "", &mut all);
    }
    all.into_iter()
        .find(|(name, _)| name == path)
        .unwrap_or_else(|| panic!("no function `{path}`"))
        .1
}
