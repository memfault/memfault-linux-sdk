//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use crate::logs::journald_parser::{Journal, JournalRaw, JournalRawImpl};

use eyre::Result;
use log::error;
use ssf::MsgMailbox;

use std::path::PathBuf;
use std::thread::spawn;

use super::{log_collector::LogEntrySender, log_entry::LogEntry, messages::LogEntryMsg};

/// A log provider that reads log entries from Journald and sends them to a receiver.
pub struct JournaldLogProvider<J: JournalRaw> {
    journal: Journal<J>,
    extra_attr: Vec<String>,
    entry_sender: LogEntrySender,
}

impl<J: JournalRaw> JournaldLogProvider<J> {
    pub fn new(journal: J, entry_sender: MsgMailbox<LogEntryMsg>, extra_attr: Vec<String>) -> Self {
        let entry_sender = LogEntrySender::new(entry_sender);

        Self {
            journal: Journal::new(journal),
            extra_attr,
            entry_sender,
        }
    }

    fn run_once(&mut self) -> Result<()> {
        for entry in self
            .journal
            .iter()
            .filter(|e| e.fields.contains_key("MESSAGE"))
        {
            // We would only fail here if 'MESSAGE' is not present. Which we verified above.
            let mut log_entry = LogEntry::try_from(entry)?;
            log_entry.filter_extra_fields(&self.extra_attr);

            self.entry_sender.send_entry(log_entry)?;
        }
        Ok(())
    }

    pub fn start(&mut self) -> Result<()> {
        loop {
            // Block until another entry is available
            self.journal.wait_for_entry()?;

            self.run_once()?;
        }
    }
}

/// Start a Journald log provider and return a receiver for the log entries.
///
/// This function will start a new thread that reads log entries from Journald and sends them to the
/// returned receiver. It takes in the temporary storage path to use as the location of storing the
/// cursor file.
pub fn start_journald_provider(
    tmp_path: PathBuf,
    extra_attr: Vec<String>,
    entry_sender: MsgMailbox<LogEntryMsg>,
) {
    spawn(move || {
        let journal_raw = JournalRawImpl::new(tmp_path);
        let mut provider = JournaldLogProvider::new(journal_raw, entry_sender, extra_attr);
        if let Err(e) = provider.start() {
            error!("Journald provider failed: {}", e);
        }
    });
}

#[cfg(test)]
mod test {
    use chrono::{DateTime, NaiveDateTime, Utc};
    use insta::{assert_json_snapshot, with_settings};
    use mockall::Sequence;
    use rstest::rstest;
    use ssf::ServiceMock;

    use super::*;

    use crate::logs::journald_parser::{JournalEntryRaw, MockJournalRaw};

    #[rstest]
    #[case("no_extra_attr".to_string(), vec![])]
    #[case("extra_attr".to_string(), vec!["EXTRA_FIELD".to_string()])]
    fn test_happy_path(#[case] test_name: String, #[case] extra_attr: Vec<String>) {
        let mut journal_raw = MockJournalRaw::new();
        let mut seq = Sequence::new();

        let mut service = ServiceMock::new();

        journal_raw
            .expect_next_entry_available()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|| Ok(true));
        journal_raw
            .expect_get_entry_field_data()
            .returning(|| Ok(Some(raw_journal_entry())));
        journal_raw
            .expect_next_entry_available()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|| Ok(false));

        let mut provider = JournaldLogProvider::new(journal_raw, service.mbox.clone(), extra_attr);

        assert!(provider.run_once().is_ok());
        let entries = service.take_messages();
        assert_eq!(entries.len(), 1);
        with_settings!({sort_maps => true}, {
            assert_json_snapshot!(test_name, entries[0].entry);
        });
    }

    #[test]
    fn test_channel_dropped() {
        let mut journal_raw = MockJournalRaw::new();
        let mut seq = Sequence::new();

        let service = ServiceMock::new();
        let mbox = service.mbox.clone();
        drop(service);

        journal_raw
            .expect_next_entry_available()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|| Ok(true));
        journal_raw
            .expect_get_entry_field_data()
            .returning(|| Ok(Some(raw_journal_entry())));

        let mut provider = JournaldLogProvider::new(journal_raw, mbox, vec![]);

        assert!(provider.run_once().is_err());
    }

    fn raw_journal_entry() -> JournalEntryRaw {
        let fields = [
            "_SYSTEMD_UNIT=user@1000.service",
            "MESSAGE=audit: type=1400 audit(1713462571.968:7508): apparmor=\"DENIED\" operation=\"open\" class=\"file\" profile=\"snap.firefox.firefox\" name=\"/etc/fstab\" pid=10122 comm=\"firefox\" requested_mask=\"r\" denied_mask=\"r\" fsuid=1000 ouid=0",
            "EXTRA_FIELD=extra",
        ];

        let timestamp = NaiveDateTime::from_timestamp_millis(1337).unwrap();
        let timestamp = DateTime::<Utc>::from_utc(timestamp, Utc);

        JournalEntryRaw::new(fields.iter().map(|s| s.to_string()).collect(), timestamp)
    }
}
