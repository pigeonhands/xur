use std::marker::PhantomData;

use facet::Facet;
use xur_resolve::NativeTypeId;

use super::hooks::TypeHooks;
use super::into_native::{IntoNativeFn, IntoNativeMethod, IntoNativeMethodMut};
use super::{CallFn, Context, NativeType, Registry, Signature};
use crate::{BuiltinType, RunResult, Value};

/// Adds items to one module of a [`Registry`].
pub struct ModuleBuilder<'r> {
    registry: &'r mut Registry,
    pub(super) module: usize,
}

impl<'r> ModuleBuilder<'r> {
    pub(super) fn new(registry: &'r mut Registry, module: usize) -> Self {
        Self { registry, module }
    }
}

impl ModuleBuilder<'_> {
    /// Registers a Rust function or closure whose argument and return
    /// types implement [`Facet`].
    pub fn function<Marker>(&mut self, name: &str, f: impl IntoNativeFn<Marker>) -> &mut Self {
        let (signature, call) = f.into_native();
        let decl = self.registry.add_fn(name, Some(signature), call);
        self.registry.modules[self.module].functions.push(decl);
        self
    }

    /// Registers a function that works on Xur values directly and can use
    /// the running program through its [`Context`].
    pub fn raw_function(
        &mut self,
        name: &str,
        f: impl Fn(&mut dyn Context, &[Value]) -> RunResult<Value> + Send + Sync + 'static,
    ) -> &mut Self {
        let decl = self.registry.add_fn(name, None, Box::new(f));
        self.registry.modules[self.module].functions.push(decl);
        self
    }

    /// Registers the Rust type `T` as `name`. Values of `T` passed to Xur
    /// become native objects holding the Rust value.
    pub fn ty<T: Facet<'static> + Clone + Send + 'static>(
        &mut self,
        name: &str,
    ) -> TypeBuilder<'_, T> {
        let id = NativeTypeId(self.registry.types.len() as u32);
        self.registry.types.push(NativeType {
            name: name.to_owned(),
            shape: T::SHAPE,
            functions: Vec::new(),
            hooks: TypeHooks::new::<T>(),
        });
        self.registry.modules[self.module].types.push(id);
        TypeBuilder {
            registry: self.registry,
            ty: id,
            _ty: PhantomData,
        }
    }
}

/// Adds functions to the registered native type `T`.
pub struct TypeBuilder<'r, T> {
    registry: &'r mut Registry,
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
        let decl = self.registry.add_fn(name, Some(signature), call);
        self.registry.types[self.ty.0 as usize].functions.push(decl);
        self
    }
}

/// Adds methods to a built-in type, like `list` or `str`.
pub struct BuiltinBuilder<'r> {
    registry: &'r mut Registry,
    ty: BuiltinType,
}

impl<'r> BuiltinBuilder<'r> {
    pub(super) fn new(registry: &'r mut Registry, ty: BuiltinType) -> Self {
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
        let decl = self.registry.add_fn(name, signature, call);
        self.registry
            .builtin_methods
            .insert((self.ty, name.to_owned()), decl.id);
        self
    }
}
