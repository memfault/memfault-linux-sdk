//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{any::TypeId, sync::mpsc::Receiver, thread::spawn};

use crate::{Envelope, Mailbox, Service, ShutdownServiceMessage, StatsAggregator};

/// Run a service inside a dedicated thread using a mpsc::channel to send/receive messages
pub struct ServiceThread<S: Service> {
    // Unused so far - handle: JoinHandle<()>,
    pub mailbox: Mailbox<S>,
}

impl<S: Service + 'static> ServiceThread<S> {
    pub fn spawn_with(service: S) -> Self {
        let (mailbox, receiver) = Mailbox::create();
        let _handle = spawn(move || ServiceThread::run(service, receiver));

        ServiceThread {
            /* handle, */ mailbox,
        }
    }

    pub fn mbox(&self) -> Mailbox<S> {
        self.mailbox.clone()
    }

    pub fn run(
        mut service: S,
        receiver: Receiver<Envelope<S>>,
    ) -> Result<StatsAggregator, &'static str> {
        let mut stats_aggregator = StatsAggregator::new();
        for mut envelope in receiver {
            let type_id = envelope.message_type_id();
            match envelope.deliver_to(&mut service) {
                Err(_e) => {
                    // Delivery failed - probably "attempt to deliver twice" - should never happen.
                    return Err("delivery failed");
                }
                Ok(stats) => {
                    stats_aggregator.add(&stats);
                }
            }
            if type_id == Some(TypeId::of::<ShutdownServiceMessage>()) {
                break;
            }
        }
        Ok(stats_aggregator)
    }
}
