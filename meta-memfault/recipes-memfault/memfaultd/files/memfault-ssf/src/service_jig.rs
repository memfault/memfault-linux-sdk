//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use tokio::{
    runtime::{Builder, Runtime},
    sync::mpsc as tokio_mpsc,
};

use crate::{BoundedTaskMailbox, Service, TaskEnvelope};

const SERVICE_JIG_CHANNEL_SIZE: usize = 1024;

/// The ServiceJig allows you to create a mailbox for a service and control when
/// messages will be processed. It does not use thread and is specifically well
/// suited for unit tests.
pub struct ServiceJig<S: Service> {
    pub mailbox: BoundedTaskMailbox<S>,
    service: S,
    receiver: tokio_mpsc::Receiver<TaskEnvelope<S>>,
    runtime: Runtime,
}

impl<S: Service> ServiceJig<S> {
    pub fn prepare(service: S) -> Self {
        let (mailbox, receiver) = BoundedTaskMailbox::create(SERVICE_JIG_CHANNEL_SIZE);
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to build task runtime");

        ServiceJig {
            service,
            receiver,
            mailbox,
            runtime,
        }
    }

    /// Process all waiting messages
    pub fn process_all(&mut self) {
        while let Ok(envelope) = self.receiver.try_recv() {
            let _x = match envelope {
                TaskEnvelope::Sync(mut envelope) => envelope.deliver_to(&mut self.service),
                TaskEnvelope::Async(mut envelope) => self
                    .runtime
                    .block_on(envelope.deliver_to(&mut self.service)),
            };
        }
    }

    pub fn get_service(&self) -> &S {
        &self.service
    }

    pub fn get_service_mut(&mut self) -> &mut S {
        &mut self.service
    }
}
