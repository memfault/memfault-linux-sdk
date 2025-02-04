//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use chrono::{DateTime, NaiveDateTime, Utc};
use eyre::{eyre, Error, Result};
use libc::free;
use nix::poll::{poll, PollFd};
use serde::Serialize;
use std::ffi::CString;
use std::fs::read_to_string;
use std::{collections::HashMap, path::PathBuf};
use std::{ffi::c_char, mem::MaybeUninit};

use log::{debug, warn};
use memfaultc_sys::systemd::{
    sd_journal, sd_journal_add_match, sd_journal_enumerate_data, sd_journal_get_cursor,
    sd_journal_get_fd, sd_journal_get_realtime_usec, sd_journal_next, sd_journal_open,
    sd_journal_process, sd_journal_seek_cursor,
};

use super::log_entry::{LogData, LogEntry, LogValue};
use crate::util::system::read_system_boot_id;
/// A trait for reading journal entries from the systemd journal.
///
/// This trait is used to abstract the raw journal entry reading logic from the rest of the codebase.
/// This allows for easier testing and mocking of the journal reading logic.
#[cfg_attr(test, mockall::automock)]
pub trait JournalRaw {
    /// Check if the next journal entry is available.
    ///
    /// Returns `Ok(true)` if the next journal entry is available, `Ok(false)` if there are no more entries,
    /// and an error if there was an issue reading the journal.
    fn next_entry_available(&mut self) -> Result<bool>;

    /// Get the raw field data of the current journal entry.
    ///
    /// Returns the raw string representing the key-value pairs of the journal entry, or `None` if there are no more entries.
    /// Returns an error if there was an issue reading the journal.
    fn get_entry_field_data(&mut self) -> Result<Option<JournalEntryRaw>>;

    /// Waits for the next journal entry to be available.
    ///
    /// This method should block until the next journal entry is available.
    fn wait_for_entry(&mut self) -> Result<()>;
}

/// Raw journal entry data.
///
/// This struct represents the raw data of a journal entry. It can be converted into a `JournalEntry` struct,
/// which contains the parsed key-value pairs of the journal entry.
#[derive(Debug)]
pub struct JournalEntryRaw {
    pub ts: DateTime<Utc>,
    pub fields: Vec<String>,
}

impl JournalEntryRaw {
    pub fn new(fields: Vec<String>, ts: DateTime<Utc>) -> Self {
        Self { ts, fields }
    }
}

/// An implementation of the `JournalRaw` trait that reads journal entries from the systemd journal.
///
/// This struct is used to read journal entries from the systemd journal. It relies on ffi calls into
/// libsystemd.
pub struct JournalRawImpl {
    journal: *mut sd_journal,
    wait_fd: PollFd,
    cursor_file: PathBuf,
}

impl JournalRawImpl {
    /// Timeout journal polling after 1 minute.
    const POLL_TIMEOUT_MS: i32 = 1000 * 60;
    const JOURNAL_CURSOR_FILE: &str = "JOURNALD_CURSOR";

    pub fn new(tmp_path: PathBuf) -> Self {
        let mut journal = std::ptr::null_mut();
        let cursor_file = tmp_path.join(Self::JOURNAL_CURSOR_FILE);
        let cursor_string = read_to_string(&cursor_file).ok();

        unsafe {
            sd_journal_open(&mut journal, 0);
        }

        let fd = unsafe { sd_journal_get_fd(journal) };
        let wait_fd = PollFd::new(fd, nix::poll::PollFlags::POLLIN);

        if let Some(cursor) = cursor_string {
            let cursor = cursor.trim();
            let ret = unsafe { sd_journal_seek_cursor(journal, cursor.as_ptr() as *const c_char) };
            if ret < 0 {
                warn!("Failed to seek journal to cursor: {}", ret);
            }
        } else if let Err(e) = Self::seek_to_current_boot_start(journal) {
            warn!("Couldn't seek journal to start of current boot: {}", e);
        }

        Self {
            journal,
            wait_fd,
            cursor_file,
        }
    }

    /// Seeks the journal to the start of the current boot's logs
    ///
    /// Returns OK if the journal is now set up to return the first log
    /// from the current boot
    /// Returns an error if the function could not confirm the next
    /// entry will be the start of the current boot
    fn seek_to_current_boot_start(journal: *mut sd_journal) -> Result<()> {
        let boot_id = read_system_boot_id()?;
        let boot_id_match = CString::new(format!("_BOOT_ID={}", boot_id.as_simple()))?;

        let ret =
            unsafe { sd_journal_add_match(journal, boot_id_match.as_ptr() as *const c_char, 0) };
        match ret {
            ret if ret < 0 => Err(eyre!(
                "Failed to add match on current boot ID to journal: {}",
                ret
            )),
            0 => Ok(()),
            _ => Ok(()),
        }
    }

    /// Save the current journal cursor to a file.
    ///
    /// This method saves the current journal cursor to a file so that the journal can be resumed from the
    /// same point after a restart.
    fn save_cursor(&self) {
        let mut cursor: MaybeUninit<*const c_char> = MaybeUninit::uninit();
        let ret = unsafe { sd_journal_get_cursor(self.journal, cursor.as_mut_ptr()) };
        if ret < 0 {
            warn!("Failed to get journal cursor: {}", ret);
        } else {
            let cursor = unsafe { cursor.assume_init() };
            let cursor_cstr = unsafe { std::ffi::CStr::from_ptr(cursor) };
            let cursor_str = cursor_cstr.to_str().unwrap_or_default();

            let write_result = std::fs::write(&self.cursor_file, cursor_str);
            unsafe {
                free(cursor as *mut libc::c_void);
            }
            match write_result {
                Ok(_) => (),
                Err(e) => warn!(
                    "Failed to write journal cursor to {:?}: {}",
                    self.cursor_file, e
                ),
            }
        }
    }

    pub fn get_timestamp(&self) -> Result<DateTime<Utc>> {
        let mut timestamp = 0u64;
        let ret = unsafe { sd_journal_get_realtime_usec(self.journal, &mut timestamp) };
        if ret < 0 {
            return Err(eyre!("Failed to get journal entry timestamp: {}", ret));
        }

        let datetime = NaiveDateTime::from_timestamp_micros(timestamp as i64)
            .ok_or_else(|| eyre!("Failed to convert journal timestamp to DateTime"))?;

        Ok(DateTime::<Utc>::from_utc(datetime, Utc))
    }
}

impl Drop for JournalRawImpl {
    fn drop(&mut self) {
        self.save_cursor();
        unsafe {
            libc::free(self.journal as *mut libc::c_void);
        }
    }
}

impl JournalRaw for JournalRawImpl {
    fn next_entry_available(&mut self) -> Result<bool> {
        let ret = unsafe { sd_journal_next(self.journal) };
        match ret {
            ret if ret < 0 => Err(eyre!("Failed to get next journal entry: {}", ret)),
            0 => Ok(false),
            _ => Ok(true),
        }
    }

    fn get_entry_field_data(&mut self) -> Result<Option<JournalEntryRaw>> {
        let mut data = MaybeUninit::uninit();
        let mut data_len = MaybeUninit::uninit();
        let mut fields = Vec::new();

        let timestamp = self.get_timestamp().unwrap_or_else(|e| {
            debug!(
                "Failed to get journal entry timestamp, falling back to ingestion time: {}",
                e
            );
            Utc::now()
        });

        let mut enum_ret = unsafe {
            sd_journal_enumerate_data(self.journal, data.as_mut_ptr(), data_len.as_mut_ptr())
        };

        while enum_ret > 0 {
            let bytes =
                unsafe { std::slice::from_raw_parts(data.assume_init(), data_len.assume_init()) };

            let kv_string = String::from_utf8_lossy(bytes).to_string();
            fields.push(kv_string);

            enum_ret = unsafe {
                sd_journal_enumerate_data(self.journal, data.as_mut_ptr(), data_len.as_mut_ptr())
            };
        }

        if enum_ret < 0 {
            Err(eyre!("Failed to read journal entry data: {}", enum_ret))
        } else {
            Ok(Some(JournalEntryRaw::new(fields, timestamp)))
        }
    }

    fn wait_for_entry(&mut self) -> Result<()> {
        let mut fds = [self.wait_fd];
        let ret = poll(&mut fds, Self::POLL_TIMEOUT_MS)?;
        if ret < 0 {
            return Err(eyre!("Failed to poll for journal entry: {}", ret));
        }

        // This call clears the queue status of the poll fd
        let ret = unsafe { sd_journal_process(self.journal) };
        if ret < 0 {
            return Err(eyre!("Failed to process journal entry: {}", ret));
        }

        Ok(())
    }
}

/// A fully parsed journal entry.
///
/// This struct represents a fully parsed journal entry, with all key-value pairs parsed into a HashMap.
#[derive(Serialize, Debug)]
pub struct JournalEntry {
    pub ts: DateTime<Utc>,
    pub fields: HashMap<String, String>,
}

impl From<JournalEntryRaw> for JournalEntry {
    fn from(raw: JournalEntryRaw) -> Self {
        let fields = raw
            .fields
            .into_iter()
            .fold(HashMap::new(), |mut acc, field| {
                let kv: Vec<&str> = field.splitn(2, '=').collect();
                if kv.len() == 2 {
                    acc.insert(kv[0].to_string(), kv[1].to_string());
                }
                acc
            });

        Self { ts: raw.ts, fields }
    }
}

impl TryFrom<JournalEntry> for LogEntry {
    type Error = Error;

    fn try_from(mut entry: JournalEntry) -> Result<Self, Self::Error> {
        let ts = entry.ts;

        let fields = &mut entry.fields;
        let message = fields
            .remove("MESSAGE")
            .ok_or_else(|| eyre!("Journal entry is missing MESSAGE field"))?;

        let pid = fields.remove("_PID");
        let systemd_unit = fields.remove("_SYSTEMD_UNIT");
        let priority = fields.remove("PRIORITY");

        let extra_fields = entry
            .fields
            .into_iter()
            .fold(HashMap::new(), |mut acc, (k, v)| {
                acc.insert(k, LogValue::String(v));
                acc
            });
        let data = LogData {
            message,
            pid,
            systemd_unit,
            priority,
            original_priority: None,
            extra_fields,
        };

        Ok(LogEntry { ts, data })
    }
}

/// A wrapper around the `JournalRaw` trait that provides an iterator over journal entries.
///
/// This struct provides an iterator over journal entries, abstracting the raw journal reading logic.
pub struct Journal<J: JournalRaw> {
    journal: J,
}

impl<J: JournalRaw> Journal<J> {
    pub fn new(journal: J) -> Self {
        Self { journal }
    }

    /// Get the next journal entry.
    ///
    /// This function will return a `JournalEntry` if the next entry is available, or `None` if there are no more entries.
    /// Returns an error if there was an issue reading the journal.
    fn next_entry(&mut self) -> Result<Option<JournalEntry>> {
        match self.journal.next_entry_available()? {
            false => Ok(None),
            true => {
                let raw = self.journal.get_entry_field_data()?;
                Ok(raw.map(|raw| raw.into()))
            }
        }
    }

    /// Get an iterator over all available journal entries.
    pub fn iter(&mut self) -> impl Iterator<Item = JournalEntry> + '_ {
        std::iter::from_fn(move || match self.next_entry() {
            Ok(entry) => entry,
            Err(e) => {
                warn!("Failed to get next journal entry: {}", e);
                None
            }
        })
    }

    pub fn wait_for_entry(&mut self) -> Result<()> {
        self.journal.wait_for_entry()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use insta::{assert_json_snapshot, with_settings};
    use mockall::Sequence;

    #[test]
    fn test_from_raw_journal_entry() {
        let raw_entry = raw_journal_entry();
        let entry = JournalEntry::from(raw_entry);

        with_settings!({sort_maps => true}, {
            assert_json_snapshot!(entry);
        });
    }

    #[test]
    fn test_journal_happy_path() {
        let mut raw_journal = MockJournalRaw::new();
        let mut seq = Sequence::new();

        raw_journal
            .expect_next_entry_available()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|| Ok(true));

        raw_journal
            .expect_get_entry_field_data()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|| Ok(Some(raw_journal_entry())));

        raw_journal
            .expect_next_entry_available()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|| Ok(false));

        let mut journal = Journal::new(raw_journal);
        let mut journal_iter = journal.iter();
        let entry = journal_iter.next();
        assert!(entry.is_some());
        with_settings!({sort_maps => true}, {
            assert_json_snapshot!(entry.unwrap());
        });
        let entry = journal_iter.next();
        assert!(entry.is_none());
    }

    fn timestamp() -> DateTime<Utc> {
        let timestamp = NaiveDateTime::from_timestamp_millis(1713462571).unwrap();
        DateTime::<Utc>::from_utc(timestamp, Utc)
    }

    fn raw_journal_entry() -> JournalEntryRaw {
        let fields = [
            "_SYSTEMD_UNIT=user@1000.service",
            "MESSAGE=audit: type=1400 audit(1713462571.968:7508): apparmor=\"DENIED\" operation=\"open\" class=\"file\" profile=\"snap.firefox.firefox\" name=\"/etc/fstab\" pid=10122 comm=\"firefox\" requested_mask=\"r\" denied_mask=\"r\" fsuid=1000 ouid=0",
        ];

        JournalEntryRaw::new(fields.iter().map(|s| s.to_string()).collect(), timestamp())
    }
}
