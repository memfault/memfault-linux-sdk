//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use memfault_ssf::{self, Handler, MailboxError, Message, MsgMailbox, Service, ServiceJig};

struct MyService {
    count: usize,
}

impl Service for MyService {
    fn name(&self) -> &str {
        "MyService"
    }
}

struct MyMessage;
impl Message for MyMessage {
    type Reply = ();
}

impl Handler<MyMessage> for MyService {
    fn deliver(&mut self, _m: MyMessage) {
        self.count += 1;
    }
}

struct OtherService {
    mailbox: MsgMailbox<MyMessage>,
}

impl Service for OtherService {
    fn name(&self) -> &str {
        "OtherService"
    }
}

struct OtherMessage();
impl Message for OtherMessage {
    type Reply = Result<(), MailboxError>;
}

impl Handler<OtherMessage> for OtherService {
    fn deliver(&mut self, _m: OtherMessage) -> Result<(), MailboxError> {
        self.mailbox.send_and_forget(MyMessage)
    }
}

#[test]
fn message_mailbox() {
    let mut service = ServiceJig::prepare(MyService { count: 0 });
    let mut other = ServiceJig::prepare(OtherService {
        mailbox: service.mailbox.clone().into(),
    });

    other.mailbox.send_and_forget(OtherMessage()).unwrap();

    other.process_all();
    service.process_all();

    assert_eq!(service.get_service().count, 1);
}
