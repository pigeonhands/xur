//! `std::sync`: channels for sending values between threads.

use std::sync::{Arc, Mutex, mpsc};

use facet::Facet;
use xur_runtime::{Opt, Registry, Value, lock};

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
            .method("send", |sender: &Sender, value: Value| -> Result<(), String> {
                sender
                    .0
                    .send(value)
                    .map_err(|_| "the receiver is gone".to_string())
            });
        module
            .ty::<Receiver>("Receiver")
            .method("recv", |receiver: &Receiver| -> Opt<Value> {
                Opt(lock(&receiver.0).recv().ok())
            })
            .method("try_recv", |receiver: &Receiver| -> Opt<Value> {
                Opt(lock(&receiver.0).try_recv().ok())
            });
    });
}
