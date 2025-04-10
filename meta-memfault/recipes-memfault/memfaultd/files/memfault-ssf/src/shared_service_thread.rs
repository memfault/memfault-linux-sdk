//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    any::TypeId,
    borrow::BorrowMut,
    sync::{mpsc::Receiver, Arc, Mutex},
    thread::spawn,
};

use crate::{Envelope, Mailbox, Service, ShutdownServiceMessage, StatsAggregator};

/// This runs a service into a thread but, unlike `ServiceThread`, it will use
/// an `Arc<Mutex<S>>` so that the service object is also available as shared
/// memory.
///
/// This is mostly here for backwards compatibility and to make adoption easier:
/// start by using `SharedServiceThread` and when all usage of the shared memory
/// have been removed, switch to `ServiceThread` for a "pure actor".
pub struct SharedServiceThread<S: Service> {
    mailbox: Mailbox<S>,
    service: Arc<Mutex<S>>,
}

impl<S: Service + Send + 'static> SharedServiceThread<S> {
    pub fn spawn_with(service: S) -> Self {
        let (mailbox, receiver) = Mailbox::create();
        let shared_service = Arc::new(Mutex::new(service));
        {
            let shared_service = shared_service.clone();
            let _handle = spawn(move || SharedServiceThread::run(shared_service, receiver));
        }

        SharedServiceThread {
            /* handle, */ mailbox,
            service: shared_service,
        }
    }

    pub fn mbox(&self) -> Mailbox<S> {
        self.mailbox.clone()
    }

    pub fn run(
        service: Arc<Mutex<S>>,
        receiver: Receiver<Envelope<S>>,
    ) -> Result<StatsAggregator, &'static str> {
        let mut stats_aggregator = StatsAggregator::new();
        for mut envelope in receiver {
            let type_id = envelope.message_type_id();
            match service.lock().borrow_mut() {
                Ok(service) => match envelope.deliver_to(service) {
                    Err(_e) => {
                        // Delivery failed - probably "attempt to deliver twice" - should never happen.
                        return Err("delivery failed");
                    }
                    Ok(stats) => {
                        stats_aggregator.add(&stats);
                    }
                },
                Err(_) => {
                    return Err("Shared mutex is poisoned. Shutting down.");
                }
            }
            if type_id == Some(TypeId::of::<ShutdownServiceMessage>()) {
                break;
            }
        }
        Ok(stats_aggregator)
    }

    pub fn shared(&self) -> Arc<Mutex<S>> {
        self.service.clone()
    }
}
