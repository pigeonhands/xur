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
