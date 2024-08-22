//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    error::Error,
    fmt::Display,
    sync::mpsc::{channel, Receiver, Sender},
};

use crate::{Envelope, Handler, Message, Service};

/// The only reason for a message to fail to send is if the receiver channel is closed.
// An improvement would be to return the message back to the sender (the
// channel does it but after we wrap it in an envelope, it's complicated...)
#[derive(Debug)]
pub enum MailboxError {
    SendChannelClosed,
    NoResponse,
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
