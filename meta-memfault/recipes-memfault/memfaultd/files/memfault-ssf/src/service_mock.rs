//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::sync::mpsc::Receiver;

use crate::{Message, MsgMailbox};

/// The ServiceMock allows you to mock a service processing messages of a specific type.
pub struct ServiceMock<M: Message> {
    pub mbox: MsgMailbox<M>,
    receiver: Receiver<M>,
}

impl<M: Message> ServiceMock<M> {
    pub fn new() -> Self {
        let (mbox, receiver) = MsgMailbox::mock();
        Self { mbox, receiver }
    }

    pub fn new_bounded(channel_size: usize) -> Self {
        let (mbox, receiver) = MsgMailbox::bounded_mock(channel_size);
        Self { mbox, receiver }
    }

    pub fn take_messages(&mut self) -> Vec<M> {
        self.receiver.try_iter().collect()
    }
}

impl<M: Message> Default for ServiceMock<M> {
    fn default() -> Self {
        Self::new()
    }
}
