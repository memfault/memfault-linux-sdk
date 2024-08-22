//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Simple Service Framework
//!
//! This library provides a simple abstraction to implement a service oriented
//! architecture in a non-async Rust program.
//!
//! Each service runs in its own thread with a processing loop waiting on a
//! channel.
//!
//! This library provides a few essential traits:
//!
//! - `Service`: a trait implemented by services
//! - `Message`: a trait implemented by messages
//! - `Handler<M: Message>`: a trait implemented by services that can handle
//! messages of type `M`.
//!
//! We provide some important structs to deploy the services:
//! - `ServiceThread`: will start and run a service inside a dedicated thread.
//! It returns a `Mailbox`.
//! - `Mailbox<S: Service>`: a lightweight (cheap to `clone()`) handle to send
//! messages to a thread.
//! - `Scheduler`: a utility thread which keeps a schedule of messages that need
//! to be sent at fixed intervals.
//!
//! As well as important testing utilities that are a big part of the value
//! provided by this framework:
//! - `ServiceJig`: a way to run a service inside a test without using threads.
//! The test can precisely decide when messages should be delivered and inspect
//! the state of the service at any time.
//! - `ServiceMock`: a service mock. Use this when you just need a place where
//! to send messages. Your test can then verify that the right messages were
//! sent to the mock.
//!

use std::any::Any;

mod envelope;
mod mailbox;
mod msg_mailbox;
mod scheduler;
mod service_jig;
mod service_mock;
mod service_thread;
mod shared_service_thread;
mod stats;
mod system_messages;

pub use envelope::*;
pub use mailbox::*;
pub use msg_mailbox::*;
pub use scheduler::*;
pub use service_jig::*;
pub use service_mock::*;
pub use service_thread::*;
pub use shared_service_thread::*;
pub use stats::*;
pub use system_messages::*;

/// All services should implement this trait. It guarantees that we will be able
/// to run the service inside a thread (it is `Send`).
pub trait Service: Send {
    fn name(&self) -> &str;
}

/// Any type that will be sent between services needs to implement this trait.
///
/// Sending a Reply is optional (use `type Reply = ()` if you will not send a reply).
pub trait Message: Send + Sync + Any + 'static {
    type Reply: Send;
}

/// Implement this trait to indicate that your service can process a specific message.
///
/// The `deliver()` method is passed a `&mut self` reference to the service,
/// making it very easy to update your state. You can optionally include a
/// reply.
pub trait Handler<M: Message> {
    fn deliver(&mut self, m: M) -> M::Reply;
}

/// Blanket implementation of Message for any Vec<M>. You lose the return value.
impl<M: Message> Message for Vec<M> {
    type Reply = ();
}

/// Blanket implementation of delivering a `Vec<Message>` to a `Handler<Message>`.
impl<M: Message, S: Handler<M>> Handler<Vec<M>> for S {
    fn deliver(&mut self, messages: Vec<M>) {
        for m in messages {
            self.deliver(m);
        }
    }
}
