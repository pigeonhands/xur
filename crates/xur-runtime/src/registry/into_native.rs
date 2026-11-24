//! Turning Rust functions into native functions, for each arity.

use facet::{Facet, Shape};

use super::{CallFn, Context, Signature};
use crate::marshal::{FromValue, IntoXurValue};
use crate::{RuntimeError, Value};

fn shape_or_unit(s: Option<&'static Shape>) -> &'static Shape {
    s.unwrap_or(<() as Facet>::SHAPE)
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
///
/// # Taking a registered native type as an argument
///
/// Each argument is marshaled with [`from_value::<Arg>`](crate::convert::from_value),
/// which matches a value against the argument type's *shape*. A registered
/// native type (`ty::<T>(..)`) must therefore be taken **by value** (`|x: T|`),
/// so its own registered shape resolves and the native fast-path applies — e.g.
/// `|tex: ManagedObject|`, `|obj: XurUnityObject|`.
///
/// Taking one **by reference** (`|x: &T|`) does *not* work: the `&T` reference
/// shape is not a registered type, so `from_value` skips the native path and the
/// call fails at runtime with `expected `&T`, found native object`. Only method
/// receivers (`&self` in [`IntoNativeMethod`]) borrow a native by reference,
/// because the receiver is pulled from the `NativeObject` directly (see the
/// `@method` arm below). Registered native types are typically cheap `Copy`
/// handles, so by-value is also the right default.
///
// TODO: support `&T` arguments for registered native types by borrowing the
// `NativeObject` in-scope for the call (as the `@method` arm does for `&self`),
// instead of routing `&T` through `from_value`. Would make the arg API
// symmetric with receivers; low priority since by-value already works.
pub trait IntoNativeFn<Marker> {
    fn into_native(self) -> (Signature, Box<CallFn>);
}

macro_rules! impl_into_native_fn {
    ($($arg:ident),*) => {
        impl<F, R, $($arg),*> IntoNativeFn<fn($($arg),*) -> R> for F
        where
            F: Fn($($arg),*) -> R + Send + Sync + 'static,
            R: IntoXurValue,
            $($arg: FromValue,)*
        {
            fn into_native(self) -> (Signature, Box<CallFn>) {
                let signature = Signature {
                    params: vec![$(shape_or_unit(<$arg as FromValue>::SHAPE)),*],
                    ret: shape_or_unit(<R as IntoXurValue>::SHAPE),
                    param_names: Vec::new(),
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
                    let result = self($(<$arg as FromValue>::from_value(cx, $arg)?),*);
                    result.into_xur_value(cx)
                };
                (signature, Box::new(call))
            }
        }

        impl<F, T, R, $($arg),*> IntoNativeMethod<T, fn($($arg),*) -> R> for F
        where
            F: Fn(&T, $($arg),*) -> R + Send + Sync + 'static,
            T: Facet<'static> + Send + 'static,
            R: IntoXurValue,
            $($arg: FromValue,)*
        {
            #[allow(non_snake_case)]
            fn into_native(self) -> (Signature, Box<CallFn>) {
                impl_into_native_fn!(@method self, T, R, with, $($arg),*)
            }
        }

        impl<F, T, R, $($arg),*> IntoNativeMethodMut<T, fn($($arg),*) -> R> for F
        where
            F: Fn(&mut T, $($arg),*) -> R + Send + Sync + 'static,
            T: Facet<'static> + Send + 'static,
            R: IntoXurValue,
            $($arg: FromValue,)*
        {
            #[allow(non_snake_case)]
            fn into_native(self) -> (Signature, Box<CallFn>) {
                impl_into_native_fn!(@method self, T, R, with_mut, $($arg),*)
            }
        }
    };
    (@method $f:ident, $ty:ident, $ret:ident, $with:ident, $($arg:ident),*) => {{
        let signature = Signature {
            params: vec![$ty::SHAPE, $(shape_or_unit(<$arg as FromValue>::SHAPE)),*],
            ret: shape_or_unit(<$ret as IntoXurValue>::SHAPE),
            param_names: Vec::new(),
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
            $(let $arg = <$arg as FromValue>::from_value(cx, $arg)?;)*
            let object = match receiver {
                Value::Native(object) => object,
                Value::NativeEnum(ne) => &ne.object,
                _ => {
                    return Err(RuntimeError::new(format!(
                        "expected `{}`, found {}",
                        $ty::SHAPE,
                        receiver.kind()
                    )));
                }
            };
            let result = object.$with(|this| $f(this, $($arg),*)).ok_or_else(|| {
                RuntimeError::new(format!("expected `{}`, found another native type", $ty::SHAPE))
            })?;
            result.into_xur_value(cx)
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

// ---------------------------------------------------------------------------
// Builtin-type methods: borrow the receiver from a Value without cloning.
// ---------------------------------------------------------------------------

use std::sync::Arc;
use crate::value::lock;

/// A type that can be borrowed from a [`Value`] variant without cloning.
///
/// Implementations exist for `[u8]` (from `Value::Bytes`) and `str`
/// (from `Value::Str`), letting `BuiltinBuilder::method` register
/// closures like `|bytes: &[u8], i: i64| -> Option<i64>` that work on
/// the data in place.
pub trait BuiltinRecv {
    /// Borrows `self` from `value` and passes it to `f`.
    fn with<R>(value: &Value, f: impl FnOnce(&Self) -> R) -> Result<R, RuntimeError>;
}

impl BuiltinRecv for [u8] {
    fn with<R>(value: &Value, f: impl FnOnce(&[u8]) -> R) -> Result<R, RuntimeError> {
        let Value::Bytes(bytes) = value else {
            return Err(RuntimeError::new(format!("expected bytes, found {}", value.kind())));
        };
        Ok(f(&lock(bytes)))
    }
}

impl BuiltinRecv for str {
    fn with<R>(value: &Value, f: impl FnOnce(&str) -> R) -> Result<R, RuntimeError> {
        let Value::Str(s) = value else {
            return Err(RuntimeError::new(format!("expected string, found {}", value.kind())));
        };
        Ok(f(s))
    }
}

impl BuiltinRecv for [Value] {
    fn with<R>(value: &Value, f: impl FnOnce(&[Value]) -> R) -> Result<R, RuntimeError> {
        let Value::List(list) = value else {
            return Err(RuntimeError::new(format!("expected list, found {}", value.kind())));
        };
        Ok(f(&lock(list)))
    }
}

impl BuiltinRecv for crate::XurList {
    fn with<R>(value: &Value, f: impl FnOnce(&Self) -> R) -> Result<R, RuntimeError> {
        let Value::List(list) = value else {
            return Err(RuntimeError::new(format!("expected list, found {}", value.kind())));
        };
        Ok(f(&crate::XurList(Arc::clone(list))))
    }
}

impl BuiltinRecv for crate::XurBytes {
    fn with<R>(value: &Value, f: impl FnOnce(&Self) -> R) -> Result<R, RuntimeError> {
        let Value::Bytes(bytes) = value else {
            return Err(RuntimeError::new(format!("expected bytes, found {}", value.kind())));
        };
        Ok(f(&crate::XurBytes(Arc::clone(bytes))))
    }
}

/// A type that can be mutably borrowed from a [`Value`] variant.
pub trait BuiltinRecvMut {
    fn with_mut<R>(value: &Value, f: impl FnOnce(&mut Self) -> R) -> Result<R, RuntimeError>;
}

impl BuiltinRecvMut for Vec<u8> {
    fn with_mut<R>(value: &Value, f: impl FnOnce(&mut Vec<u8>) -> R) -> Result<R, RuntimeError> {
        let Value::Bytes(bytes) = value else {
            return Err(RuntimeError::new(format!("expected bytes, found {}", value.kind())));
        };
        Ok(f(&mut lock(bytes)))
    }
}

impl BuiltinRecvMut for Vec<Value> {
    fn with_mut<R>(value: &Value, f: impl FnOnce(&mut Vec<Value>) -> R) -> Result<R, RuntimeError> {
        let Value::List(list) = value else {
            return Err(RuntimeError::new(format!("expected list, found {}", value.kind())));
        };
        Ok(f(&mut lock(list)))
    }
}

/// Methods taking `&Recv` where `Recv: BuiltinRecv`, for
/// [`BuiltinBuilder::method`].
pub trait IntoBuiltinMethod<Recv: ?Sized, Marker> {
    fn into_native(self) -> (Signature, Box<CallFn>);
}

/// Methods taking `&mut Recv` where `Recv: BuiltinRecvMut`, for
/// [`BuiltinBuilder::method_mut`].
pub trait IntoBuiltinMethodMut<Recv, Marker> {
    fn into_native(self) -> (Signature, Box<CallFn>);
}

macro_rules! impl_builtin_method {
    ($($arg:ident),*) => {
        impl<F, Recv, R, $($arg),*> IntoBuiltinMethod<Recv, fn($($arg),*) -> R> for F
        where
            F: Fn(&Recv, $($arg),*) -> R + Send + Sync + 'static,
            Recv: BuiltinRecv + ?Sized + 'static,
            R: IntoXurValue,
            $($arg: FromValue,)*
        {
            #[allow(non_snake_case)]
            fn into_native(self) -> (Signature, Box<CallFn>) {
                let signature = Signature {
                    params: vec![$(shape_or_unit(<$arg as FromValue>::SHAPE)),*],
                    ret: shape_or_unit(<R as IntoXurValue>::SHAPE),
                    param_names: Vec::new(),
                };
                let arity = signature.params.len() + 1;
                let call = move |cx: &mut dyn Context, args: &[Value]| {
                    #[allow(non_snake_case)]
                    let [receiver, $($arg),*] = args else {
                        return Err(RuntimeError::new(format!(
                            "expected {arity} argument(s), found {}",
                            args.len()
                        )));
                    };
                    $(let $arg = <$arg as FromValue>::from_value(cx, $arg)?;)*
                    let result = Recv::with(receiver, |recv| self(recv, $($arg),*))?;
                    result.into_xur_value(cx)
                };
                (signature, Box::new(call))
            }
        }

        impl<F, Recv, R, $($arg),*> IntoBuiltinMethodMut<Recv, fn($($arg),*) -> R> for F
        where
            F: Fn(&mut Recv, $($arg),*) -> R + Send + Sync + 'static,
            Recv: BuiltinRecvMut + 'static,
            R: IntoXurValue,
            $($arg: FromValue,)*
        {
            #[allow(non_snake_case)]
            fn into_native(self) -> (Signature, Box<CallFn>) {
                let signature = Signature {
                    params: vec![$(shape_or_unit(<$arg as FromValue>::SHAPE)),*],
                    ret: shape_or_unit(<R as IntoXurValue>::SHAPE),
                    param_names: Vec::new(),
                };
                let arity = signature.params.len() + 1;
                let call = move |cx: &mut dyn Context, args: &[Value]| {
                    #[allow(non_snake_case)]
                    let [receiver, $($arg),*] = args else {
                        return Err(RuntimeError::new(format!(
                            "expected {arity} argument(s), found {}",
                            args.len()
                        )));
                    };
                    $(let $arg = <$arg as FromValue>::from_value(cx, $arg)?;)*
                    let result = Recv::with_mut(receiver, |recv| self(recv, $($arg),*))?;
                    result.into_xur_value(cx)
                };
                (signature, Box::new(call))
            }
        }
    };
}

impl_builtin_method!();
impl_builtin_method!(A);
impl_builtin_method!(A, B);
impl_builtin_method!(A, B, C);
impl_builtin_method!(A, B, C, D);
impl_builtin_method!(A, B, C, D, E);
impl_builtin_method!(A, B, C, D, E, G);
