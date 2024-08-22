//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Provide a `struct Envelope<S>` that can be used to wrap messages of any type
//! M, as long as:
//! -  S is a service
//! -  S can handle the type M.
//!
//! Because the type `Envelope<S>` is only generic on the service, it enables
//! grouping together multiple messages of different types.
//!
//! This is the magic that makes it possible to deliver messages of multiple
//! unrelated types (they are not one enum) to services.
//!
//! The implementation relies on dynamic dispatch to an internal hidden type
//! that supports calling `envelope->handle(service)` (an inversion of
//! responsibility).

use std::{
    any::TypeId,
    sync::mpsc::{channel, Receiver, Sender},
    time::Instant,
};

use crate::{DeliveryStats, Handler, Message, Service};

/// Wrap a message that can be handled by `S`.
pub struct Envelope<S> {
    message: Box<dyn EnvelopeT<S>>,
}

impl<S: Service> Envelope<S> {
    pub fn wrap<M>(message: M) -> Self
    where
        M: Message,
        S: Handler<M>,
    {
        Self::wrap_with_reply(message).0
    }

    pub fn wrap_with_reply<M>(message: M) -> (Self, Receiver<M::Reply>)
    where
        M: Message,
        S: Handler<M>,
    {
        let (ack_sender, ack_receiver) = channel();
        (
            Envelope {
                message: Box::new(EnvelopeTImpl {
                    timestamp: Instant::now(),
                    message: Some(message),
                    ack_sender,
                }),
            },
            ack_receiver,
        )
    }

    pub fn deliver_to(&mut self, service: &mut S) -> Result<DeliveryStats, &str> {
        self.message.handle(service)
    }

    pub fn message_type_id(&self) -> Option<TypeId> {
        self.message.type_id()
    }
}

trait EnvelopeT<S: Service>: Send {
    fn type_id(&self) -> Option<TypeId>;
    fn handle(&mut self, service: &mut S) -> Result<DeliveryStats, &str>;
}
struct EnvelopeTImpl<M>
where
    M: Message,
{
    message: Option<M>,
    ack_sender: Sender<M::Reply>,
    timestamp: Instant,
}
impl<S: Service + Handler<M>, M: Message> EnvelopeT<S> for EnvelopeTImpl<M> {
    fn type_id(&self) -> Option<TypeId> {
        self.message.as_ref().map(|m| m.type_id())
    }

    fn handle(&mut self, service: &mut S) -> Result<DeliveryStats, &'static str> {
        if let Some(message) = self.message.take() {
            let processing_at = Instant::now();
            let r = service.deliver(message);

            let queued = processing_at - self.timestamp;
            let processing = Instant::now() - processing_at;

            // We ignore errors to deliver the ack as the caller might have moved on with their life.
            let _error = self.ack_sender.send(r);
            Ok(DeliveryStats { queued, processing })
        } else {
            Err("Attempt to deliver multiple times")
        }
    }
}
