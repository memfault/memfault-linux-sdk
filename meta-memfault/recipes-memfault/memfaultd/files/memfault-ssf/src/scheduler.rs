//
// Copyright (c) Memfault, Inc.
// See License.txt for details
mod job;

use job::*;

use std::{
    collections::BinaryHeap,
    thread::{sleep, spawn, JoinHandle},
    time::{Duration, Instant},
};

use crate::{Handler, Mailbox, MailboxError, Message, Service};

/// The `Scheduler` is a tool to schedule sending specific messages at a given
/// interval.  It runs as its own thread.
pub struct Scheduler {
    schedule: BinaryHeap<Job>,
}

impl Scheduler {
    pub fn new() -> Self {
        Scheduler {
            schedule: BinaryHeap::new(),
        }
    }

    /// Schedule a new subscription. The `message` will be sent to the `mailbox` every  `period`.
    /// The return value (if any) is ignored.
    pub fn schedule_message_subscription<M: Message + Clone, S: Service + Handler<M> + 'static>(
        &mut self,
        message: M,
        mailbox: &Mailbox<S>,
        period: &Duration,
    ) {
        let task = DeliverMessageJobImpl::new(mailbox.clone(), message);
        let job = Job {
            next_run: Instant::now() + *period,
            period: *period,
            task: Box::new(task),
        };

        self.schedule.push(job);
    }

    /// Run the Scheduler on its own thread
    /// `on_error` will be called when one of the messages cannot be delivered to the service.
    pub fn run(mut self, on_error: Box<dyn Fn(MailboxError) + Send>) -> JoinHandle<()> {
        spawn(move || loop {
            if let Some(job) = self.schedule.pop() {
                while Instant::now() < job.next_run {
                    sleep(job.next_run - Instant::now());
                }
                if let Err(e) = job.task.execute() {
                    on_error(e);
                }

                self.schedule.push(Job {
                    next_run: job.next_run + job.period,
                    period: job.period,
                    task: job.task,
                })
            }
        })
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Scheduler::new()
    }
}

trait ScheduledTask: Send {
    fn execute(&self) -> Result<(), MailboxError>;
    fn prepare_next(&self) -> Box<dyn ScheduledTask>;
}

struct DeliverMessageJobImpl<S, M>
where
    S: Service,
    M: Message + Clone,
    S: Handler<M>,
{
    message: M,
    mailbox: Mailbox<S>,
}

impl<S, M> DeliverMessageJobImpl<S, M>
where
    S: Service,
    M: Message + Clone,
    S: Handler<M>,
{
    fn new(mailbox: Mailbox<S>, message: M) -> Self {
        Self { message, mailbox }
    }
}

impl<S, M> ScheduledTask for DeliverMessageJobImpl<S, M>
where
    S: Service + 'static,
    M: Message + Clone,
    S: Handler<M>,
{
    fn execute(&self) -> Result<(), MailboxError> {
        self.mailbox.send_and_wait_for_reply(self.message.clone())?;
        Ok(())
    }

    fn prepare_next(&self) -> Box<dyn ScheduledTask> {
        Box::new(Self {
            message: self.message.clone(),
            mailbox: self.mailbox.clone(),
        })
    }
}
