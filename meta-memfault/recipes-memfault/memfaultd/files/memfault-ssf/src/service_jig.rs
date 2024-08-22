//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::sync::mpsc::Receiver;

use crate::{Envelope, Mailbox, Service};

/// The ServiceJig allows you to create a mailbox for a service and control when
/// messages will be processed. It does not use thread and is specifically well
/// suited for unit tests.
pub struct ServiceJig<S: Service> {
    pub mailbox: Mailbox<S>,
    service: S,
    receiver: Receiver<Envelope<S>>,
}

impl<S: Service> ServiceJig<S> {
    pub fn prepare(service: S) -> Self {
        let (mailbox, receiver) = Mailbox::create();

        ServiceJig {
            service,
            receiver,
            mailbox,
        }
    }

    /// Process all waiting messages
    pub fn process_all(&mut self) {
        let iter = self.receiver.try_iter();
        for mut m in iter {
            let _x = m.deliver_to(&mut self.service);
        }
    }

    pub fn get_service(&self) -> &S {
        &self.service
    }

    pub fn get_service_mut(&mut self) -> &mut S {
        &mut self.service
    }
}
