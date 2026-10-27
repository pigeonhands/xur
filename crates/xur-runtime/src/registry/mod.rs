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
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::thread::JoinHandle;

use facet::{Peek, Shape, StructType, Type, UserType};
use xur_resolve::{
    DefId, NativeFnDecl, NativeFnId, NativeModule, NativeProvider, NativeTypeDecl, NativeTypeId,
};

use crate::convert::{Builder, ConvertCx};
use crate::value::NativeObject;
use crate::{AdtInfo, BuiltinType, RunResult, TypeRef, Value};

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
    /// The generic arguments the native function was called with, as in
    /// `type_name::<T>()`.
    fn type_args(&self) -> &[TypeRef];
    /// Writes `text` to the program's standard output.
    fn print(&mut self, text: &str) -> RunResult<()>;
    /// Calls a function, closure or constructor value.
    fn call(&mut self, callee: &Value, args: Vec<Value>) -> RunResult<Value>;
    /// Calls `f`, a function or closure taking no arguments, on a new
    /// thread; joining the thread gives its result.
    fn spawn(&mut self, f: Value) -> RunResult<JoinHandle<RunResult<Value>>>;

    // Reflection, for `std::reflect`.

    /// How the type of `value` is named in messages, e.g. `` `Point` ``.
    fn describe_type(&self, value: &Value) -> String;
    /// The struct or enum variant `def`, or `None` if it is neither.
    fn adt_info(&self, def: DefId) -> Option<AdtInfo>;
    /// The names of the methods `value.name()` can call.
    fn methods(&self, value: &Value) -> Vec<String>;
    /// `value.name`.
    fn field(&mut self, value: &Value, name: &str) -> RunResult<Value>;
    /// A copy of the struct `value` with the field `name` set to `new`.
    fn set_field(&mut self, value: &Value, name: &str, new: Value) -> RunResult<Value>;
    /// `receiver.name(args)`.
    fn call_method(&mut self, receiver: Value, name: &str, args: Vec<Value>) -> RunResult<Value>;
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

pub(super) struct NativeType {
    name: String,
    shape: &'static Shape,
    functions: Vec<NativeFnDecl>,
    /// `None` for types registered by shape only, whose values are
    /// records rather than native objects.
    hooks: Option<TypeHooks>,
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
///
/// A registry is shared by every thread of a running program and by name
/// resolution, which may expand lazy modules while a native function runs,
/// so it is internally locked: each operation holds the lock only briefly,
/// and functions are handed out as `Arc`s to call without it.
#[derive(Default)]
pub struct Registry {
    inner: RwLock<Inner>,
}

#[derive(Default)]
struct Inner {
    fns: Vec<Arc<NativeFn>>,
    /// Methods of built-in values, such as `list.push`.
    builtin_methods: HashMap<(BuiltinType, String), NativeFnId>,
    types: Vec<NativeType>,
    modules: Vec<ModuleData>,
    /// Modules that are not submodules of another, like `std`.
    roots: Vec<usize>,
    /// Conversions to Xur for shapes facet cannot see inside.
    converters: Vec<(&'static Shape, Converter)>,
}

/// Converts a value of some shape to Xur, given a peek at it.
pub type Converter = fn(&dyn ConvertCx, Peek<'_, 'static>) -> RunResult<Value>;

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    fn read(&self) -> RwLockReadGuard<'_, Inner> {
        self.inner.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, Inner> {
        self.inner.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// The module at `path`, such as `std::io`, created if needed.
    pub fn module(&self, path: &str) -> ModuleBuilder<'_> {
        let module = self.write().module_at(path);
        ModuleBuilder::new(self, module)
    }

    /// Registers the module at `path` lazily: `init` registers its contents
    /// the first time a program looks inside it, so large modules that a
    /// program doesn't use cost nothing.
    pub fn lazy_module(
        &self,
        path: &str,
        init: impl FnOnce(&mut ModuleBuilder<'_>) + Send + Sync + 'static,
    ) {
        let mut inner = self.write();
        let module = inner.module_at(path);
        inner.modules[module].init = Some(Box::new(init));
    }

    /// Methods of the built-in type `ty`, such as `list.push(x)`.
    pub fn builtin(&self, ty: BuiltinType) -> BuiltinBuilder<'_> {
        BuiltinBuilder::new(self, ty)
    }

    /// The method `name` of the built-in type `ty`.
    pub fn builtin_method(&self, ty: BuiltinType, name: &str) -> Option<NativeFnId> {
        self.read()
            .builtin_methods
            .get(&(ty, name.to_owned()))
            .copied()
    }

    /// The function `name` registered for the native type `ty`.
    pub fn type_function(&self, ty: NativeTypeId, name: &str) -> Option<NativeFnId> {
        let inner = self.read();
        let functions = &inner.types[ty.0 as usize].functions;
        functions.iter().find(|f| f.name == name).map(|f| f.id)
    }

    /// The functions of the native type `ty` that can be called as methods:
    /// those whose first parameter is the type itself, and raw functions.
    pub fn type_methods(&self, ty: NativeTypeId) -> Vec<String> {
        let inner = self.read();
        let native = &inner.types[ty.0 as usize];
        native
            .functions
            .iter()
            .filter(|decl| match &inner.fns[decl.id.0 as usize].signature {
                Some(signature) => signature.params.first() == Some(&native.shape),
                None => true,
            })
            .map(|decl| decl.name.clone())
            .collect()
    }

    /// The names of the methods of the built-in type `ty`, sorted.
    pub fn builtin_methods(&self, ty: BuiltinType) -> Vec<String> {
        let mut names: Vec<_> = self
            .read()
            .builtin_methods
            .keys()
            .filter(|(builtin, _)| *builtin == ty)
            .map(|(_, name)| name.clone())
            .collect();
        names.sort();
        names
    }

    /// The names of the fields of the native type `ty`, if it is a struct.
    pub fn type_fields(&self, ty: NativeTypeId) -> Vec<String> {
        let shape = self.read().types[ty.0 as usize].shape;
        match shape.ty {
            Type::User(UserType::Struct(StructType { fields, .. })) => {
                fields.iter().map(|field| field.name.to_owned()).collect()
            }
            _ => Vec::new(),
        }
    }

    pub fn function(&self, id: NativeFnId) -> Arc<NativeFn> {
        Arc::clone(&self.read().fns[id.0 as usize])
    }

    pub fn type_name(&self, id: NativeTypeId) -> String {
        self.read().types[id.0 as usize].name.clone()
    }

    /// The native type registered for `shape`, if any.
    pub fn type_of_shape(&self, shape: &'static Shape) -> Option<NativeTypeId> {
        let index = self
            .read()
            .types
            .iter()
            .position(|ty| ty.shape == shape && ty.hooks.is_some())?;
        Some(NativeTypeId(index as u32))
    }

    /// The type registered by shape only for `shape`, whose values are
    /// typed records.
    pub fn record_type_of_shape(&self, shape: &'static Shape) -> Option<NativeTypeId> {
        let index = self
            .read()
            .types
            .iter()
            .position(|ty| ty.shape == shape && ty.hooks.is_none())?;
        Some(NativeTypeId(index as u32))
    }

    /// Converts values of `shape` to Xur with `convert`, for types facet
    /// cannot see inside, such as opaque ones. Such values cannot be
    /// converted back.
    pub fn converter(&self, shape: &'static Shape, convert: Converter) {
        self.write().converters.push((shape, convert));
    }

    /// Whether a conversion is registered for `shape`.
    pub fn has_converter(&self, shape: &'static Shape) -> bool {
        self.converter_of(shape).is_some()
    }

    /// The conversion registered for `shape`, if any.
    pub(crate) fn converter_of(&self, shape: &'static Shape) -> Option<Converter> {
        let inner = self.read();
        let (_, convert) = inner.converters.iter().find(|(s, _)| *s == shape)?;
        Some(*convert)
    }

    /// Whether any type, of either kind, is registered for `shape`.
    pub fn has_shape(&self, shape: &'static Shape) -> bool {
        self.read().types.iter().any(|ty| ty.shape == shape)
    }

    /// Reads field `name` of a native object.
    pub fn field(
        &self,
        cx: &dyn ConvertCx,
        object: &NativeObject,
        name: &str,
    ) -> Option<RunResult<Value>> {
        let field = self.hooks(object.ty).field;
        object.with_any(|value| field(cx, value, name))
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
        let set = self.hooks(object.ty).set;
        object.with_any(|value| set(partial, value))
    }

    /// The hooks of a type registered with its Rust type, which every type
    /// with native objects is.
    fn hooks(&self, ty: NativeTypeId) -> TypeHooks {
        self.read().types[ty.0 as usize]
            .hooks
            .expect("native objects only exist for types registered with their Rust type")
    }

    /// The registered modules as name resolution sees them.
    pub fn decls(&self) -> Vec<NativeModule> {
        let inner = self.read();
        inner
            .roots
            .iter()
            .map(|&module| inner.decl(module))
            .collect()
    }

    /// Registers the contents of the lazy module at `path`, if not yet
    /// registered, and describes it.
    pub fn expand(&self, path: &[String]) -> NativeModule {
        let (module, init) = {
            let mut inner = self.write();
            let module = inner
                .find(path)
                .unwrap_or_else(|| panic!("no native module `{}`", path.join("::")));
            (module, inner.modules[module].init.take())
        };
        // The initializer registers through the builder, which locks.
        if let Some(init) = init {
            init(&mut ModuleBuilder::new(self, module));
        }
        self.read().decl(module)
    }

    pub(super) fn add_fn(
        &self,
        name: &str,
        signature: Option<Signature>,
        call: Box<CallFn>,
    ) -> NativeFnDecl {
        self.write().add_fn(name, signature, call)
    }

    fn with_inner<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        f(&mut self.write())
    }
}

impl Inner {
    fn module_at(&mut self, path: &str) -> usize {
        let mut names = path.split("::");
        let root = names.next().expect("split yields at least one item");
        let mut module = self.child(None, root);
        for name in names {
            module = self.child(Some(module), name);
        }
        module
    }

    fn find(&self, path: &[String]) -> Option<usize> {
        let mut module: Option<usize> = None;
        for name in path {
            let siblings = match module {
                Some(parent) => &self.modules[parent].children,
                None => &self.roots,
            };
            module = Some(*siblings.iter().find(|&&m| self.modules[m].name == *name)?);
        }
        module
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
        self.fns.push(Arc::new(NativeFn {
            name: name.to_owned(),
            signature,
            call,
        }));
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

    fn expand(&self, path: &[String]) -> NativeModule {
        Registry::expand(self, path)
    }
}
