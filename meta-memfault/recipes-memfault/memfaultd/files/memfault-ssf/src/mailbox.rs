//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    error::Error,
    fmt::Display,
    sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender, TrySendError},
};

use crate::{Envelope, Handler, Message, Service};

use tokio::sync::mpsc as tokio_mpsc;

/// The only reason for a message to fail to send is if the receiver channel is closed.
// An improvement would be to return the message back to the sender (the
// channel does it but after we wrap it in an envelope, it's complicated...)
#[derive(Debug)]
pub enum MailboxError {
    SendChannelClosed,
    NoResponse,
    SendChannelFull,
}

impl Display for MailboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "error sending message")
    }
}
impl Error for MailboxError {}

pub struct Mailbox<S: Service> {
    sender: Sender<Envelope<S>>,
}

impl<S: Service> Mailbox<S> {
    pub fn create() -> (Self, Receiver<Envelope<S>>) {
        let (sender, receiver) = channel();
        (Mailbox { sender }, receiver)
    }

    pub fn send_and_forget<M>(&self, message: M) -> Result<(), MailboxError>
    where
        M: Message,
        S: Handler<M>,
    {
        self.sender
            .send(Envelope::wrap(message))
            .map_err(|_e| MailboxError::SendChannelClosed)
    }

    pub fn send_and_wait_for_reply<M>(&self, message: M) -> Result<M::Reply, MailboxError>
    where
        M: Message,
        S: Handler<M>,
    {
        let (envelope, ack_receiver) = Envelope::wrap_with_reply(message);

        self.sender
            .send(envelope)
            .map_err(|_e| MailboxError::SendChannelClosed)?;

        ack_receiver.recv().map_err(|_e| MailboxError::NoResponse)
    }
}

impl<S: Service> Clone for Mailbox<S> {
    fn clone(&self) -> Self {
        Mailbox {
            sender: self.sender.clone(),
        }
    }
}

pub struct BoundedMailbox<S: Service> {
    sender: SyncSender<Envelope<S>>,
}

impl<S: Service> BoundedMailbox<S> {
    pub fn create(channel_size: usize) -> (Self, Receiver<Envelope<S>>) {
        let (sender, receiver) = sync_channel(channel_size);
        (BoundedMailbox { sender }, receiver)
    }

    pub fn send_and_forget<M>(&self, message: M) -> Result<(), MailboxError>
    where
        M: Message,
        S: Handler<M>,
    {
        self.sender
            .try_send(Envelope::wrap(message))
            .map_err(|e| match e {
                std::sync::mpsc::TrySendError::Full(_) => MailboxError::SendChannelFull,
                std::sync::mpsc::TrySendError::Disconnected(_) => MailboxError::SendChannelClosed,
            })
    }

    pub fn send_and_wait_for_reply<M>(&self, message: M) -> Result<M::Reply, MailboxError>
    where
        M: Message,
        S: Handler<M>,
    {
        let (envelope, ack_receiver) = Envelope::wrap_with_reply(message);

        self.sender.try_send(envelope).map_err(|e| match e {
            TrySendError::Full(_) => MailboxError::SendChannelFull,
            TrySendError::Disconnected(_) => MailboxError::SendChannelClosed,
        })?;

        ack_receiver.recv().map_err(|_e| MailboxError::NoResponse)
    }
}

impl<S: Service> Clone for BoundedMailbox<S> {
    fn clone(&self) -> Self {
        BoundedMailbox {
            sender: self.sender.clone(),
        }
    }
}

pub struct BoundedTaskMailbox<S: Service> {
    sender: tokio_mpsc::Sender<Envelope<S>>,
}

impl<S: Service> BoundedTaskMailbox<S> {
    pub fn create(channel_size: usize) -> (Self, tokio_mpsc::Receiver<Envelope<S>>) {
        let (sender, receiver) = tokio_mpsc::channel(channel_size);
        (BoundedTaskMailbox { sender }, receiver)
    }

    pub fn send_and_forget<M>(&self, message: M) -> Result<(), MailboxError>
    where
        M: Message,
        S: Handler<M>,
    {
        self.sender
            .try_send(Envelope::wrap(message))
            .map_err(|e| match e {
                tokio_mpsc::error::TrySendError::Full(_) => MailboxError::SendChannelFull,
                tokio_mpsc::error::TrySendError::Closed(_) => MailboxError::SendChannelClosed,
            })
    }

    pub fn send_and_wait_for_reply<M>(&self, message: M) -> Result<M::Reply, MailboxError>
    where
        M: Message,
        S: Handler<M>,
    {
        let (envelope, ack_receiver) = Envelope::wrap_with_reply(message);

        self.sender.try_send(envelope).map_err(|e| match e {
            tokio_mpsc::error::TrySendError::Full(_) => MailboxError::SendChannelFull,
            tokio_mpsc::error::TrySendError::Closed(_) => MailboxError::SendChannelClosed,
        })?;

        ack_receiver.recv().map_err(|_e| MailboxError::NoResponse)
    }
}

impl<S: Service> Clone for BoundedTaskMailbox<S> {
    fn clone(&self) -> Self {
        BoundedTaskMailbox {
            sender: self.sender.clone(),
        }
    }
}
