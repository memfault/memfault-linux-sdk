//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::sync::MutexGuard;

use crate::{Message, MockMsgMailbox, MsgMailbox};

/// The ServiceMock allows you to mock a service processing messages of a specific type.
pub struct ServiceMock<M: Message> {
    pub mbox: MsgMailbox<M>,
    mock: MockMsgMailbox<M>,
}

impl<M: Message> ServiceMock<M> {
    pub fn new() -> Self {
        let (mbox, mock) = MsgMailbox::mock();
        Self { mbox, mock }
    }

    pub fn messages(&mut self) -> MutexGuard<Vec<M>> {
        self.mock.messages()
    }

    pub fn take_messages(&mut self) -> Vec<M> {
        self.mock.take_messages()
    }
}

impl<M: Message> Default for ServiceMock<M> {
    fn default() -> Self {
        Self::new()
    }
}
