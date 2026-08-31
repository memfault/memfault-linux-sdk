//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    sync::mpsc::{channel, Sender},
    thread::{sleep, spawn},
    time::Duration,
};

use futures::future::LocalBoxFuture;
use log::{error, warn};
use thiserror::Error;
use tokio::{
    runtime::Builder,
    sync::{mpsc as tokio_mpsc, watch},
    task::{spawn_local, LocalSet},
    time::{interval_at, Instant, MissedTickBehavior},
};

use crate::{
    service_thread::async_run, BoundedMailbox, BoundedServiceThread, BoundedTaskMailbox,
    BoundedTaskServiceThread, Mailbox, MailboxError, Message, MsgMailbox, Service,
    ServiceJoinHandle, ServiceJoinHandleError, ServiceThread, ShutdownServiceMessage,
    StatsAggregator, TaskEnvelope, TaskService,
};

const SHUTDOWN_TIMEOUT_MS: u64 = 1000;
const SHUTDOWN_RETRIES: u64 = 10;
const SHUTDOWN_LOOP_TIMEOUT: Duration =
    Duration::from_millis(SHUTDOWN_TIMEOUT_MS / SHUTDOWN_RETRIES);

#[derive(Error, Debug)]
pub enum ServiceManagerError {
    #[error("Failed to build service runtime: {0}")]
    RuntimeBuild(String),
    #[error("Service runtime thread stopped")]
    RuntimeStopped,
    #[error("Failed to initialize {service}: {reason}")]
    ServiceInit { service: String, reason: String },
}

type ServiceSpawner =
    Box<dyn FnOnce() -> LocalBoxFuture<'static, Result<(), ServiceManagerError>> + Send>;
type JobSpawner = Box<dyn FnOnce(watch::Receiver<bool>) -> LocalBoxFuture<'static, ()> + Send>;

/// Register task services, then run them all on one shared runtime thread.
///
/// Registration returns a usable mailbox immediately, so services can be wired
/// to each other before any of them is constructed. Each service is built and
/// initialized on the runtime thread, so a `TaskService` never needs to be
/// `Send`.
///
/// All task services share a single thread: a `run_task()` that blocks
/// synchronously will starve every other service.
///
/// Additionally allows you to register periodic message sending.
#[derive(Default)]
pub struct ServiceManagerBuilder {
    spawners: Vec<ServiceSpawner>,
    job_spawners: Vec<JobSpawner>,
    shutdown_handles: Vec<ShutdownHandle>,
}

impl ServiceManagerBuilder {
    /// Register a task service to be built and run later
    ///
    /// This allows you to register a service that will later be
    /// started and run on a shared current thread async executor.
    pub fn register_task_service<S: TaskService + Send + 'static>(
        &mut self,
        service: S,
        channel_size: usize,
    ) -> BoundedTaskMailbox<S> {
        self.register_task_service_with_fn(move || service, channel_size)
    }

    /// Register a task service with initialization.
    ///
    /// See `register_task_service` for functionality.
    ///
    /// This is useful for cases where
    pub fn register_task_service_with_fn<S, I>(
        &mut self,
        init_fn: I,
        channel_size: usize,
    ) -> BoundedTaskMailbox<S>
    where
        S: TaskService + 'static,
        I: FnOnce() -> S + Send + 'static,
    {
        let (mailbox, receiver) = BoundedTaskMailbox::create(channel_size);
        let (handle_tx, handle_rx) = channel();

        self.shutdown_handles.push(ShutdownHandle {
            mbox: mailbox.clone().into(),
            join_handle: ServiceJoinHandle::new(handle_rx),
        });

        self.spawners.push(Box::new(move || {
            Box::pin(init_and_spawn_task_service(init_fn, receiver, handle_tx))
        }));

        mailbox
    }

    /// Send `message` to `mailbox` every `period`, starting one `period` from
    /// when the manager is built.
    pub fn register_periodic_message<M, T>(&mut self, message: M, mailbox: T, period: Duration)
    where
        M: Message + Clone,
        T: Into<MsgMailbox<M>>,
    {
        let mailbox = mailbox.into();

        self.job_spawners.push(Box::new(move |shutdown| {
            Box::pin(run_job(message, mailbox, period, shutdown))
        }));
    }

    /// Construct all futures and start runtime
    pub fn build(self) -> Result<ServiceManager, ServiceManagerError> {
        let Self {
            spawners,
            job_spawners,
            shutdown_handles,
        } = self;
        let service_count = spawners.len();
        let (ready_tx, ready_rx) = channel();
        let (runtime_tx, runtime_rx) = channel();
        let (job_shutdown_tx, job_shutdown_rx) = watch::channel(false);

        spawn(move || {
            let runtime = match Builder::new_current_thread()
                .enable_io()
                .enable_time()
                .build()
            {
                Ok(runtime) => runtime,
                Err(e) => {
                    if runtime_tx
                        .send(Err(ServiceManagerError::RuntimeBuild(e.to_string())))
                        .is_err()
                    {
                        error!("Failed to report service runtime build failure: {e}");
                    }
                    return;
                }
            };

            if runtime_tx.send(Ok(())).is_err() {
                return;
            }

            let local_set = LocalSet::new();
            runtime.block_on(async move {
                local_set
                    .run_until(async move {
                        for spawner in spawners {
                            if ready_tx.send(spawner().await).is_err() {
                                return;
                            }
                        }

                        for job_spawner in job_spawners {
                            spawn_local(job_spawner(job_shutdown_rx.clone()));
                        }
                    })
                    .await;
                local_set.await;
            });
        });

        runtime_rx
            .recv()
            .map_err(|_| ServiceManagerError::RuntimeStopped)??;

        let mut manager = ServiceManager {
            shutdown_handles,
            job_shutdown_tx,
        };
        for _ in 0..service_count {
            let result = ready_rx
                .recv()
                .map_err(|_| ServiceManagerError::RuntimeStopped)
                .and_then(|result| result);

            if let Err(e) = result {
                manager.stop();
                return Err(e);
            }
        }

        Ok(manager)
    }
}

async fn init_and_spawn_task_service<S, I>(
    init_fn: I,
    receiver: tokio_mpsc::Receiver<TaskEnvelope<S>>,
    join_handle_tx: Sender<Result<StatsAggregator, &'static str>>,
) -> Result<(), ServiceManagerError>
where
    S: TaskService + 'static,
    I: FnOnce() -> S,
{
    let mut service = init_fn();

    if let Err(e) = service.init().await {
        return Err(ServiceManagerError::ServiceInit {
            service: service.name().to_string(),
            reason: e,
        });
    }

    spawn_local(async_run(service, receiver, join_handle_tx));

    Ok(())
}

/// Sends a message every period to the registered mailbox
async fn run_job<M: Message + Clone>(
    message: M,
    mailbox: MsgMailbox<M>,
    period: Duration,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut interval = interval_at(Instant::now() + period, period);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = interval.tick() => {
                match mailbox.send_and_forget_async(message.clone()).await {
                    Err(MailboxError::SendChannelClosed) => return,
                    Err(e) => warn!("Failed to send job message: {e}"),
                    Ok(()) => (),
                }
            }
            _ = shutdown.changed() => return,
        }
    }
}

pub struct ServiceManager {
    shutdown_handles: Vec<ShutdownHandle>,
    job_shutdown_tx: watch::Sender<bool>,
}

impl ServiceManager {
    pub fn stop(&mut self) -> Vec<StatsAggregator> {
        if self.job_shutdown_tx.send(true).is_err() {
            warn!("No jobs left to shut down");
        }

        stop_services(&mut self.shutdown_handles)
    }
}

#[derive(Default)]
pub struct ServiceManagerSync {
    shutdown_handles: Vec<ShutdownHandle>,
}

impl ServiceManagerSync {
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

    pub fn spawn_bounded_task_service_thread<S: TaskService + Send + 'static>(
        &mut self,
        service: S,
        channel_size: usize,
    ) -> BoundedTaskMailbox<S> {
        let service_thread = BoundedTaskServiceThread::spawn_with(service, channel_size);

        let service_mailbox = service_thread.mailbox.clone();

        let shutdown_handle = ShutdownHandle::from(service_thread);
        self.shutdown_handles.push(shutdown_handle);

        service_mailbox
    }

    pub fn stop(&mut self) -> Vec<StatsAggregator> {
        stop_services(&mut self.shutdown_handles)
    }
}

fn stop_services(shutdown_handles: &mut [ShutdownHandle]) -> Vec<StatsAggregator> {
    shutdown_handles
        .iter()
        .map(|handle| handle.mbox.send_and_forget(ShutdownServiceMessage {}))
        .filter_map(|res| res.err())
        .for_each(|e| warn!("Failed to shutdown service: {e}"));

    let mut join_handles = shutdown_handles
        .iter_mut()
        .map(|handle| &mut handle.join_handle)
        .collect::<Vec<_>>();
    let mut stats_aggregators = Vec::with_capacity(join_handles.len());
    for _ in 0..SHUTDOWN_RETRIES {
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

        sleep(SHUTDOWN_LOOP_TIMEOUT);
    }

    stats_aggregators
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
    use super::*;

    use std::{
        rc::Rc,
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc,
        },
    };

    use crate::{Handler, PingMessage};

    const JOB_PERIOD: Duration = Duration::from_millis(10);

    #[test]
    fn test_builder_starts_and_stops_task_service() {
        let flags = TestFlags::new();

        let mut builder = ServiceManagerBuilder::default();
        let service_flags = flags.clone();
        let mailbox = builder
            .register_task_service_with_fn(move || TestTaskService::new(service_flags, Ok(())), 16);

        let mut manager = builder.build().unwrap();

        assert!(flags.is_initialized.load(Ordering::SeqCst));
        assert!(mailbox.send_and_wait_for_reply(PingMessage {}).is_ok());

        manager.stop();

        assert!(!flags.is_running.load(Ordering::SeqCst));
    }

    #[test]
    fn test_builder_stops_all_registered_task_services() {
        let service1_flags = TestFlags::new();
        let service2_flags = TestFlags::new();

        let mut builder = ServiceManagerBuilder::default();
        let service1_init_flags = service1_flags.clone();
        builder.register_task_service_with_fn(
            move || TestTaskService::new(service1_init_flags, Ok(())),
            16,
        );
        builder.register_task_service(TestTaskService::new(service2_flags.clone(), Ok(())), 16);

        let mut manager = builder.build().unwrap();
        manager.stop();

        assert!(!service1_flags.is_running.load(Ordering::SeqCst));
        assert!(!service2_flags.is_running.load(Ordering::SeqCst));
    }

    #[test]
    fn test_builder_fails_when_init_fails() {
        let mut builder = ServiceManagerBuilder::default();
        builder.register_task_service_with_fn(
            move || TestTaskService::new(TestFlags::new(), Err("init failed".to_string())),
            16,
        );

        let error = match builder.build() {
            Ok(_) => panic!("build should have failed"),
            Err(error) => error,
        };

        assert!(
            matches!(
                &error,
                ServiceManagerError::ServiceInit { service, reason }
                    if service == "TestTaskService" && reason == "init failed"
            ),
            "{error:?}"
        );
        assert_eq!(
            error.to_string(),
            "Failed to initialize TestTaskService: init failed"
        );
    }

    #[test]
    fn test_registered_job_delivers_messages() {
        let flags = TestFlags::new();

        let mut builder = ServiceManagerBuilder::default();
        let service_flags = flags.clone();
        let mailbox = builder
            .register_task_service_with_fn(move || TestTaskService::new(service_flags, Ok(())), 16);
        builder.register_periodic_message(JobTick, mailbox, JOB_PERIOD);

        let mut manager = builder.build().unwrap();
        sleep(JOB_PERIOD * 10);
        manager.stop();

        assert!(flags.tick_count.load(Ordering::SeqCst) > 0);
    }

    #[test]
    fn test_job_does_not_tick_before_first_period() {
        let flags = TestFlags::new();

        let mut builder = ServiceManagerBuilder::default();
        let service_flags = flags.clone();
        let mailbox = builder
            .register_task_service_with_fn(move || TestTaskService::new(service_flags, Ok(())), 16);
        builder.register_periodic_message(JobTick, mailbox, Duration::from_secs(3600));

        let mut manager = builder.build().unwrap();
        manager.stop();

        assert_eq!(flags.tick_count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_jobs_stop_with_the_manager() {
        let flags = TestFlags::new();

        let mut builder = ServiceManagerBuilder::default();
        let service_flags = flags.clone();
        let mailbox = builder
            .register_task_service_with_fn(move || TestTaskService::new(service_flags, Ok(())), 16);
        builder.register_periodic_message(JobTick, mailbox, JOB_PERIOD);

        let mut manager = builder.build().unwrap();
        sleep(JOB_PERIOD * 2);
        manager.stop();

        let ticks_at_stop = flags.tick_count.load(Ordering::SeqCst);
        sleep(JOB_PERIOD * 10);

        assert_eq!(flags.tick_count.load(Ordering::SeqCst), ticks_at_stop);
    }

    #[test]
    fn test_builder_accepts_non_send_task_service() {
        let mut builder = ServiceManagerBuilder::default();
        builder.register_task_service_with_fn(
            || NotSendTaskService {
                _not_send: Rc::new(()),
            },
            16,
        );

        assert!(builder.build().is_ok());
    }

    #[derive(Clone)]
    struct TestFlags {
        is_running: Arc<AtomicBool>,
        is_initialized: Arc<AtomicBool>,
        tick_count: Arc<AtomicUsize>,
    }

    impl TestFlags {
        fn new() -> Self {
            Self {
                is_running: Arc::new(AtomicBool::new(true)),
                is_initialized: Arc::new(AtomicBool::new(false)),
                tick_count: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    #[derive(Clone)]
    struct JobTick;

    impl Message for JobTick {
        type Reply = ();
    }

    struct TestTaskService {
        flags: TestFlags,
        init_result: Result<(), String>,
    }

    impl TestTaskService {
        fn new(flags: TestFlags, init_result: Result<(), String>) -> Self {
            Self { flags, init_result }
        }
    }

    impl Service for TestTaskService {
        fn name(&self) -> &str {
            "TestTaskService"
        }
    }

    impl TaskService for TestTaskService {
        fn init(&mut self) -> LocalBoxFuture<'_, Result<(), String>> {
            Box::pin(async {
                self.flags.is_initialized.store(true, Ordering::SeqCst);
                self.init_result.clone()
            })
        }
    }

    impl Handler<JobTick> for TestTaskService {
        fn deliver(&mut self, _m: JobTick) {
            self.flags.tick_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl Drop for TestTaskService {
        fn drop(&mut self) {
            self.flags.is_running.store(false, Ordering::SeqCst);
        }
    }

    struct NotSendTaskService {
        _not_send: Rc<()>,
    }

    impl Service for NotSendTaskService {
        fn name(&self) -> &str {
            "NotSendTaskService"
        }
    }

    impl TaskService for NotSendTaskService {}

    #[test]
    fn test_system_stop() {
        let mut system = ServiceManagerSync::default();
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
