//! Registering Rust functions and types as Xur modules.
//!
//! ```ignore
//! let mut registry = Registry::new();
//! registry
//!     .module("std::math")
//!     .function("sqrt", |x: f64| x.sqrt())
//!     .function("max", |a: i64, b: i64| a.max(b));
//! registry
//!     .module("geo")
//!     .ty::<Point>("Point")
//!     .function("new", |x: f64, y: f64| Point { x, y }) // `Point::new(1.0, 2.0)`
//!     .method("norm", |p: &Point| p.x.hypot(p.y)) //     `p.norm()`
//!     .method_mut("scale", |p: &mut Point, k: f64| { p.x *= k; p.y *= k });
//! ```
//!
//! Any `Fn(A, B, ...) -> R` whose argument and return types implement
//! [`Facet`] can be registered: arguments are converted from Xur values and
//! the result back with [`convert`](crate::convert). Functions that need the
//! running program itself, such as `std::import`, are registered with
//! [`ModuleBuilder::raw_function`] and receive a [`Context`].
//!
//! Values of a registered type live in Xur as [`Value::Native`]: the Rust
//! value itself, shared, so methods taking `&mut self` change it in place.
//! Its fields can be read (`p.x`) and it is displayed with its `Display` or
//! `Debug` implementation.

mod builder;
mod hooks;
mod into_native;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::io::Write;

use facet::{Peek, Shape};
use xur_resolve::{
    NativeFnDecl, NativeFnId, NativeModule, NativeProvider, NativeTypeDecl, NativeTypeId,
};

use crate::convert::{Builder, ConvertCx};
use crate::value::NativeObject;
use crate::{BuiltinType, RunResult, Value};

pub use builder::{BuiltinBuilder, ModuleBuilder, TypeBuilder};
pub use into_native::{IntoNativeFn, IntoNativeMethod, IntoNativeMethodMut};

use hooks::TypeHooks;

/// What a native function can ask of the running program.
pub trait Context: ConvertCx {
    /// Loads the module at `path`, relative to the file of the module
    /// making the call, and returns it as a value.
    fn import(&mut self, path: &str) -> RunResult<Value>;
    /// Formats `value` as `{}` does in format strings.
    fn display(&self, value: &Value) -> String;
    /// The names of the generic arguments the native function was called
    /// with, as in `type_name::<T>()`.
    fn type_args(&self) -> &[String];
    /// Where the program's standard output goes.
    fn stdout(&mut self) -> &mut dyn Write;
}

pub(crate) type CallFn = dyn Fn(&mut dyn Context, &[Value]) -> RunResult<Value> + Send + Sync;

/// The Rust types of a typed native function, for checking calls.
#[derive(Debug, Clone)]
pub struct Signature {
    pub params: Vec<&'static Shape>,
    pub ret: &'static Shape,
}

pub struct NativeFn {
    pub name: String,
    /// `None` for raw functions, which take any arguments.
    pub signature: Option<Signature>,
    call: Box<CallFn>,
}

impl NativeFn {
    pub fn call(&self, cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
        (self.call)(cx, args)
    }
}

struct NativeType {
    name: String,
    shape: &'static Shape,
    functions: Vec<NativeFnDecl>,
    hooks: TypeHooks,
}

/// Fills in a lazy module, the first time a program uses it.
type ModuleInit = Box<dyn FnOnce(&mut ModuleBuilder<'_>) + Send + Sync>;

struct ModuleData {
    name: String,
    /// For a lazy module not yet needed, what registers its contents.
    init: Option<ModuleInit>,
    children: Vec<usize>,
    functions: Vec<NativeFnDecl>,
    types: Vec<NativeTypeId>,
}

/// Every native function and type, organised into modules.
#[derive(Default)]
pub struct Registry {
    fns: Vec<NativeFn>,
    /// Methods of built-in values, such as `list.push`.
    builtin_methods: HashMap<(BuiltinType, String), NativeFnId>,
    types: Vec<NativeType>,
    modules: Vec<ModuleData>,
    /// Modules that are not submodules of another, like `std`.
    roots: Vec<usize>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// The module at `path`, such as `std::io`, created if needed.
    pub fn module(&mut self, path: &str) -> ModuleBuilder<'_> {
        let mut names = path.split("::");
        let root = names.next().expect("split yields at least one item");
        let mut module = self.child(None, root);
        for name in names {
            module = self.child(Some(module), name);
        }
        ModuleBuilder::new(self, module)
    }

    /// Registers the module at `path` lazily: `init` registers its contents
    /// the first time a program looks inside it, so large modules that a
    /// program doesn't use cost nothing.
    pub fn lazy_module(
        &mut self,
        path: &str,
        init: impl FnOnce(&mut ModuleBuilder<'_>) + Send + Sync + 'static,
    ) {
        let module = self.module(path).module;
        self.modules[module].init = Some(Box::new(init));
    }

    /// Methods of the built-in type `ty`, such as `list.push(x)`.
    pub fn builtin(&mut self, ty: BuiltinType) -> BuiltinBuilder<'_> {
        BuiltinBuilder::new(self, ty)
    }

    /// The method `name` of the built-in type `ty`.
    pub fn builtin_method(&self, ty: BuiltinType, name: &str) -> Option<NativeFnId> {
        self.builtin_methods.get(&(ty, name.to_owned())).copied()
    }

    pub fn function(&self, id: NativeFnId) -> &NativeFn {
        &self.fns[id.0 as usize]
    }

    pub fn type_name(&self, id: NativeTypeId) -> &str {
        &self.types[id.0 as usize].name
    }

    /// The native type registered for `shape`, if any.
    pub fn type_of_shape(&self, shape: &'static Shape) -> Option<NativeTypeId> {
        let index = self.types.iter().position(|ty| ty.shape == shape)?;
        Some(NativeTypeId(index as u32))
    }

    /// Reads field `name` of a native object.
    pub fn field(
        &self,
        cx: &dyn ConvertCx,
        object: &NativeObject,
        name: &str,
    ) -> Option<RunResult<Value>> {
        object.with_any(|value| (self.hooks(object.ty).field)(cx, value, name))
    }

    /// Formats a native object with its `Display`, or else `Debug`.
    pub fn display(&self, object: &NativeObject) -> String {
        object.with_any(self.hooks(object.ty).display)
    }

    /// Whether two native objects are equal by their Rust `PartialEq`.
    pub fn native_eq(&self, a: &NativeObject, b: &NativeObject) -> bool {
        if a.ty != b.ty {
            return false;
        }
        if std::ptr::eq(a, b) {
            return true;
        }
        let eq = self.hooks(a.ty).eq;
        a.with_any(|a| b.with_any(|b| eq(a, b)))
    }

    /// A native object holding a clone of the registered value behind `peek`.
    pub(crate) fn object_from_peek(
        &self,
        ty: NativeTypeId,
        peek: Peek<'_, 'static>,
    ) -> Option<NativeObject> {
        let value = (self.hooks(ty).clone_peek)(peek)?;
        Some(NativeObject::new(ty, value))
    }

    /// Writes a clone of `object` into a builder for its type.
    pub(crate) fn set_object(&self, partial: Builder, object: &NativeObject) -> RunResult<Builder> {
        object.with_any(|value| (self.hooks(object.ty).set)(partial, value))
    }

    fn hooks(&self, ty: NativeTypeId) -> &TypeHooks {
        &self.types[ty.0 as usize].hooks
    }

    /// The registered modules as name resolution sees them.
    pub fn decls(&self) -> Vec<NativeModule> {
        self.roots.iter().map(|&module| self.decl(module)).collect()
    }

    fn decl(&self, module: usize) -> NativeModule {
        let data = &self.modules[module];
        if data.init.is_some() {
            return NativeModule {
                name: data.name.clone(),
                lazy: true,
                ..NativeModule::default()
            };
        }
        NativeModule {
            name: data.name.clone(),
            lazy: false,
            functions: data.functions.clone(),
            types: data
                .types
                .iter()
                .map(|&id| {
                    let ty = &self.types[id.0 as usize];
                    NativeTypeDecl {
                        name: ty.name.clone(),
                        id,
                        functions: ty.functions.clone(),
                    }
                })
                .collect(),
            modules: data
                .children
                .iter()
                .map(|&child| self.decl(child))
                .collect(),
        }
    }

    /// The submodule `name` of `parent` (or the root module `name`).
    fn child(&mut self, parent: Option<usize>, name: &str) -> usize {
        let siblings = match parent {
            Some(parent) => &self.modules[parent].children,
            None => &self.roots,
        };
        if let Some(&existing) = siblings.iter().find(|&&m| self.modules[m].name == name) {
            return existing;
        }
        let module = self.modules.len();
        self.modules.push(ModuleData {
            name: name.to_owned(),
            init: None,
            children: Vec::new(),
            functions: Vec::new(),
            types: Vec::new(),
        });
        match parent {
            Some(parent) => self.modules[parent].children.push(module),
            None => self.roots.push(module),
        }
        module
    }

    fn add_fn(
        &mut self,
        name: &str,
        signature: Option<Signature>,
        call: Box<CallFn>,
    ) -> NativeFnDecl {
        let id = NativeFnId(self.fns.len() as u32);
        self.fns.push(NativeFn {
            name: name.to_owned(),
            signature,
            call,
        });
        NativeFnDecl {
            name: name.to_owned(),
            id,
        }
    }
}

impl NativeProvider for Registry {
    fn modules(&self) -> Vec<NativeModule> {
        self.decls()
    }

    fn expand(&mut self, path: &[String]) -> NativeModule {
        let mut module: Option<usize> = None;
        for name in path {
            let siblings = match module {
                Some(parent) => &self.modules[parent].children,
                None => &self.roots,
            };
            module = siblings
                .iter()
                .copied()
                .find(|&m| self.modules[m].name == *name);
        }
        let module = module.unwrap_or_else(|| panic!("no native module `{}`", path.join("::")));
        if let Some(init) = self.modules[module].init.take() {
            init(&mut ModuleBuilder::new(self, module));
        }
        self.decl(module)
    }
}
