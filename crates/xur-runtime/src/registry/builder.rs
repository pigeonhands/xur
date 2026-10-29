use std::marker::PhantomData;

use facet::{Facet, Shape};
use xur_resolve::NativeTypeId;

use super::hooks::TypeHooks;
use super::into_native::{IntoNativeFn, IntoNativeMethod, IntoNativeMethodMut};
use super::{CallFn, Context, NativeType, Registry, Signature};
use crate::{BuiltinType, RunResult, Value};
use xur_resolve::{NativeVariantDecl, NativeVariantKind};

/// A type whose static [`Shape`] can serve as a return-type hint for the LSP.
///
/// Blanket-implemented for every [`Facet`] type. Implement manually for
/// xur-only runtime types that have no Rust equivalent (see [`XurPtr`]).
pub trait Returns {
    const RET: &'static Shape;
}

impl<T: Facet<'static>> Returns for T {
    const RET: &'static Shape = T::SHAPE;
}

/// Runtime-only xur types that have no Facet equivalent. Use with the
/// `_xur` builder methods to annotate return types for the LSP.
#[derive(Clone, Copy)]
pub enum XurType {
    Ptr,
    List,
    Map,
    Unit,
}

#[derive(Facet)] struct ShapePtr;
#[derive(Facet)] struct ShapeList;
#[derive(Facet)] struct ShapeMap;

impl XurType {
    pub fn shape(self) -> &'static Shape {
        match self {
            Self::Ptr => ShapePtr::SHAPE,
            Self::List => ShapeList::SHAPE,
            Self::Map => ShapeMap::SHAPE,
            Self::Unit => <()>::SHAPE,
        }
    }
}

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
        self.add_module_fn(name, Some(signature), Vec::new(), call, None)
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
        self.add_module_fn(name, Some(signature), names, call, None)
    }

    /// Registers a function that works on Xur values directly and can use
    /// the running program through its [`Context`].
    pub fn raw_function(
        &mut self,
        name: &str,
        f: impl Fn(&mut dyn Context, &[Value]) -> RunResult<Value> + Send + Sync + 'static,
    ) -> &mut Self {
        self.add_module_fn(name, None, Vec::new(), Box::new(f), None)
    }

    /// Like [`raw_function`](Self::raw_function), declaring the return type
    /// for LSP inlay hints.
    pub fn raw_function_returns<R: Returns>(
        &mut self,
        name: &str,
        f: impl Fn(&mut dyn Context, &[Value]) -> RunResult<Value> + Send + Sync + 'static,
    ) -> &mut Self {
        self.add_module_fn(name, None, Vec::new(), Box::new(f), Some(R::RET))
    }

    /// Like [`raw_function_returns`](Self::raw_function_returns) for
    /// xur-internal types that have no Facet equivalent.
    pub fn raw_function_returns_xur(
        &mut self,
        name: &str,
        ret: XurType,
        f: impl Fn(&mut dyn Context, &[Value]) -> RunResult<Value> + Send + Sync + 'static,
    ) -> &mut Self {
        self.add_module_fn(name, None, Vec::new(), Box::new(f), Some(ret.shape()))
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
        self.add_module_fn(name, None, names, Box::new(f), None)
    }

    /// Like [`named_raw_function`](Self::named_raw_function), declaring the
    /// return type for LSP inlay hints.
    pub fn named_raw_function_returns<R: Returns>(
        &mut self,
        name: &str,
        params: &[&str],
        f: impl Fn(&mut dyn Context, &[Value]) -> RunResult<Value> + Send + Sync + 'static,
    ) -> &mut Self {
        let names = params.iter().map(|s| s.to_string()).collect();
        self.add_module_fn(name, None, names, Box::new(f), Some(R::RET))
    }

    /// Like [`named_raw_function_returns`](Self::named_raw_function_returns)
    /// for xur-internal types that have no Facet equivalent.
    pub fn named_raw_function_returns_xur(
        &mut self,
        name: &str,
        params: &[&str],
        ret: XurType,
        f: impl Fn(&mut dyn Context, &[Value]) -> RunResult<Value> + Send + Sync + 'static,
    ) -> &mut Self {
        let names = params.iter().map(|s| s.to_string()).collect();
        self.add_module_fn(name, None, names, Box::new(f), Some(ret.shape()))
    }

    fn add_module_fn(
        &mut self,
        name: &str,
        signature: Option<Signature>,
        names: Vec<String>,
        call: Box<CallFn>,
        return_hint: Option<&'static Shape>,
    ) -> &mut Self {
        let decl = self.registry.add_fn(name, signature, names, call);
        if let Some(shape) = return_hint {
            self.registry.set_return_hint(decl.id, shape);
        }
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
                explicit_variants: None,
            });
            inner.modules[module].types.push(id);
            id
        })
    }

    /// Registers a `#[repr(inttype)]` enum whose variants are integer
    /// constants. Each `(name, value)` pair becomes a unit variant that
    /// lowers to `Const(Int(value))`.
    pub fn repr_enum(&mut self, name: &str, variants: &[(&str, i64)]) -> &mut Self {
        let module = self.module;
        let variant_decls: Vec<NativeVariantDecl> = variants
            .iter()
            .map(|(vname, value)| NativeVariantDecl {
                name: vname.to_string(),
                kind: NativeVariantKind::Unit,
                repr_value: Some(*value),
            })
            .collect();
        self.registry.with_inner(|inner| {
            let id = NativeTypeId(inner.types.len() as u32);
            inner.types.push(NativeType {
                name: name.to_owned(),
                shape: <()>::SHAPE,
                functions: Vec::new(),
                hooks: None,
                explicit_variants: Some(variant_decls),
            });
            inner.modules[module].types.push(id);
        });
        self
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
                explicit_variants: None,
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
                explicit_variants: None,
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
        self.add(name, Some(signature), call, None)
    }

    /// Registers a function working on Xur values directly, with the
    /// object first when called as a method. For methods taking or
    /// returning arbitrary Xur values, like a channel's `send`.
    pub fn raw_function(
        &mut self,
        name: &str,
        f: impl Fn(&mut dyn Context, &[Value]) -> RunResult<Value> + Send + Sync + 'static,
    ) -> &mut Self {
        self.add(name, None, Box::new(f), None)
    }

    /// Like [`raw_function`](Self::raw_function), declaring the return type
    /// for LSP inlay hints.
    pub fn raw_function_returns<R: Returns>(
        &mut self,
        name: &str,
        f: impl Fn(&mut dyn Context, &[Value]) -> RunResult<Value> + Send + Sync + 'static,
    ) -> &mut Self {
        self.add(name, None, Box::new(f), Some(R::RET))
    }

    /// Like [`raw_function_returns`](Self::raw_function_returns) for
    /// xur-internal types that have no Facet equivalent.
    pub fn raw_function_returns_xur(
        &mut self,
        name: &str,
        ret: XurType,
        f: impl Fn(&mut dyn Context, &[Value]) -> RunResult<Value> + Send + Sync + 'static,
    ) -> &mut Self {
        self.add(name, None, Box::new(f), Some(ret.shape()))
    }

    /// Registers a method taking `&self`: `value.name(args)` in Xur.
    pub fn method<Marker>(&mut self, name: &str, f: impl IntoNativeMethod<T, Marker>) -> &mut Self {
        let (signature, call) = f.into_native();
        self.add(name, Some(signature), call, None)
    }

    /// Registers a method taking `&mut self`, which changes the object in
    /// place.
    pub fn method_mut<Marker>(
        &mut self,
        name: &str,
        f: impl IntoNativeMethodMut<T, Marker>,
    ) -> &mut Self {
        let (signature, call) = f.into_native();
        self.add(name, Some(signature), call, None)
    }

    fn add(
        &mut self,
        name: &str,
        signature: Option<Signature>,
        call: Box<CallFn>,
        return_hint: Option<&'static Shape>,
    ) -> &mut Self {
        let decl = self.registry.add_fn(name, signature, Vec::new(), call);
        if let Some(shape) = return_hint {
            self.registry.set_return_hint(decl.id, shape);
        }
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
        self.add(name, Some(signature), call, None)
    }

    /// Registers a method working on Xur values directly, with the receiver
    /// first; needed to change a list in place.
    pub fn raw_function(
        &mut self,
        name: &str,
        f: impl Fn(&mut dyn Context, &[Value]) -> RunResult<Value> + Send + Sync + 'static,
    ) -> &mut Self {
        self.add(name, None, Box::new(f), None)
    }

    /// Like [`raw_function`](Self::raw_function), declaring the return type
    /// for LSP inlay hints.
    pub fn raw_function_returns<R: Returns>(
        &mut self,
        name: &str,
        f: impl Fn(&mut dyn Context, &[Value]) -> RunResult<Value> + Send + Sync + 'static,
    ) -> &mut Self {
        self.add(name, None, Box::new(f), Some(R::RET))
    }

    /// Like [`raw_function_returns`](Self::raw_function_returns) for
    /// xur-internal types that have no Facet equivalent.
    pub fn raw_function_returns_xur(
        &mut self,
        name: &str,
        ret: XurType,
        f: impl Fn(&mut dyn Context, &[Value]) -> RunResult<Value> + Send + Sync + 'static,
    ) -> &mut Self {
        self.add(name, None, Box::new(f), Some(ret.shape()))
    }

    fn add(
        &mut self,
        name: &str,
        signature: Option<Signature>,
        call: Box<CallFn>,
        return_hint: Option<&'static Shape>,
    ) -> &mut Self {
        let decl = self.registry.add_fn(name, signature, Vec::new(), call);
        if let Some(shape) = return_hint {
            self.registry.set_return_hint(decl.id, shape);
        }
        let key = (self.ty, name.to_owned());
        self.registry
            .with_inner(|inner| inner.builtin_methods.insert(key, decl.id));
        self
    }
}
