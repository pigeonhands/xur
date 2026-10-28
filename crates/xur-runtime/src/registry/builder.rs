use std::marker::PhantomData;

use facet::{Facet, Shape};
use xur_resolve::NativeTypeId;

use super::hooks::TypeHooks;
use super::into_native::{IntoNativeFn, IntoNativeMethod, IntoNativeMethodMut};
use super::{CallFn, Context, NativeType, Registry, Signature};
use crate::{BuiltinType, RunResult, Value};

/// Adds items to one module of a [`Registry`].
pub struct ModuleBuilder<'r> {
    registry: &'r Registry,
    pub(super) module: usize,
}

impl<'r> ModuleBuilder<'r> {
    pub(super) fn new(registry: &'r Registry, module: usize) -> Self {
        Self { registry, module }
    }

    /// The registry being filled in.
    pub fn registry(&self) -> &'r Registry {
        self.registry
    }

    /// The submodule `name` of this module, created if needed. A lazy
    /// module can fill in submodules as well as itself.
    pub fn submodule(&self, name: &str) -> ModuleBuilder<'r> {
        let parent = self.module;
        let module = self
            .registry
            .with_inner(|inner| inner.child(Some(parent), name));
        ModuleBuilder::new(self.registry, module)
    }
}

impl ModuleBuilder<'_> {
    /// Registers a Rust function or closure whose argument and return
    /// types implement [`Facet`].
    pub fn function<Marker>(&mut self, name: &str, f: impl IntoNativeFn<Marker>) -> &mut Self {
        let (signature, call) = f.into_native();
        let decl = self.registry.add_fn(name, Some(signature), Vec::new(), call);
        let module = self.module;
        self.registry
            .with_inner(|inner| inner.modules[module].functions.push(decl));
        self
    }

    /// Like [`function`](Self::function), with explicit parameter names for
    /// LSP hover and inlay hints.
    pub fn named_function<Marker>(
        &mut self,
        name: &str,
        params: &[&str],
        f: impl IntoNativeFn<Marker>,
    ) -> &mut Self {
        let (signature, call) = f.into_native();
        let names = params.iter().map(|s| s.to_string()).collect();
        let decl = self.registry.add_fn(name, Some(signature), names, call);
        let module = self.module;
        self.registry
            .with_inner(|inner| inner.modules[module].functions.push(decl));
        self
    }

    /// Registers a function that works on Xur values directly and can use
    /// the running program through its [`Context`].
    pub fn raw_function(
        &mut self,
        name: &str,
        f: impl Fn(&mut dyn Context, &[Value]) -> RunResult<Value> + Send + Sync + 'static,
    ) -> &mut Self {
        let decl = self.registry.add_fn(name, None, Vec::new(), Box::new(f));
        let module = self.module;
        self.registry
            .with_inner(|inner| inner.modules[module].functions.push(decl));
        self
    }

    /// Like [`raw_function`](Self::raw_function), with explicit parameter
    /// names for LSP hover and inlay hints.
    pub fn named_raw_function(
        &mut self,
        name: &str,
        params: &[&str],
        f: impl Fn(&mut dyn Context, &[Value]) -> RunResult<Value> + Send + Sync + 'static,
    ) -> &mut Self {
        let names = params.iter().map(|s| s.to_string()).collect();
        let decl = self.registry.add_fn(name, None, names, Box::new(f));
        let module = self.module;
        self.registry
            .with_inner(|inner| inner.modules[module].functions.push(decl));
        self
    }

    /// Registers the type with facet shape `shape` as `name`, without its
    /// Rust type: values of it passed to Xur become records of this type,
    /// which written types can check and which display with its name. For
    /// registering every type reachable from some shape, such as a
    /// library's.
    pub fn shape_type(&mut self, name: &str, shape: &'static Shape) -> NativeTypeId {
        let module = self.module;
        self.registry.with_inner(|inner| {
            let id = NativeTypeId(inner.types.len() as u32);
            inner.types.push(NativeType {
                name: name.to_owned(),
                shape,
                functions: Vec::new(),
                hooks: None,
            });
            inner.modules[module].types.push(id);
            id
        })
    }

    /// Registers the Rust type `T` as `name`. Values of `T` passed to Xur
    /// become native objects holding the Rust value.
    pub fn ty<T: Facet<'static> + Clone + Send + 'static>(
        &mut self,
        name: &str,
    ) -> TypeBuilder<'_, T> {
        let module = self.module;
        let id = self.registry.with_inner(|inner| {
            let id = NativeTypeId(inner.types.len() as u32);
            inner.types.push(NativeType {
                name: name.to_owned(),
                shape: T::SHAPE,
                functions: Vec::new(),
                hooks: Some(TypeHooks::new::<T>()),
            });
            inner.modules[module].types.push(id);
            id
        });
        TypeBuilder {
            registry: self.registry,
            ty: id,
            _ty: PhantomData,
        }
    }

    /// Registers the Rust type `T` as `name` without requiring `Clone`, for
    /// a type that borrows memory kept alive beside it (see
    /// [`Registry::borrowing_object`]). Its fields and methods work, but it
    /// cannot be cloned into another value or rebuilt from Xur.
    pub fn ty_borrowing<T: Facet<'static> + Send + 'static>(
        &mut self,
        name: &str,
    ) -> TypeBuilder<'_, T> {
        let module = self.module;
        let id = self.registry.with_inner(|inner| {
            let id = NativeTypeId(inner.types.len() as u32);
            inner.types.push(NativeType {
                name: name.to_owned(),
                shape: T::SHAPE,
                functions: Vec::new(),
                hooks: Some(TypeHooks::borrowing::<T>()),
            });
            inner.modules[module].types.push(id);
            id
        });
        TypeBuilder {
            registry: self.registry,
            ty: id,
            _ty: PhantomData,
        }
    }
}

/// Adds functions to the registered native type `T`.
pub struct TypeBuilder<'r, T> {
    registry: &'r Registry,
    ty: NativeTypeId,
    _ty: PhantomData<T>,
}

impl<T: Facet<'static> + 'static> TypeBuilder<'_, T> {
    /// Registers the associated function `Type::name`. If its first
    /// parameter is `T` it can also be called as a method, on a copy.
    pub fn function<Marker>(&mut self, name: &str, f: impl IntoNativeFn<Marker>) -> &mut Self {
        let (signature, call) = f.into_native();
        self.add(name, signature, call)
    }

    /// Registers a function working on Xur values directly, with the
    /// object first when called as a method. For methods taking or
    /// returning arbitrary Xur values, like a channel's `send`.
    pub fn raw_function(
        &mut self,
        name: &str,
        f: impl Fn(&mut dyn Context, &[Value]) -> RunResult<Value> + Send + Sync + 'static,
    ) -> &mut Self {
        let decl = self.registry.add_fn(name, None, Vec::new(), Box::new(f));
        let ty = self.ty;
        self.registry
            .with_inner(|inner| inner.types[ty.0 as usize].functions.push(decl));
        self
    }

    /// Registers a method taking `&self`: `value.name(args)` in Xur.
    pub fn method<Marker>(&mut self, name: &str, f: impl IntoNativeMethod<T, Marker>) -> &mut Self {
        let (signature, call) = f.into_native();
        self.add(name, signature, call)
    }

    /// Registers a method taking `&mut self`, which changes the object in
    /// place.
    pub fn method_mut<Marker>(
        &mut self,
        name: &str,
        f: impl IntoNativeMethodMut<T, Marker>,
    ) -> &mut Self {
        let (signature, call) = f.into_native();
        self.add(name, signature, call)
    }

    fn add(&mut self, name: &str, signature: Signature, call: Box<CallFn>) -> &mut Self {
        let decl = self.registry.add_fn(name, Some(signature), Vec::new(), call);
        let ty = self.ty;
        self.registry
            .with_inner(|inner| inner.types[ty.0 as usize].functions.push(decl));
        self
    }
}

/// Adds methods to a built-in type, like `list` or `str`.
pub struct BuiltinBuilder<'r> {
    registry: &'r Registry,
    ty: BuiltinType,
}

impl<'r> BuiltinBuilder<'r> {
    pub(super) fn new(registry: &'r Registry, ty: BuiltinType) -> Self {
        Self { registry, ty }
    }

    /// Registers a method as a Rust function taking the receiver, converted,
    /// first: `|s: String| s.to_uppercase()` for `"a".to_upper()`.
    pub fn function<Marker>(&mut self, name: &str, f: impl IntoNativeFn<Marker>) -> &mut Self {
        let (signature, call) = f.into_native();
        self.add(name, Some(signature), call)
    }

    /// Registers a method working on Xur values directly, with the receiver
    /// first; needed to change a list in place.
    pub fn raw_function(
        &mut self,
        name: &str,
        f: impl Fn(&mut dyn Context, &[Value]) -> RunResult<Value> + Send + Sync + 'static,
    ) -> &mut Self {
        self.add(name, None, Box::new(f))
    }

    fn add(&mut self, name: &str, signature: Option<Signature>, call: Box<CallFn>) -> &mut Self {
        let decl = self.registry.add_fn(name, signature, Vec::new(), call);
        let key = (self.ty, name.to_owned());
        self.registry
            .with_inner(|inner| inner.builtin_methods.insert(key, decl.id));
        self
    }
}
