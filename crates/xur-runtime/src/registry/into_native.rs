//! Turning Rust functions into native functions, for each arity.

use facet::Facet;

use super::{CallFn, Context, Signature};
use crate::convert::{from_value, into_value};
use crate::{RuntimeError, Value};

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
            F: Fn($($arg),*) -> R + Send + Sync + 'static,
            R: Facet<'static> + Send + 'static,
            $($arg: Facet<'static>,)*
        {
            fn into_native(self) -> (Signature, Box<CallFn>) {
                let signature = Signature {
                    params: vec![$($arg::SHAPE),*],
                    ret: R::SHAPE,
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
                    let result = self($(from_value::<$arg>(cx, $arg)?),*);
                    into_value(cx, result)
                };
                (signature, Box::new(call))
            }
        }

        impl<F, T, R, $($arg),*> IntoNativeMethod<T, fn($($arg),*) -> R> for F
        where
            F: Fn(&T, $($arg),*) -> R + Send + Sync + 'static,
            T: Facet<'static> + Send + 'static,
            R: Facet<'static> + Send + 'static,
            $($arg: Facet<'static>,)*
        {
            // Arguments are bound to variables named after their types.
            #[allow(non_snake_case)]
            fn into_native(self) -> (Signature, Box<CallFn>) {
                impl_into_native_fn!(@method self, T, R, with, $($arg),*)
            }
        }

        impl<F, T, R, $($arg),*> IntoNativeMethodMut<T, fn($($arg),*) -> R> for F
        where
            F: Fn(&mut T, $($arg),*) -> R + Send + Sync + 'static,
            T: Facet<'static> + Send + 'static,
            R: Facet<'static> + Send + 'static,
            $($arg: Facet<'static>,)*
        {
            // Arguments are bound to variables named after their types.
            #[allow(non_snake_case)]
            fn into_native(self) -> (Signature, Box<CallFn>) {
                impl_into_native_fn!(@method self, T, R, with_mut, $($arg),*)
            }
        }
    };
    // The body shared by `&self` and `&mut self` methods, which differ only
    // in how the receiver is borrowed.
    (@method $f:ident, $ty:ident, $ret:ident, $with:ident, $($arg:ident),*) => {{
        let signature = Signature {
            params: vec![$ty::SHAPE, $($arg::SHAPE),*],
            ret: $ret::SHAPE,
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
            $(let $arg = from_value::<$arg>(cx, $arg)?;)*
            let Value::Native(object) = receiver else {
                return Err(RuntimeError::new(format!(
                    "expected `{}`, found {}",
                    $ty::SHAPE,
                    receiver.kind()
                )));
            };
            let result = object.$with(|this| $f(this, $($arg),*)).ok_or_else(|| {
                RuntimeError::new(format!("expected `{}`, found another native type", $ty::SHAPE))
            })?;
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

// ---------------------------------------------------------------------------
// Builtin-type methods: borrow the receiver from a Value without cloning.
// ---------------------------------------------------------------------------

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
            R: Facet<'static> + Send + 'static,
            $($arg: Facet<'static>,)*
        {
            #[allow(non_snake_case)]
            fn into_native(self) -> (Signature, Box<CallFn>) {
                let signature = Signature {
                    params: vec![$($arg::SHAPE),*],
                    ret: R::SHAPE,
                    param_names: Vec::new(),
                };
                let arity = signature.params.len() + 1; // +1 for receiver
                let call = move |cx: &mut dyn Context, args: &[Value]| {
                    #[allow(non_snake_case)]
                    let [receiver, $($arg),*] = args else {
                        return Err(RuntimeError::new(format!(
                            "expected {arity} argument(s), found {}",
                            args.len()
                        )));
                    };
                    $(let $arg = from_value::<$arg>(cx, $arg)?;)*
                    let result = Recv::with(receiver, |recv| self(recv, $($arg),*))?;
                    into_value(cx, result)
                };
                (signature, Box::new(call))
            }
        }

        impl<F, Recv, R, $($arg),*> IntoBuiltinMethodMut<Recv, fn($($arg),*) -> R> for F
        where
            F: Fn(&mut Recv, $($arg),*) -> R + Send + Sync + 'static,
            Recv: BuiltinRecvMut + 'static,
            R: Facet<'static> + Send + 'static,
            $($arg: Facet<'static>,)*
        {
            #[allow(non_snake_case)]
            fn into_native(self) -> (Signature, Box<CallFn>) {
                let signature = Signature {
                    params: vec![$($arg::SHAPE),*],
                    ret: R::SHAPE,
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
                    $(let $arg = from_value::<$arg>(cx, $arg)?;)*
                    let result = Recv::with_mut(receiver, |recv| self(recv, $($arg),*))?;
                    into_value(cx, result)
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
