//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender};

use crate::{BoundedMailbox, BoundedTaskMailbox, Handler, Mailbox, MailboxError, Message, Service};

/// A `MsgMailbox` only depends on the type of the messages it can contain.
///
/// This allows a real separation between the caller and the recipient, they do
/// not need to know about each other.
pub struct MsgMailbox<M: Message> {
    service_mailbox: Box<dyn MsgMailboxT<M>>,
}

impl<M: Message> MsgMailbox<M> {
    /// Create a mock msg mailbox. Messages will be kept in a Vec - Do not use this directly but use ServiceMock::new()
    pub(super) fn mock() -> (Self, Receiver<M>) {
        let (sender, receiver) = channel();
        let mock = MockMsgMailbox::new(sender);
        (
            MsgMailbox {
                service_mailbox: mock.duplicate(),
            },
            receiver,
        )
    }

    /// Create a bounded mock msg mailbox. Messages will be kept in a Vec - Do not use this directly but use ServiceMock::new()
    pub(super) fn bounded_mock(channel_size: usize) -> (Self, Receiver<M>) {
        let (sender, receiver) = sync_channel(channel_size);
        let mock = BoundedMockMsgMailbox::new(sender);
        (
            MsgMailbox {
                service_mailbox: mock.duplicate(),
            },
            receiver,
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

trait MsgMailboxT<M: Message>: Send + Sync {
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

impl<M, S> MsgMailboxT<M> for BoundedMailbox<S>
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

impl<M, S> MsgMailboxT<M> for BoundedTaskMailbox<S>
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

impl<M, S> From<BoundedMailbox<S>> for MsgMailbox<M>
where
    M: Message,
    S: Service,
    S: Handler<M>,
    S: 'static,
{
    fn from(mailbox: BoundedMailbox<S>) -> Self {
        MsgMailbox {
            service_mailbox: Box::new(mailbox),
        }
    }
}

impl<M, S> From<BoundedTaskMailbox<S>> for MsgMailbox<M>
where
    M: Message,
    S: Service,
    S: Handler<M>,
    S: 'static,
{
    fn from(mailbox: BoundedTaskMailbox<S>) -> Self {
        MsgMailbox {
            service_mailbox: Box::new(mailbox),
        }
    }
}

pub(super) struct MockMsgMailbox<M> {
    sender: Sender<M>,
}

impl<M> MockMsgMailbox<M> {
    pub fn new(sender: Sender<M>) -> Self {
        MockMsgMailbox { sender }
    }
}

impl<M: Message> MsgMailboxT<M> for MockMsgMailbox<M> {
    fn send_and_forget(&self, message: M) -> Result<(), MailboxError> {
        if self.sender.send(message).is_err() {
            return Err(MailboxError::SendChannelClosed);
        }

        Ok(())
    }

    fn send_and_wait_for_reply(&self, _message: M) -> Result<M::Reply, MailboxError> {
        unimplemented!("We have not implemented send_and_wait_for_reply for MockMsgMailbox yet.")
    }

    fn duplicate(&self) -> Box<dyn MsgMailboxT<M>> {
        Box::new(MockMsgMailbox {
            sender: self.sender.clone(),
        })
    }
}

pub(super) struct BoundedMockMsgMailbox<M> {
    sender: SyncSender<M>,
}

impl<M> BoundedMockMsgMailbox<M> {
    pub fn new(sender: SyncSender<M>) -> Self {
        BoundedMockMsgMailbox { sender }
    }
}

impl<M: Message> MsgMailboxT<M> for BoundedMockMsgMailbox<M> {
    fn send_and_forget(&self, message: M) -> Result<(), MailboxError> {
        self.sender.try_send(message).map_err(|e| match e {
            std::sync::mpsc::TrySendError::Full(_) => MailboxError::SendChannelFull,
            std::sync::mpsc::TrySendError::Disconnected(_) => MailboxError::SendChannelClosed,
        })
    }

    fn send_and_wait_for_reply(&self, _message: M) -> Result<M::Reply, MailboxError> {
        unimplemented!(
            "We have not implemented send_and_wait_for_reply for BoundedMockMsgMailbox yet."
        )
    }

    fn duplicate(&self) -> Box<dyn MsgMailboxT<M>> {
        Box::new(BoundedMockMsgMailbox {
            sender: self.sender.clone(),
        })
    }
}
