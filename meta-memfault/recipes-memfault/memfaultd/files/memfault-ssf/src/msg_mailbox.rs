//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    mem::take,
    sync::{Arc, Mutex, MutexGuard},
};

use crate::{Handler, Mailbox, MailboxError, Message, Service};

/// A `MsgMailbox` only depends on the type of the messages it can contain.
///
/// This allows a real separation between the caller and the recipient, they do
/// not need to know about each other.
pub struct MsgMailbox<M: Message> {
    service_mailbox: Box<dyn MsgMailboxT<M>>,
}

impl<M: Message> MsgMailbox<M> {
    /// Create a mock msg mailbox. Messages will be kept in a Vec - Do not use this directly but use ServiceMock::new()
    pub(super) fn mock() -> (Self, MockMsgMailbox<M>) {
        let mock = MockMsgMailbox::new();
        (
            MsgMailbox {
                service_mailbox: mock.duplicate(),
            },
            mock,
        )
    }

    pub fn send_and_forget(&self, message: M) -> Result<(), MailboxError> {
        self.service_mailbox.send_and_forget(message)
    }
    pub fn send_and_wait_for_reply(&self, message: M) -> Result<M::Reply, MailboxError> {
        self.service_mailbox.send_and_wait_for_reply(message)
    }
}

impl<M: Message> Clone for MsgMailbox<M> {
    fn clone(&self) -> Self {
        MsgMailbox {
            service_mailbox: self.service_mailbox.duplicate(),
        }
    }
}

trait MsgMailboxT<M: Message>: Send {
    fn send_and_forget(&self, message: M) -> Result<(), MailboxError>;
    fn send_and_wait_for_reply(&self, message: M) -> Result<M::Reply, MailboxError>;
    fn duplicate(&self) -> Box<dyn MsgMailboxT<M>>;
}

impl<M, S> MsgMailboxT<M> for Mailbox<S>
where
    S: Service + 'static,
    M: Message,
    S: Handler<M>,
{
    fn send_and_forget(&self, message: M) -> Result<(), MailboxError> {
        self.send_and_forget(message)
    }
    fn send_and_wait_for_reply(&self, message: M) -> Result<M::Reply, MailboxError> {
        self.send_and_wait_for_reply(message)
    }
    fn duplicate(&self) -> Box<dyn MsgMailboxT<M>> {
        Box::new(self.clone())
    }
}

impl<M, S> From<Mailbox<S>> for MsgMailbox<M>
where
    M: Message,
    S: Service,
    S: Handler<M>,
    S: 'static,
{
    fn from(mailbox: Mailbox<S>) -> Self {
        MsgMailbox {
            service_mailbox: Box::new(mailbox),
        }
    }
}

pub(super) struct MockMsgMailbox<M> {
    messages: Arc<Mutex<Vec<M>>>,
}

impl<M> MockMsgMailbox<M> {
    pub fn new() -> Self {
        MockMsgMailbox {
            messages: Arc::new(Mutex::new(vec![])),
        }
    }

    pub fn messages(&mut self) -> MutexGuard<'_, Vec<M>> {
        self.messages.lock().expect("Mutex poisoned")
    }

    pub fn take_messages(&mut self) -> Vec<M> {
        take(&mut self.messages.lock().expect("Mutex poisoned"))
    }
}

impl<M: Message> MsgMailboxT<M> for MockMsgMailbox<M> {
    fn send_and_forget(&self, message: M) -> Result<(), MailboxError> {
        self.messages
            .lock()
            .expect("cant lock msgmailbox queue")
            .push(message);
        Ok(())
    }

    fn send_and_wait_for_reply(&self, _message: M) -> Result<M::Reply, MailboxError> {
        unimplemented!("We have not implemented send_and_wait_for_reply for MockMsgMailbox yet.")
    }

    fn duplicate(&self) -> Box<dyn MsgMailboxT<M>> {
        Box::new(MockMsgMailbox {
            messages: self.messages.clone(),
        })
    }
}
