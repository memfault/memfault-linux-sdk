//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender};

use futures::future::{ready, BoxFuture};

use crate::{
    AsyncBoundedTaskMailbox, AsyncHandler, BoundedMailbox, BoundedTaskMailbox, Handler, Mailbox,
    MailboxError, Message, Service,
};

// This type alias includes a trait bound on the generic parameter which
// triggers clippy's `type_alias_bounds` lint. The bound is intentional to
// tie the alias to `Message` and to keep the surrounding mocks simple.
// Allow the lint here so the compiler doesn't warn in downstream builds.
#[allow(type_alias_bounds)]
type MockReplySender<M: Message> = Receiver<(M, Sender<M::Reply>)>;

/// A `MsgMailbox` only depends on the type of the messages it can contain.
///
/// This allows a real separation between the caller and the recipient, they do
/// not need to know about each other.
pub struct MsgMailbox<M: Message> {
    service_mailbox: Box<dyn MsgMailboxT<M>>,
}

impl<M: Message> MsgMailbox<M> {
    /// Create a mock msg mailbox. Messages will be kept in a Vec - Do not use this directly but use ServiceMock::new()
    pub(super) fn mock() -> (Self, MockReplySender<M>) {
        let (sender, receiver) = channel();
        let mock = MockMsgMailbox::new(sender);
        (
            MsgMailbox {
                service_mailbox: mock.duplicate(),
            },
            receiver,
        )
    }

    /// Create a bounded mock msg mailbox. Messages will be kept in a Vec - Do not use this directly but use ServiceMock::new()
    pub(super) fn bounded_mock(channel_size: usize) -> (Self, MockReplySender<M>) {
        let (sender, receiver) = sync_channel(channel_size);
        let mock = BoundedMockMsgMailbox::new(sender);
        (
            MsgMailbox {
                service_mailbox: mock.duplicate(),
            },
            receiver,
        )
    }

    pub fn send_and_forget(&self, message: M) -> Result<(), MailboxError> {
        self.service_mailbox.send_and_forget(message)
    }
    pub fn send_and_wait_for_reply(&self, message: M) -> Result<M::Reply, MailboxError> {
        self.service_mailbox.send_and_wait_for_reply(message)
    }

    pub async fn send_and_forget_async(&self, message: M) -> Result<(), MailboxError> {
        self.service_mailbox.send_and_forget_async(message).await
    }
}

impl<M: Message> Clone for MsgMailbox<M> {
    fn clone(&self) -> Self {
        MsgMailbox {
            service_mailbox: self.service_mailbox.duplicate(),
        }
    }
}

/// A `MsgMailbox` that will send to all services that have registered with it
///
/// This mailbox allows you to do a one-to-many mapping of mailboxes to services.
/// It's used in the case where we need to notiy several services of the same
/// message simultaneously.
#[derive(Clone)]
pub struct BroadcastMsgMailbox<M: Message + Clone> {
    msg_mailboxes: Vec<MsgMailbox<M>>,
}

impl<M: Message + Clone> BroadcastMsgMailbox<M> {
    pub fn send_and_forget(&self, message: M) -> Result<(), MailboxError> {
        self.msg_mailboxes
            .iter()
            .try_for_each(|mbox| mbox.send_and_forget(message.clone()))
    }

    pub fn send_and_wait_for_reply(&self, message: M) -> Result<Vec<M::Reply>, MailboxError> {
        self.msg_mailboxes
            .iter()
            .map(|mbox| mbox.send_and_wait_for_reply(message.clone()))
            .collect()
    }
}

impl<M: Message + Clone> From<Vec<MsgMailbox<M>>> for BroadcastMsgMailbox<M> {
    fn from(msg_mailboxes: Vec<MsgMailbox<M>>) -> Self {
        Self { msg_mailboxes }
    }
}

trait MsgMailboxT<M: Message>: Send + Sync {
    fn send_and_forget(&self, message: M) -> Result<(), MailboxError>;
    fn send_and_wait_for_reply(&self, message: M) -> Result<M::Reply, MailboxError>;

    /// No async reply-wait counterpart: every impl but one would have to block
    /// behind a resolved future, deadlocking if the target shares the caller's
    /// executor. Use `AsyncBoundedTaskMailbox` directly for that.
    fn send_and_forget_async(&self, message: M) -> BoxFuture<'_, Result<(), MailboxError>> {
        Box::pin(ready(self.send_and_forget(message)))
    }

    fn duplicate(&self) -> Box<dyn MsgMailboxT<M>>;
}

impl<M, S> MsgMailboxT<M> for Mailbox<S>
where
    S: Service + 'static,
    M: Message,
    S: Handler<M>,
{
    fn send_and_forget(&self, message: M) -> Result<(), MailboxError> {
        self.send_and_forget(message)
    }
    fn send_and_wait_for_reply(&self, message: M) -> Result<M::Reply, MailboxError> {
        self.send_and_wait_for_reply(message)
    }
    fn duplicate(&self) -> Box<dyn MsgMailboxT<M>> {
        Box::new(self.clone())
    }
}

impl<M, S> MsgMailboxT<M> for BoundedMailbox<S>
where
    S: Service + 'static,
    M: Message,
    S: Handler<M>,
{
    fn send_and_forget(&self, message: M) -> Result<(), MailboxError> {
        self.send_and_forget(message)
    }
    fn send_and_wait_for_reply(&self, message: M) -> Result<M::Reply, MailboxError> {
        self.send_and_wait_for_reply(message)
    }
    fn duplicate(&self) -> Box<dyn MsgMailboxT<M>> {
        Box::new(self.clone())
    }
}

impl<M, S> MsgMailboxT<M> for BoundedTaskMailbox<S>
where
    S: Service + 'static,
    M: Message,
    S: Handler<M>,
{
    fn send_and_forget(&self, message: M) -> Result<(), MailboxError> {
        self.send_and_forget(message)
    }
    fn send_and_wait_for_reply(&self, message: M) -> Result<M::Reply, MailboxError> {
        self.send_and_wait_for_reply(message)
    }
    fn duplicate(&self) -> Box<dyn MsgMailboxT<M>> {
        Box::new(self.clone())
    }
}

impl<M, S> MsgMailboxT<M> for AsyncBoundedTaskMailbox<S>
where
    S: Service + 'static,
    M: Message,
    S: AsyncHandler<M>,
{
    fn send_and_forget(&self, message: M) -> Result<(), MailboxError> {
        self.0.try_send_and_forget_async(message)
    }
    fn send_and_wait_for_reply(&self, message: M) -> Result<M::Reply, MailboxError> {
        self.0.blocking_send_and_wait_for_reply_async(message)
    }
    fn send_and_forget_async(&self, message: M) -> BoxFuture<'_, Result<(), MailboxError>> {
        Box::pin(self.0.send_and_forget_async(message))
    }
    fn duplicate(&self) -> Box<dyn MsgMailboxT<M>> {
        Box::new(self.clone())
    }
}

impl<M, S> From<Mailbox<S>> for MsgMailbox<M>
where
    M: Message,
    S: Service,
    S: Handler<M>,
    S: 'static,
{
    fn from(mailbox: Mailbox<S>) -> Self {
        MsgMailbox {
            service_mailbox: Box::new(mailbox),
        }
    }
}

impl<M, S> From<BoundedMailbox<S>> for MsgMailbox<M>
where
    M: Message,
    S: Service,
    S: Handler<M>,
    S: 'static,
{
    fn from(mailbox: BoundedMailbox<S>) -> Self {
        MsgMailbox {
            service_mailbox: Box::new(mailbox),
        }
    }
}

impl<M, S> From<BoundedTaskMailbox<S>> for MsgMailbox<M>
where
    M: Message,
    S: Service,
    S: Handler<M>,
    S: 'static,
{
    fn from(mailbox: BoundedTaskMailbox<S>) -> Self {
        MsgMailbox {
            service_mailbox: Box::new(mailbox),
        }
    }
}

impl<M, S> From<AsyncBoundedTaskMailbox<S>> for MsgMailbox<M>
where
    M: Message,
    S: Service,
    S: AsyncHandler<M>,
    S: 'static,
{
    fn from(mailbox: AsyncBoundedTaskMailbox<S>) -> Self {
        MsgMailbox {
            service_mailbox: Box::new(mailbox),
        }
    }
}

pub(super) struct MockMsgMailbox<M: Message> {
    sender: Sender<(M, Sender<M::Reply>)>,
}

impl<M: Message> MockMsgMailbox<M> {
    pub fn new(sender: Sender<(M, Sender<M::Reply>)>) -> Self {
        MockMsgMailbox { sender }
    }
}

impl<M: Message> MsgMailboxT<M> for MockMsgMailbox<M> {
    fn send_and_forget(&self, message: M) -> Result<(), MailboxError> {
        let (tx, _rx) = channel();
        if self.sender.send((message, tx)).is_err() {
            return Err(MailboxError::SendChannelClosed);
        }

        Ok(())
    }

    fn send_and_wait_for_reply(&self, message: M) -> Result<M::Reply, MailboxError> {
        let (tx, rx) = channel();

        if self.sender.send((message, tx)).is_err() {
            return Err(MailboxError::SendChannelClosed);
        }

        rx.recv().map_err(|_| MailboxError::NoResponse)
    }

    fn duplicate(&self) -> Box<dyn MsgMailboxT<M>> {
        Box::new(MockMsgMailbox {
            sender: self.sender.clone(),
        })
    }
}

pub(super) struct BoundedMockMsgMailbox<M: Message> {
    sender: SyncSender<(M, Sender<M::Reply>)>,
}

impl<M: Message> BoundedMockMsgMailbox<M> {
    pub fn new(sender: SyncSender<(M, Sender<M::Reply>)>) -> Self {
        BoundedMockMsgMailbox { sender }
    }
}

impl<M: Message> MsgMailboxT<M> for BoundedMockMsgMailbox<M> {
    fn send_and_forget(&self, message: M) -> Result<(), MailboxError> {
        let (tx, _rx) = channel();
        self.sender.try_send((message, tx)).map_err(|e| match e {
            std::sync::mpsc::TrySendError::Full(_) => MailboxError::SendChannelFull,
            std::sync::mpsc::TrySendError::Disconnected(_) => MailboxError::SendChannelClosed,
        })
    }

    fn send_and_wait_for_reply(&self, message: M) -> Result<M::Reply, MailboxError> {
        let (tx, rx) = channel();
        self.sender.try_send((message, tx)).map_err(|e| match e {
            std::sync::mpsc::TrySendError::Full(_) => MailboxError::SendChannelFull,
            std::sync::mpsc::TrySendError::Disconnected(_) => MailboxError::SendChannelClosed,
        })?;

        rx.recv().map_err(|_| MailboxError::NoResponse)
    }

    fn duplicate(&self) -> Box<dyn MsgMailboxT<M>> {
        Box::new(BoundedMockMsgMailbox {
            sender: self.sender.clone(),
        })
    }
}

#[cfg(test)]
mod test {
    use std::thread::spawn;

    use tokio::runtime::Builder;

    use super::*;
    use crate::ServiceJig;

    #[test]
    fn test_broadcast_mailbox() {
        let (mbox1, rx1) = MsgMailbox::<TestMessage>::mock();
        let (mbox2, rx2) = MsgMailbox::<TestMessage>::mock();

        let broadcast_mbox = BroadcastMsgMailbox::from(vec![mbox1, mbox2]);

        broadcast_mbox.send_and_forget(TestMessage).unwrap();

        assert!(rx1.try_recv().is_ok());
        assert!(rx2.try_recv().is_ok());
    }

    #[test]
    fn test_broadcast_mailbox_send_and_wait() {
        let (mbox1, rx1) = MsgMailbox::<TestMessage>::mock();
        let (mbox2, rx2) = MsgMailbox::<TestMessage>::mock();

        let broadcast_mbox = BroadcastMsgMailbox::from(vec![mbox1, mbox2]);

        let join_handle =
            spawn(move || broadcast_mbox.send_and_wait_for_reply(TestMessage).unwrap());

        let (_, reply_tx1) = rx1.recv().unwrap();
        reply_tx1.send(()).unwrap();
        let (_, reply_tx2) = rx2.recv().unwrap();
        reply_tx2.send(()).unwrap();

        let replies = join_handle.join().unwrap();
        assert_eq!(replies.len(), 2);
    }

    // `AsyncTestService` has no `Handler` impl, so arrival proves an
    // `AsyncEnvelope` was enqueued.
    #[test]
    fn test_erased_async_mailbox_awaits_and_delivers_via_async_handler() {
        let mut jig = ServiceJig::prepare(AsyncTestService::default());
        let mbox: MsgMailbox<TestMessage> = jig.mailbox.for_async().into();

        let runtime = Builder::new_current_thread().build().unwrap();
        runtime
            .block_on(mbox.send_and_forget_async(TestMessage))
            .unwrap();

        jig.process_all();
        assert_eq!(jig.get_service().delivered, 1);
    }

    #[test]
    fn test_sync_backed_mailbox_send_and_forget_async_resolves_immediately() {
        let (mbox, rx) = MsgMailbox::<TestMessage>::mock();

        let runtime = Builder::new_current_thread().build().unwrap();
        runtime
            .block_on(mbox.send_and_forget_async(TestMessage))
            .unwrap();

        assert!(rx.try_recv().is_ok());
    }

    #[derive(Default)]
    struct AsyncTestService {
        delivered: usize,
    }

    impl Service for AsyncTestService {
        fn name(&self) -> &str {
            "async_test_service"
        }
    }

    impl AsyncHandler<TestMessage> for AsyncTestService {
        async fn deliver_async(&mut self, _m: TestMessage) {
            self.delivered += 1;
        }
    }

    #[derive(Clone)]
    struct TestMessage;

    impl Message for TestMessage {
        type Reply = ();
    }
}
