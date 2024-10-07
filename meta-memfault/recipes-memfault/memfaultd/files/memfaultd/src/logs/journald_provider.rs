//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use crate::logs::journald_parser::{Journal, JournalRaw, JournalRawImpl};

use eyre::{eyre, Result};
use log::{error, warn};

use std::thread::spawn;
use std::{
    path::PathBuf,
    sync::mpsc::{sync_channel, Receiver, SyncSender},
};

use super::log_entry::LogEntry;

const ENTRY_CHANNEL_SIZE: usize = 1024;

/// A log provider that reads log entries from Journald and sends them to a receiver.
pub struct JournaldLogProvider<J: JournalRaw> {
    journal: Journal<J>,
    entry_sender: SyncSender<LogEntry>,
}

impl<J: JournalRaw> JournaldLogProvider<J> {
    pub fn new(journal: J, entry_sender: SyncSender<LogEntry>) -> Self {
        Self {
            journal: Journal::new(journal),
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
            // TODO: Add support for filtering additional fields
            log_entry.filter_extra_fields(&[]);

            if let Err(e) = self.entry_sender.send(log_entry) {
                return Err(eyre!("Journald channel dropped: {}", e));
            }
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

/// A receiver for log entries from Journald.
pub struct JournaldLogReceiver {
    entry_receiver: Receiver<LogEntry>,
}

impl JournaldLogReceiver {
    pub fn new(entry_receiver: Receiver<LogEntry>) -> Self {
        Self { entry_receiver }
    }
}

impl Iterator for JournaldLogReceiver {
    type Item = LogEntry;

    fn next(&mut self) -> Option<Self::Item> {
        match self.entry_receiver.recv() {
            Ok(v) => Some(v),
            Err(e) => {
                warn!("Failed to receive entry: {}", e);
                None
            }
        }
    }
}

/// Start a Journald log provider and return a receiver for the log entries.
///
/// This function will start a new thread that reads log entries from Journald and sends them to the
/// returned receiver. It takes in the temporary storage path to use as the location of storing the
/// cursor file.
pub fn start_journald_provider(tmp_path: PathBuf) -> JournaldLogReceiver {
    let (entry_sender, entry_receiver) = sync_channel(ENTRY_CHANNEL_SIZE);

    spawn(move || {
        let journal_raw = JournalRawImpl::new(tmp_path);
        let mut provider = JournaldLogProvider::new(journal_raw, entry_sender);
        if let Err(e) = provider.start() {
            error!("Journald provider failed: {}", e);
        }
    });

    JournaldLogReceiver::new(entry_receiver)
}

#[cfg(test)]
mod test {
    use chrono::{DateTime, NaiveDateTime, Utc};
    use insta::{assert_json_snapshot, with_settings};
    use mockall::Sequence;

    use super::*;

    use crate::logs::journald_parser::{JournalEntryRaw, MockJournalRaw};

    #[test]
    fn test_happy_path() {
        let mut journal_raw = MockJournalRaw::new();
        let mut seq = Sequence::new();

        let (sender, receiver) = sync_channel(ENTRY_CHANNEL_SIZE);

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

        let mut provider = JournaldLogProvider::new(journal_raw, sender);

        assert!(provider.run_once().is_ok());
        let entry = receiver.try_recv().unwrap();
        with_settings!({sort_maps => true}, {
            assert_json_snapshot!(entry);
        });
    }

    #[test]
    fn test_channel_dropped() {
        let mut journal_raw = MockJournalRaw::new();
        let mut seq = Sequence::new();

        let (sender, receiver) = sync_channel(1);
        drop(receiver);

        journal_raw
            .expect_next_entry_available()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|| Ok(true));
        journal_raw
            .expect_get_entry_field_data()
            .returning(|| Ok(Some(raw_journal_entry())));

        let mut provider = JournaldLogProvider::new(journal_raw, sender);

        assert!(provider.run_once().is_err());
    }

    fn raw_journal_entry() -> JournalEntryRaw {
        let fields = [
            "_SYSTEMD_UNIT=user@1000.service",
            "MESSAGE=audit: type=1400 audit(1713462571.968:7508): apparmor=\"DENIED\" operation=\"open\" class=\"file\" profile=\"snap.firefox.firefox\" name=\"/etc/fstab\" pid=10122 comm=\"firefox\" requested_mask=\"r\" denied_mask=\"r\" fsuid=1000 ouid=0",
        ];

        let timestamp = NaiveDateTime::from_timestamp_millis(1337).unwrap();
        let timestamp = DateTime::<Utc>::from_utc(timestamp, Utc);

        JournalEntryRaw::new(fields.iter().map(|s| s.to_string()).collect(), timestamp)
    }
}
