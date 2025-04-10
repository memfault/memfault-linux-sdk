//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{thread::sleep, time::Duration};

use log::warn;

use crate::{
    BoundedMailbox, BoundedServiceThread, BoundedTaskMailbox, BoundedTaskServiceThread, Mailbox,
    MsgMailbox, Service, ServiceJoinHandle, ServiceJoinHandleError, ServiceThread,
    ShutdownServiceMessage, StatsAggregator, TaskService,
};

#[derive(Default)]
pub struct ServiceManager {
    shutdown_handles: Vec<ShutdownHandle>,
}

impl ServiceManager {
    const SHUTDOWN_TIMEOUT_MS: u64 = 1000;
    const SHUTDOWN_RETRIES: u64 = 10;
    const SHUTDOWN_LOOP_TIMEOUT_MS: Duration =
        Duration::from_millis(Self::SHUTDOWN_TIMEOUT_MS / Self::SHUTDOWN_RETRIES);

    pub fn new() -> Self {
        Self::default()
    }

    pub fn spawn_service_thread<S: Service + Send + 'static>(&mut self, service: S) -> Mailbox<S> {
        let service_thread = ServiceThread::spawn_with(service);

        let service_mailbox = service_thread.mailbox.clone();

        let shutdown_handle = ShutdownHandle::from(service_thread);
        self.shutdown_handles.push(shutdown_handle);

        service_mailbox
    }

    pub fn spawn_bounded_service_thread<S: Service + Send + 'static>(
        &mut self,
        service: S,
        channel_size: usize,
    ) -> BoundedMailbox<S> {
        let service_thread = BoundedServiceThread::spawn_with(service, channel_size);
        let service_mailbox = service_thread.mailbox.clone();

        let shutdown_handle = ShutdownHandle::from(service_thread);
        self.shutdown_handles.push(shutdown_handle);

        service_mailbox
    }

    pub fn spawn_bounded_task_service_thread_with_fn<
        S: TaskService + 'static,
        I: FnOnce() -> S + Send + 'static,
    >(
        &mut self,
        init_fn: I,
        channel_size: usize,
    ) -> BoundedTaskMailbox<S> {
        let service_thread = BoundedTaskServiceThread::spawn_with_init_fn(init_fn, channel_size);

        let service_mailbox = service_thread.mailbox.clone();

        let shutdown_handle = ShutdownHandle::from(service_thread);
        self.shutdown_handles.push(shutdown_handle);

        service_mailbox
    }

    pub fn stop(&mut self) -> Vec<StatsAggregator> {
        self.shutdown_handles
            .iter()
            .map(|handle| handle.mbox.send_and_forget(ShutdownServiceMessage {}))
            .filter_map(|res| res.err())
            .for_each(|e| warn!("Failed to shutdown service: {e}"));

        let mut join_handles = self
            .shutdown_handles
            .iter_mut()
            .map(|handle| &mut handle.join_handle)
            .collect::<Vec<_>>();
        let mut stats_aggregators = Vec::with_capacity(join_handles.len());
        for _ in 0..Self::SHUTDOWN_RETRIES {
            join_handles.retain_mut(|jh| match jh.try_join() {
                Ok(stats) => {
                    stats_aggregators.push(stats);
                    false
                }
                Err(ServiceJoinHandleError::ServiceFailed(msg)) => {
                    warn!("Service failed while stopping: {msg}");
                    false
                }
                Err(ServiceJoinHandleError::ServiceRunning) => true,
                Err(ServiceJoinHandleError::ServiceStopped) => false,
            });

            if join_handles.is_empty() {
                break;
            }

            sleep(Self::SHUTDOWN_LOOP_TIMEOUT_MS);
        }

        stats_aggregators
    }
}

pub struct ShutdownHandle {
    mbox: MsgMailbox<ShutdownServiceMessage>,
    join_handle: ServiceJoinHandle,
}

impl<S: Service + 'static> From<ServiceThread<S>> for ShutdownHandle {
    fn from(service: ServiceThread<S>) -> Self {
        Self {
            mbox: service.mailbox.into(),
            join_handle: service.join_handle,
        }
    }
}

impl<S: Service + 'static> From<BoundedServiceThread<S>> for ShutdownHandle {
    fn from(service: BoundedServiceThread<S>) -> Self {
        Self {
            mbox: service.mailbox.into(),
            join_handle: service.join_handle,
        }
    }
}

impl<S: TaskService + 'static> From<BoundedTaskServiceThread<S>> for ShutdownHandle {
    fn from(service: BoundedTaskServiceThread<S>) -> Self {
        Self {
            mbox: service.mailbox.into(),
            join_handle: service.join_handle,
        }
    }
}

#[cfg(test)]
mod test {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    use super::*;

    #[test]
    fn test_system_stop() {
        let mut system = ServiceManager::default();
        let service1_is_running = Arc::new(AtomicBool::new(true));
        let service2_is_running = Arc::new(AtomicBool::new(true));

        let test_service1 = TestService::new(service1_is_running.clone());
        let test_service2 = TestService::new(service2_is_running.clone());

        system.spawn_service_thread(test_service1);
        system.spawn_bounded_service_thread(test_service2, 128);

        system.stop();

        assert!(!service1_is_running.load(Ordering::SeqCst));
        assert!(!service2_is_running.load(Ordering::SeqCst));
    }

    struct TestService {
        is_running: Arc<AtomicBool>,
    }

    impl TestService {
        fn new(is_running: Arc<AtomicBool>) -> Self {
            Self { is_running }
        }
    }

    impl Service for TestService {
        fn name(&self) -> &str {
            "TestService"
        }
    }

    impl Drop for TestService {
        fn drop(&mut self) {
            self.is_running.store(false, Ordering::SeqCst);
        }
    }
}
