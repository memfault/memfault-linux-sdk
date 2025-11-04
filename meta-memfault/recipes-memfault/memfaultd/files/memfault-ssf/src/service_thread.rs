//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    any::TypeId,
    sync::mpsc::{channel, Receiver, RecvError, Sender, TryRecvError},
    thread::spawn,
};

use log::{error, warn};
use tokio::runtime::Builder;
use tokio::sync::mpsc as tokio_mpsc;

use crate::{
    BoundedMailbox, BoundedTaskMailbox, Envelope, Mailbox, Service, ShutdownServiceMessage,
    StatsAggregator, TaskService,
};

/// Run a service inside a dedicated thread using a mpsc::channel to send/receive messages
pub struct ServiceThread<S: Service> {
    pub join_handle: ServiceJoinHandle,
    pub mailbox: Mailbox<S>,
}

impl<S: Service + Send + 'static> ServiceThread<S> {
    pub fn spawn_with(service: S) -> Self {
        let (mailbox, receiver) = Mailbox::create();
        let (handle_tx, handle_rx) = channel();
        let join_handle = ServiceJoinHandle::new(handle_rx);

        spawn(move || run(service, receiver, handle_tx));

        ServiceThread {
            join_handle,
            mailbox,
        }
    }

    pub fn mbox(&self) -> Mailbox<S> {
        self.mailbox.clone()
    }
}

impl<S: Service + 'static> ServiceThread<S> {
    pub fn spawn_with_init_fn<F: FnOnce() -> S + Send + 'static>(init_fn: F) -> Self {
        let (mailbox, receiver) = Mailbox::create();
        let (handle_tx, handle_rx) = channel();
        let join_handle = ServiceJoinHandle::new(handle_rx);

        spawn(move || {
            let service = init_fn();
            run(service, receiver, handle_tx)
        });

        ServiceThread {
            join_handle,
            mailbox,
        }
    }
}

pub struct BoundedServiceThread<S: Service> {
    pub join_handle: ServiceJoinHandle,
    pub mailbox: BoundedMailbox<S>,
}

impl<S: Service + Send + 'static> BoundedServiceThread<S> {
    pub fn spawn_with(service: S, channel_size: usize) -> Self {
        let (mailbox, receiver) = BoundedMailbox::create(channel_size);
        let (handle_tx, handle_rx) = channel();
        let join_handle = ServiceJoinHandle::new(handle_rx);

        spawn(move || run(service, receiver, handle_tx));

        BoundedServiceThread {
            join_handle,
            mailbox,
        }
    }

    pub fn mbox(&self) -> BoundedMailbox<S> {
        self.mailbox.clone()
    }
}

impl<S: Service + 'static> BoundedServiceThread<S> {
    pub fn spawn_with_init_fn<F: FnOnce() -> S + Send + 'static>(
        init_fn: F,
        channel_size: usize,
    ) -> Self {
        let (mailbox, receiver) = BoundedMailbox::create(channel_size);
        let (handle_tx, handle_rx) = channel();
        let join_handle = ServiceJoinHandle::new(handle_rx);

        spawn(move || {
            let service = init_fn();
            run(service, receiver, handle_tx)
        });

        BoundedServiceThread {
            join_handle,
            mailbox,
        }
    }
}

fn run<S: Service>(
    mut service: S,
    receiver: Receiver<Envelope<S>>,
    join_handle_tx: Sender<Result<StatsAggregator, &'static str>>,
) {
    let mut stats_aggregator = StatsAggregator::new();
    for mut envelope in receiver {
        let type_id = envelope.message_type_id();
        match envelope.deliver_to(&mut service) {
            Err(_e) => {
                // Delivery failed - probably "attempt to deliver twice" - should never happen.
                if let Err(e) = join_handle_tx.send(Err("Message delivery failed")) {
                    error!("ssf delivery failed: {e}");
                }
                return;
            }
            Ok(stats) => {
                stats_aggregator.add(&stats);
            }
        }
        if type_id == Some(TypeId::of::<ShutdownServiceMessage>()) {
            break;
        }
    }

    // drop service, and send message indicating the the service thread is closed
    drop(service);
    if let Err(e) = join_handle_tx.send(Ok(stats_aggregator)) {
        error!("ssf delivery failed: {e}");
    }
}

pub struct BoundedTaskServiceThread<S: TaskService> {
    pub join_handle: ServiceJoinHandle,
    pub mailbox: BoundedTaskMailbox<S>,
}

impl<S: TaskService + Send + 'static> BoundedTaskServiceThread<S> {
    pub fn spawn_with(service: S, channel_size: usize) -> Self {
        let (mailbox, receiver) = BoundedTaskMailbox::create(channel_size);
        let (handle_tx, handle_rx) = channel();
        let join_handle = ServiceJoinHandle::new(handle_rx);

        spawn(move || {
            let runtime = match Builder::new_current_thread().enable_io().build() {
                Ok(runtime) => runtime,
                Err(e) => {
                    error!("Failed to build task service runtime: {}", e);
                    if let Err(send_err) =
                        handle_tx.send(Err("Failed to start task service runtime"))
                    {
                        error!(
                            "Failed to send task service failure notification: {}",
                            send_err
                        );
                    }
                    return;
                }
            };
            runtime.block_on(async_run(service, receiver, handle_tx));
        });

        BoundedTaskServiceThread {
            join_handle,
            mailbox,
        }
    }

    pub fn mbox(&self) -> BoundedTaskMailbox<S> {
        self.mailbox.clone()
    }
}

impl<S: TaskService + 'static> BoundedTaskServiceThread<S> {
    pub fn spawn_with_init_fn<I>(init_fn: I, channel_size: usize) -> Self
    where
        I: FnOnce() -> S + Send + 'static,
    {
        let (mailbox, receiver) = BoundedTaskMailbox::create(channel_size);
        let (handle_tx, handle_rx) = channel();
        let join_handle = ServiceJoinHandle::new(handle_rx);

        spawn(move || {
            let service = init_fn();
            let runtime = Builder::new_current_thread().enable_io().build();
            match runtime {
                Ok(runtime) => runtime.block_on(async_run(service, receiver, handle_tx)),
                Err(e) => error!("Failed to spawn service: {}", e),
            }
        });

        BoundedTaskServiceThread {
            join_handle,
            mailbox,
        }
    }
}

async fn async_run<S>(
    mut service: S,
    mut receiver: tokio_mpsc::Receiver<Envelope<S>>,
    join_handle_tx: Sender<Result<StatsAggregator, &'static str>>,
) where
    S: TaskService,
{
    let mut stats_aggregator = StatsAggregator::new();

    if let Err(e) = service.init().await {
        error!("Failed to initialize task: {}", e);
        return;
    }

    loop {
        tokio::select! {
            Some(mut envelope) = receiver.recv() => {
                let type_id = envelope.message_type_id();
                match envelope.deliver_to(&mut service) {
                    Err(_e) => {
                        // Delivery failed - probably "attempt to deliver twice" - should never happen.
                        if let Err(e) = join_handle_tx.send(Err("Message delivery failed")) {
                            error!("ssf delivery failed: {e}");
                        }
                        return;
                    }
                    Ok(stats) => {
                        stats_aggregator.add(&stats);
                    }
                }
                if type_id == Some(TypeId::of::<ShutdownServiceMessage>()) {
                    break;
                }
            },
            result = service.run_task() => {
                if let Err(e) = result {
                    warn!("Service task failed: {}", e);
                }
            }
        };
    }

    // drop service, and send message indicating the the service thread is closed
    drop(service);
    if let Err(e) = join_handle_tx.send(Ok(stats_aggregator)) {
        error!("ssf delivery failed: {e}");
    }
}

pub struct ServiceJoinHandle {
    rx: Receiver<Result<StatsAggregator, &'static str>>,
}

impl ServiceJoinHandle {
    pub fn new(rx: Receiver<Result<StatsAggregator, &'static str>>) -> Self {
        Self { rx }
    }

    pub fn join(&mut self) -> Result<StatsAggregator, ServiceJoinHandleError> {
        self.rx
            .recv()?
            .map_err(ServiceJoinHandleError::ServiceFailed)
    }

    pub fn try_join(&mut self) -> Result<StatsAggregator, ServiceJoinHandleError> {
        self.rx
            .try_recv()?
            .map_err(ServiceJoinHandleError::ServiceFailed)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ServiceJoinHandleError {
    ServiceStopped,
    ServiceRunning,
    ServiceFailed(&'static str),
}

impl From<RecvError> for ServiceJoinHandleError {
    fn from(_value: RecvError) -> Self {
        // recv() can only fail if the sender is dropped
        Self::ServiceStopped
    }
}

impl From<TryRecvError> for ServiceJoinHandleError {
    fn from(value: TryRecvError) -> Self {
        match value {
            TryRecvError::Empty => Self::ServiceRunning,
            TryRecvError::Disconnected => Self::ServiceStopped,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_join_handle_error_conversion() {
        assert_eq!(
            ServiceJoinHandleError::from(RecvError),
            ServiceJoinHandleError::ServiceStopped
        );
        assert_eq!(
            ServiceJoinHandleError::from(TryRecvError::Empty),
            ServiceJoinHandleError::ServiceRunning
        );
        assert_eq!(
            ServiceJoinHandleError::from(TryRecvError::Disconnected),
            ServiceJoinHandleError::ServiceStopped
        );
    }

    #[test]
    fn test_try_join() {
        let (tx, rx) = channel();
        let mut join_handle = ServiceJoinHandle::new(rx);

        assert!(matches!(
            join_handle.try_join(),
            Err(ServiceJoinHandleError::ServiceRunning)
        ));

        tx.send(Ok(StatsAggregator::new())).unwrap();
        assert!(join_handle.try_join().is_ok());
    }

    #[test]
    fn test_join() {
        let (tx, rx) = channel();
        let mut join_handle = ServiceJoinHandle::new(rx);

        tx.send(Ok(StatsAggregator::new())).unwrap();

        assert!(join_handle.join().is_ok());
    }

    #[test]
    fn test_join_dropped() {
        let (tx, rx) = channel();
        let mut join_handle = ServiceJoinHandle::new(rx);

        drop(tx);
        assert!(matches!(
            join_handle.join(),
            Err(ServiceJoinHandleError::ServiceStopped)
        ));
    }
}
