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
//!     .function("new", |x: f64, y: f64| Point { x, y })
//!     .function("norm", |p: Point| p.x.hypot(p.y)); // `p.norm()` in Xur
//! ```
//!
//! Any `Fn(A, B, ...) -> R` whose argument and return types implement
//! [`Facet`] can be registered: arguments are converted from Xur values and
//! the result back with [`convert`](crate::convert). Functions that need the
//! running program itself, such as `std::import`, are registered with
//! [`ModuleBuilder::raw_function`] and receive a [`Context`].

use std::io::Write;

use facet::{Facet, Shape};
use xur_resolve::{NativeFnDecl, NativeFnId, NativeModule, NativeTypeDecl, NativeTypeId};

use crate::convert::{ConvertCx, from_value, to_value};
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

    /// Registers the Rust type `T` as `name`. Values of `T` returned to Xur
    /// are records whose methods are the type's functions.
    pub fn ty<T: Facet<'static>>(&mut self, name: &str) -> TypeBuilder<'_> {
        let id = NativeTypeId(self.registry.types.len() as u32);
        self.registry.types.push(NativeType {
            name: name.to_owned(),
            shape: T::SHAPE,
            functions: Vec::new(),
        });
        self.registry.modules[self.module].types.push(id);
        TypeBuilder {
            registry: self.registry,
            ty: id,
        }
    }
}

/// Adds functions to a registered native type.
pub struct TypeBuilder<'r> {
    registry: &'r mut Registry,
    ty: NativeTypeId,
}

impl TypeBuilder<'_> {
    /// Registers `Type::name`. If the function's first parameter is the
    /// type itself, it can also be called as a method: `value.name()`.
    pub fn function<Marker>(&mut self, name: &str, f: impl IntoNativeFn<Marker>) -> &mut Self {
        let (signature, call) = f.into_native();
        let decl = self.registry.add_fn(name, Some(signature), call);
        self.registry.types[self.ty.0 as usize].functions.push(decl);
        self
    }
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
            R: Facet<'static>,
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
                    to_value(cx, &result)
                };
                (signature, Box::new(call))
            }
        }
    };
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
    use crate::test_util::{TestCx, fn_id};

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
            .function("norm", |p: Point| p.x.hypot(p.y));
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

    #[test]
    fn native_types_round_trip_as_records() {
        let registry = registry();
        let mut cx = TestCx::new(&registry);
        let point = cx
            .call(
                fn_id(&registry, "geo::Point::new"),
                &[Value::Float(3.0), Value::Float(4.0)],
            )
            .unwrap();
        let Value::Record(record) = &point else {
            panic!("expected a record, found {point:?}");
        };
        assert_eq!(record.ty, registry.type_of_shape(Point::SHAPE));
        assert_eq!(
            cx.call(fn_id(&registry, "geo::Point::norm"), &[point]),
            Ok(Value::Float(5.0))
        );
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
