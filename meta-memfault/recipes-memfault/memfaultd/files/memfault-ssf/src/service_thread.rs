//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    any::TypeId,
    sync::mpsc::Receiver,
    thread::{spawn, JoinHandle},
};

use crate::{BoundedMailbox, Envelope, Mailbox, Service, ShutdownServiceMessage, StatsAggregator};

/// Run a service inside a dedicated thread using a mpsc::channel to send/receive messages
pub struct ServiceThread<S: Service> {
    pub handle: JoinHandle<Result<StatsAggregator, &'static str>>,
    pub mailbox: Mailbox<S>,
}

impl<S: Service + 'static> ServiceThread<S> {
    pub fn spawn_with(service: S) -> Self {
        let (mailbox, receiver) = Mailbox::create();
        let handle = spawn(move || ServiceThread::run(service, receiver));

        ServiceThread { handle, mailbox }
    }

    pub fn mbox(&self) -> Mailbox<S> {
        self.mailbox.clone()
    }

    pub fn run(
        service: S,
        receiver: Receiver<Envelope<S>>,
    ) -> Result<StatsAggregator, &'static str> {
        run(service, receiver)
    }
}

pub struct BoundedServiceThread<S: Service> {
    pub handle: JoinHandle<Result<StatsAggregator, &'static str>>,
    pub mailbox: BoundedMailbox<S>,
}

impl<S: Service + 'static> BoundedServiceThread<S> {
    pub fn spawn_with(service: S, channel_size: usize) -> Self {
        let (mailbox, receiver) = BoundedMailbox::create(channel_size);
        let handle = spawn(move || ServiceThread::run(service, receiver));

        BoundedServiceThread { handle, mailbox }
    }

    pub fn mbox(&self) -> BoundedMailbox<S> {
        self.mailbox.clone()
    }

    pub fn run(
        service: S,
        receiver: Receiver<Envelope<S>>,
    ) -> Result<StatsAggregator, &'static str> {
        run(service, receiver)
    }
}

fn run<S: Service>(
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
