//! A stand-in for the interpreter, for unit-testing native functions
//! without running a program. Enabled by the `testing` feature.

use std::path::Path;

use xur_resolve::{NativeFnId, NativeModule, Program};
use xur_span::MemoryFileLoader;

use crate::convert::ConvertCx;
use crate::registry::Context;
use crate::{AdtInfo, LangItems, Registry, RunResult, RuntimeError, TypeRef, Value};
use xur_resolve::DefId;

pub struct TestCx<'r> {
    pub registry: &'r Registry,
    pub lang: LangItems,
    pub type_args: Vec<TypeRef>,
    pub stdout: Vec<u8>,
}

impl<'r> TestCx<'r> {
    pub fn new(registry: &'r Registry, prelude: &str) -> Self {
        let loader = MemoryFileLoader::new([("main.xur", "")]);
        let (program, _) = Program::load(
            Path::new("main.xur"),
            Box::new(loader),
            Box::new(Vec::new()),
            Some(prelude),
        )
        .unwrap();
        Self {
            registry,
            lang: program.lang_items(),
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

    fn registry(&self) -> &Registry {
        self.registry
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

    fn type_args(&self) -> &[TypeRef] {
        &self.type_args
    }

    fn print(&mut self, text: &str) -> RunResult<()> {
        self.stdout.extend_from_slice(text.as_bytes());
        Ok(())
    }

    fn call(&mut self, callee: &Value, args: &[Value]) -> RunResult<Value> {
        match callee {
            Value::NativeFn(id) => self.call(*id, args),
            _ => Err(RuntimeError::new(
                "only native functions can be called in tests",
            )),
        }
    }

    fn spawn(&mut self, _f: Value) -> RunResult<std::thread::JoinHandle<RunResult<Value>>> {
        Err(RuntimeError::new("cannot spawn threads in tests"))
    }

    fn describe_type(&self, value: &Value) -> String {
        value.kind().to_owned()
    }

    fn adt_info(&self, _def: DefId) -> Option<AdtInfo> {
        None
    }

    fn methods(&self, value: &Value) -> Vec<String> {
        match value {
            Value::Native(object) => self.registry.type_methods(object.ty),
            other => other
                .builtin_type()
                .map(|ty| self.registry.builtin_methods(ty))
                .unwrap_or_default(),
        }
    }

    fn field(&mut self, value: &Value, name: &str) -> RunResult<Value> {
        let missing = || RuntimeError::new(format!("no field `{name}` on {}", value.kind()));
        match value {
            Value::Record(record) => record
                .field(xur_span::Symbol::intern(name))
                .cloned()
                .ok_or_else(missing),
            Value::Native(object) => {
                let registry = self.registry;
                registry.field(self, object, name).ok_or_else(missing)?
            }
            _ => Err(missing()),
        }
    }

    fn set_field(&mut self, value: &Value, name: &str, _new: Value) -> RunResult<Value> {
        Err(RuntimeError::new(format!(
            "cannot set `{name}` of {} in tests",
            value.kind()
        )))
    }

    fn try_cast(&self, _value: &Value, _target: DefId) -> RunResult<Value> {
        Err(RuntimeError::new("try_cast is not available in tests"))
    }

    fn repr_value(&self, _def: DefId) -> Option<i64> {
        None
    }

    fn fn_info(&self, _value: &Value) -> Option<crate::FnInfo> {
        None
    }

    fn call_method(&mut self, receiver: Value, name: &str, args: &[Value]) -> RunResult<Value> {
        let id = match &receiver {
            Value::Native(object) => self.registry.type_function(object.ty, name),
            other => other
                .builtin_type()
                .and_then(|ty| self.registry.builtin_method(ty, name)),
        }
        .ok_or_else(|| RuntimeError::new(format!("no method `{name}` on {}", receiver.kind())))?;
        let mut full = Vec::with_capacity(1 + args.len());
        full.push(receiver);
        full.extend_from_slice(args);
        self.call(id, &full)
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
