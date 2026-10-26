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

use std::any::Any;
use std::io::Write;
use std::marker::PhantomData;

use facet::{Facet, Peek, Shape};
use xur_resolve::{NativeFnDecl, NativeFnId, NativeModule, NativeTypeDecl, NativeTypeId};

use crate::convert::{Builder, ConvertCx, from_value, into_value, peek_to_value, reflect_error};
use crate::value::NativeObject;
use crate::{RunResult, RuntimeError, Value};

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

type CallFn = dyn Fn(&mut dyn Context, &[Value]) -> RunResult<Value>;

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

/// Operations on values of one registered type, instantiated where the type
/// is still known (in [`ModuleBuilder::ty`]) and used where it is erased.
struct TypeHooks {
    /// Clones the value behind a peek, for a registered value nested inside
    /// another, like the elements of a `Vec<Point>`.
    clone_peek: fn(Peek<'_, 'static>) -> Option<Box<dyn Any>>,
    /// Writes a clone of the value into a builder for the type.
    set: fn(Builder, &dyn Any) -> RunResult<Builder>,
    /// Reads a field by name.
    field: fn(&dyn ConvertCx, &dyn Any, &str) -> Option<RunResult<Value>>,
    display: fn(&dyn Any) -> String,
    eq: fn(&dyn Any, &dyn Any) -> bool,
}

impl TypeHooks {
    fn new<T: Facet<'static> + Clone + 'static>() -> Self {
        Self {
            clone_peek: clone_peek::<T>,
            set: set::<T>,
            field: field::<T>,
            display: display::<T>,
            eq: eq::<T>,
        }
    }
}

fn downcast<T: 'static>(value: &dyn Any) -> &T {
    value
        .downcast_ref()
        .expect("type hooks are only used with their own type")
}

fn clone_peek<T: Facet<'static> + Clone>(peek: Peek<'_, 'static>) -> Option<Box<dyn Any>> {
    let value: &T = peek.get().ok()?;
    Some(Box::new(value.clone()))
}

fn set<T: Facet<'static> + Clone>(partial: Builder, value: &dyn Any) -> RunResult<Builder> {
    partial
        .set(downcast::<T>(value).clone())
        .map_err(reflect_error)
}

fn field<T: Facet<'static>>(
    cx: &dyn ConvertCx,
    value: &dyn Any,
    name: &str,
) -> Option<RunResult<Value>> {
    let fields = Peek::new(downcast::<T>(value)).into_struct().ok()?;
    let field = fields.field_by_name(name).ok()?;
    Some(peek_to_value(cx, field))
}

/// Uses `Display`, or `Debug` if the type has no `Display`.
fn display<T: Facet<'static>>(value: &dyn Any) -> String {
    let peek = Peek::new(downcast::<T>(value));
    let displayed = peek.to_string();
    // Without a `Display`, a peek shows the type name as `⟨Type⟩`.
    if displayed == format!("⟨{}⟩", T::SHAPE) {
        format!("{peek:?}")
    } else {
        displayed
    }
}

fn eq<T: Facet<'static>>(a: &dyn Any, b: &dyn Any) -> bool {
    Peek::new(downcast::<T>(a)) == Peek::new(downcast::<T>(b))
}

struct ModuleData {
    name: String,
    children: Vec<usize>,
    functions: Vec<NativeFnDecl>,
    types: Vec<NativeTypeId>,
}

/// Every native function and type, organised into modules.
#[derive(Default)]
pub struct Registry {
    fns: Vec<NativeFn>,
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
        ModuleBuilder {
            registry: self,
            module,
        }
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
        (self.hooks(object.ty).field)(cx, &*object.borrow_any(), name)
    }

    /// Formats a native object with its `Display`, or else `Debug`.
    pub fn display(&self, object: &NativeObject) -> String {
        (self.hooks(object.ty).display)(&*object.borrow_any())
    }

    /// Whether two native objects are equal by their Rust `PartialEq`.
    pub fn native_eq(&self, a: &NativeObject, b: &NativeObject) -> bool {
        a.ty == b.ty && (self.hooks(a.ty).eq)(&*a.borrow_any(), &*b.borrow_any())
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
        (self.hooks(object.ty).set)(partial, &*object.borrow_any())
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
        NativeModule {
            name: data.name.clone(),
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

/// Adds items to one module of a [`Registry`].
pub struct ModuleBuilder<'r> {
    registry: &'r mut Registry,
    module: usize,
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
        f: impl Fn(&mut dyn Context, &[Value]) -> RunResult<Value> + 'static,
    ) -> &mut Self {
        let decl = self.registry.add_fn(name, None, Box::new(f));
        self.registry.modules[self.module].functions.push(decl);
        self
    }

    /// Registers the Rust type `T` as `name`. Values of `T` passed to Xur
    /// become native objects holding the Rust value.
    pub fn ty<T: Facet<'static> + Clone + 'static>(&mut self, name: &str) -> TypeBuilder<'_, T> {
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

/// Methods taking `&T`, for [`TypeBuilder::method`].
pub trait IntoNativeMethod<T, Marker> {
    fn into_native(self) -> (Signature, Box<CallFn>);
}

/// Methods taking `&mut T`, for [`TypeBuilder::method_mut`].
pub trait IntoNativeMethodMut<T, Marker> {
    fn into_native(self) -> (Signature, Box<CallFn>);
}

/// Functions that can be registered with [`ModuleBuilder::function`]: any
/// `Fn(A, B, ...) -> R` (up to six arguments) whose types implement
/// [`Facet`]. `Marker` only distinguishes the implementations.
pub trait IntoNativeFn<Marker> {
    fn into_native(self) -> (Signature, Box<CallFn>);
}

macro_rules! impl_into_native_fn {
    ($($arg:ident),*) => {
        impl<F, R, $($arg),*> IntoNativeFn<fn($($arg),*) -> R> for F
        where
            F: Fn($($arg),*) -> R + 'static,
            R: Facet<'static> + 'static,
            $($arg: Facet<'static>,)*
        {
            fn into_native(self) -> (Signature, Box<CallFn>) {
                let signature = Signature {
                    params: vec![$($arg::SHAPE),*],
                    ret: R::SHAPE,
                };
                let arity = signature.params.len();
                let call = move |cx: &mut dyn Context, args: &[Value]| {
                    #[allow(non_snake_case)]
                    let [$($arg),*] = args else {
                        return Err(RuntimeError::new(format!(
                            "expected {arity} argument(s), found {}",
                            args.len()
                        )));
                    };
                    let result = self($(from_value::<$arg>(cx, $arg)?),*);
                    into_value(cx, result)
                };
                (signature, Box::new(call))
            }
        }

        impl<F, T, R, $($arg),*> IntoNativeMethod<T, fn($($arg),*) -> R> for F
        where
            F: Fn(&T, $($arg),*) -> R + 'static,
            T: Facet<'static> + 'static,
            R: Facet<'static> + 'static,
            $($arg: Facet<'static>,)*
        {
            // Arguments are bound to variables named after their types.
            #[allow(non_snake_case)]
            fn into_native(self) -> (Signature, Box<CallFn>) {
                impl_into_native_fn!(@method self, T, R, borrow, [&], $($arg),*)
            }
        }

        impl<F, T, R, $($arg),*> IntoNativeMethodMut<T, fn($($arg),*) -> R> for F
        where
            F: Fn(&mut T, $($arg),*) -> R + 'static,
            T: Facet<'static> + 'static,
            R: Facet<'static> + 'static,
            $($arg: Facet<'static>,)*
        {
            // Arguments are bound to variables named after their types.
            #[allow(non_snake_case)]
            fn into_native(self) -> (Signature, Box<CallFn>) {
                impl_into_native_fn!(@method self, T, R, borrow_mut, [&mut], $($arg),*)
            }
        }
    };
    // The body shared by `&self` and `&mut self` methods, which differ only
    // in how the receiver is borrowed.
    (@method $f:ident, $ty:ident, $ret:ident, $borrow:ident, [$($recv:tt)*], $($arg:ident),*) => {{
        let signature = Signature {
            params: vec![$ty::SHAPE, $($arg::SHAPE),*],
            ret: $ret::SHAPE,
        };
        let arity = signature.params.len();
        let call = move |cx: &mut dyn Context, args: &[Value]| {
            #[allow(non_snake_case)]
            let [receiver, $($arg),*] = args else {
                return Err(RuntimeError::new(format!(
                    "expected {arity} argument(s), found {}",
                    args.len()
                )));
            };
            $(let $arg = from_value::<$arg>(cx, $arg)?;)*
            let Value::Native(object) = receiver else {
                return Err(RuntimeError::new(format!(
                    "expected `{}`, found {}",
                    $ty::SHAPE,
                    receiver.kind()
                )));
            };
            let result = {
                #[allow(unused_mut)]
                let mut this = object.$borrow::<$ty>().ok_or_else(|| {
                    RuntimeError::new(format!(
                        "expected `{}` that is not already in use",
                        $ty::SHAPE
                    ))
                })?;
                $f($($recv)* *this, $($arg),*)
            };
            into_value(cx, result)
        };
        (signature, Box::new(call) as Box<CallFn>)
    }};
}

impl_into_native_fn!();
impl_into_native_fn!(A);
impl_into_native_fn!(A, B);
impl_into_native_fn!(A, B, C);
impl_into_native_fn!(A, B, C, D);
impl_into_native_fn!(A, B, C, D, E);
impl_into_native_fn!(A, B, C, D, E, G);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{TestCx, fn_id};

    #[derive(Facet, Debug, Clone, PartialEq)]
    struct Point {
        x: f64,
        y: f64,
    }

    fn registry() -> Registry {
        let mut registry = Registry::new();
        registry
            .module("geo::shapes")
            .function("double", |x: i64| x * 2)
            .function("describe", |name: String, n: u8| format!("{name}#{n}"));
        registry
            .module("geo")
            .ty::<Point>("Point")
            .function("new", |x: f64, y: f64| Point { x, y })
            .method("norm", |p: &Point| p.x.hypot(p.y))
            .method_mut("scale", |p: &mut Point, k: f64| {
                p.x *= k;
                p.y *= k;
            });
        registry
    }

    #[test]
    fn decls_describe_the_module_tree() {
        let decls = registry().decls();
        assert_eq!(decls.len(), 1);
        let geo = &decls[0];
        assert_eq!(geo.name, "geo");
        assert_eq!(geo.modules[0].name, "shapes");
        assert_eq!(geo.modules[0].functions[1].name, "describe");
        assert_eq!(geo.types[0].name, "Point");
        assert_eq!(geo.types[0].functions[1].name, "norm");
        assert_eq!(geo.types[0].functions[2].name, "scale");
    }

    #[test]
    fn typed_functions_convert_arguments_and_results() {
        let registry = registry();
        let mut cx = TestCx::new(&registry);
        let double = fn_id(&registry, "geo::shapes::double");
        assert_eq!(cx.call(double, &[Value::Int(21)]), Ok(Value::Int(42)));
        let describe = fn_id(&registry, "geo::shapes::describe");
        assert_eq!(
            cx.call(describe, &[Value::str("p"), Value::Int(3)]),
            Ok(Value::str("p#3"))
        );
    }

    #[test]
    fn signatures_record_shapes() {
        let registry = registry();
        let signature = registry
            .function(fn_id(&registry, "geo::shapes::describe"))
            .signature
            .clone()
            .unwrap();
        assert_eq!(signature.params, [String::SHAPE, u8::SHAPE]);
        assert_eq!(signature.ret, String::SHAPE);
    }

    fn new_point(cx: &mut TestCx<'_>, x: f64, y: f64) -> Value {
        let new = fn_id(cx.registry, "geo::Point::new");
        cx.call(new, &[Value::Float(x), Value::Float(y)]).unwrap()
    }

    #[test]
    fn registered_types_are_native_objects() {
        let registry = registry();
        let mut cx = TestCx::new(&registry);
        let point = new_point(&mut cx, 3.0, 4.0);
        let Value::Native(object) = &point else {
            panic!("expected a native object, found {point:?}");
        };
        assert_eq!(*object.borrow::<Point>().unwrap(), Point { x: 3.0, y: 4.0 });
        assert_eq!(
            cx.call(fn_id(&registry, "geo::Point::norm"), &[point]),
            Ok(Value::Float(5.0))
        );
    }

    #[test]
    fn mutating_methods_change_the_object_in_place() {
        let registry = registry();
        let mut cx = TestCx::new(&registry);
        let point = new_point(&mut cx, 1.0, 2.0);
        let alias = point.clone();
        let scale = fn_id(&registry, "geo::Point::scale");
        assert_eq!(cx.call(scale, &[point, Value::Float(3.0)]), Ok(Value::Unit));
        let Value::Native(object) = &alias else {
            unreachable!()
        };
        assert_eq!(*object.borrow::<Point>().unwrap(), Point { x: 3.0, y: 6.0 });
    }

    #[test]
    fn native_fields_display_and_equality() {
        let registry = registry();
        let mut cx = TestCx::new(&registry);
        let (Value::Native(a), Value::Native(b)) =
            (new_point(&mut cx, 1.0, 2.0), new_point(&mut cx, 1.0, 2.0))
        else {
            unreachable!()
        };
        assert_eq!(registry.field(&cx, &a, "y"), Some(Ok(Value::Float(2.0))));
        assert_eq!(registry.field(&cx, &a, "z"), None);
        assert_eq!(registry.display(&a), "Point { x: 1.0, y: 2.0 }");
        assert!(registry.native_eq(&a, &b));
    }

    #[test]
    fn bad_calls_are_runtime_errors() {
        let registry = registry();
        let mut cx = TestCx::new(&registry);
        let double = fn_id(&registry, "geo::shapes::double");
        assert_eq!(
            cx.call(double, &[]).unwrap_err().message,
            "expected 1 argument(s), found 0"
        );
        assert_eq!(
            cx.call(double, &[Value::str("x")]).unwrap_err().message,
            "expected `i64`, found string"
        );
    }
}
