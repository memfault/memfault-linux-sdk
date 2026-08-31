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
//!
//! Additionally we add an `AsyncEnvelope<S>`, which provides all the same
//! mechanisms, but exposes async versions of the send methods.

use std::{
    any::TypeId,
    sync::mpsc::{channel, Receiver, Sender},
    time::Instant,
};

use futures::future::LocalBoxFuture;
use tokio::sync::oneshot;

use crate::{AsyncHandler, DeliveryStats, Handler, Message, Service};

/// Wrap a message that can be handled synchronously by `S`.
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

    pub fn deliver_to(&mut self, service: &mut S) -> Result<DeliveryStats, &'static str> {
        self.message.handle(service)
    }

    pub fn message_type_id(&self) -> Option<TypeId> {
        self.message.type_id()
    }
}

trait EnvelopeT<S: Service>: Send {
    fn type_id(&self) -> Option<TypeId>;
    fn handle(&mut self, service: &mut S) -> Result<DeliveryStats, &'static str>;
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

            let _error = self.ack_sender.send(r);
            Ok(DeliveryStats { queued, processing })
        } else {
            Err("Attempt to deliver multiple times")
        }
    }
}

/// Wrap a message that can only be handled asynchronously by `S`.
pub struct AsyncEnvelope<S> {
    message: Box<dyn AsyncEnvelopeT<S>>,
}

impl<S: Service> AsyncEnvelope<S> {
    pub fn wrap<M>(message: M) -> Self
    where
        M: Message,
        S: AsyncHandler<M>,
    {
        Self::wrap_with_reply(message).0
    }

    pub fn wrap_with_reply<M>(message: M) -> (Self, oneshot::Receiver<M::Reply>)
    where
        M: Message,
        S: AsyncHandler<M>,
    {
        let (ack_sender, ack_receiver) = oneshot::channel();
        (
            AsyncEnvelope {
                message: Box::new(AsyncEnvelopeTImpl {
                    timestamp: Instant::now(),
                    message: Some(message),
                    ack_sender: Some(ack_sender),
                }),
            },
            ack_receiver,
        )
    }

    pub fn deliver_to<'a>(
        &'a mut self,
        service: &'a mut S,
    ) -> LocalBoxFuture<'a, Result<DeliveryStats, &'static str>> {
        self.message.handle(service)
    }

    pub fn message_type_id(&self) -> Option<TypeId> {
        self.message.type_id()
    }
}

trait AsyncEnvelopeT<S: Service>: Send {
    fn type_id(&self) -> Option<TypeId>;
    fn handle<'a>(
        &'a mut self,
        service: &'a mut S,
    ) -> LocalBoxFuture<'a, Result<DeliveryStats, &'static str>>;
}
struct AsyncEnvelopeTImpl<M>
where
    M: Message,
{
    message: Option<M>,
    ack_sender: Option<oneshot::Sender<M::Reply>>,
    timestamp: Instant,
}
impl<S: Service + AsyncHandler<M>, M: Message> AsyncEnvelopeT<S> for AsyncEnvelopeTImpl<M> {
    fn type_id(&self) -> Option<TypeId> {
        self.message.as_ref().map(|m| m.type_id())
    }

    fn handle<'a>(
        &'a mut self,
        service: &'a mut S,
    ) -> LocalBoxFuture<'a, Result<DeliveryStats, &'static str>> {
        Box::pin(async move {
            if let Some(message) = self.message.take() {
                let processing_at = Instant::now();
                let r = service.deliver_async(message).await;

                let queued = processing_at - self.timestamp;
                let processing = Instant::now() - processing_at;

                if let Some(ack_sender) = self.ack_sender.take() {
                    let _error = ack_sender.send(r);
                }
                Ok(DeliveryStats { queued, processing })
            } else {
                Err("Attempt to deliver multiple times")
            }
        })
    }
}

/// Either a synchronous or asynchronous envelope, sent through a single
/// `BoundedTaskMailbox` channel and dispatched accordingly by `async_run`.
pub enum TaskEnvelope<S> {
    Sync(Envelope<S>),
    Async(AsyncEnvelope<S>),
}

impl<S: Service> TaskEnvelope<S> {
    pub fn message_type_id(&self) -> Option<TypeId> {
        match self {
            TaskEnvelope::Sync(envelope) => envelope.message_type_id(),
            TaskEnvelope::Async(envelope) => envelope.message_type_id(),
        }
    }
}

impl<S: Service> From<Envelope<S>> for TaskEnvelope<S> {
    fn from(envelope: Envelope<S>) -> Self {
        TaskEnvelope::Sync(envelope)
    }
}

impl<S: Service> From<AsyncEnvelope<S>> for TaskEnvelope<S> {
    fn from(envelope: AsyncEnvelope<S>) -> Self {
        TaskEnvelope::Async(envelope)
    }
}
