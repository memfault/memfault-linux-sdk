//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use crate::{Handler, Message, Service};

/// The `ShutdownServiceMessage` is supported by all services. It terminates the
/// thread. No other message in the queue will be processed.
/// The service will receive this message and can do something before shutting
/// down.
pub struct ShutdownServiceMessage {}
impl Message for ShutdownServiceMessage {
    type Reply = ();
}

impl<S: Service> Handler<ShutdownServiceMessage> for S {
    fn deliver(&mut self, _m: ShutdownServiceMessage) {}
}

/// The `PingMessage` is supported by all services. It allows the caller to
/// verify that the service is still running and that it has processed its
/// queue.
pub struct PingMessage {}
impl Message for PingMessage {
    type Reply = ();
}

impl<S: Service> Handler<PingMessage> for S {
    fn deliver(&mut self, _m: PingMessage) {}
}
