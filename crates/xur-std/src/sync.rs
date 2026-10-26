//! `std::sync`: channels for sending values between threads.

use std::sync::{Arc, Mutex, mpsc};

use facet::Facet;
use xur_runtime::{NativeObject, Registry, RunResult, RuntimeError, Value, lock};

use crate::option;

/// The sending half of a channel. Copies of it send to the same receiver.
#[derive(Facet, Clone)]
#[facet(opaque)]
pub struct Sender(mpsc::Sender<Value>);

/// The receiving half of a channel.
#[derive(Facet, Clone)]
#[facet(opaque)]
pub struct Receiver(Arc<Mutex<mpsc::Receiver<Value>>>);

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::sync", |module| {
        module
            .function("channel", || {
                let (sender, receiver) = mpsc::channel();
                (Sender(sender), Receiver(Arc::new(Mutex::new(receiver))))
            })
            .ty::<Sender>("Sender")
            .raw_function("send", |cx, args| {
                let [Value::Native(object), value] = args else {
                    return Err(RuntimeError::new("`send` takes the value to send"));
                };
                let sent = with(object, |sender: &Sender| {
                    sender.0.send(value.clone()).is_ok()
                })?;
                let lang = cx.lang();
                Ok(match sent {
                    true => Value::adt(lang.ok, vec![Value::Unit]),
                    false => Value::adt(lang.err, vec![Value::str("the receiver is gone")]),
                })
            });
        module
            .ty::<Receiver>("Receiver")
            .raw_function("recv", |cx, args| {
                let object = receiver(args)?;
                let value = with(object, |receiver: &Receiver| lock(&receiver.0).recv().ok())?;
                Ok(option(cx, value))
            })
            .raw_function("try_recv", |cx, args| {
                let object = receiver(args)?;
                let value = with(object, |receiver: &Receiver| {
                    lock(&receiver.0).try_recv().ok()
                })?;
                Ok(option(cx, value))
            });
    });
}

fn receiver(args: &[Value]) -> RunResult<&NativeObject> {
    match args {
        [Value::Native(object)] => Ok(object),
        _ => Err(RuntimeError::new("expected a `Receiver` and no arguments")),
    }
}

fn with<T: 'static, R>(object: &NativeObject, f: impl FnOnce(&T) -> R) -> RunResult<R> {
    object
        .with(f)
        .ok_or_else(|| RuntimeError::new("expected a channel end"))
}
