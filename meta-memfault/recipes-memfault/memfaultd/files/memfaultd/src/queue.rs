//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use chrono::{TimeZone, Utc};
use eyre::Result;
use libc::time_t;
use memfaultc_sys::{
    memfaultd_queue_complete_read, memfaultd_queue_destroy, memfaultd_queue_read_head, QueueHandle,
};
use std::ffi::CStr;

/// An opaque struct holding a pointer to a C memfault_queue.
#[derive(Debug)]
pub struct Queue {
    handle: QueueHandle,
    destroy_on_drop: bool,
}

impl Queue {
    /// Create a queue from a [QueueHandle]. This queue will **not** be destroyed when it's dropped.
    pub fn attach(handle: QueueHandle) -> Self {
        Queue {
            handle,
            destroy_on_drop: false,
        }
    }

    /// A function to read the next message from the queue if there is any
    /// To read the next message, mark this one as processed and drop it out of scope.
    pub fn read(&mut self) -> Option<QueueMessage> {
        let mut size_bytes: u32 = 0;
        unsafe {
            let data: *const u8 = memfaultd_queue_read_head(self.handle, &mut size_bytes);
            if data.is_null() {
                return None;
            }
            let msg = std::slice::from_raw_parts(data, size_bytes as usize);

            Some(QueueMessage {
                msg,
                queue: self,
                processed: false,
            })
        }
    }
}

impl Drop for Queue {
    fn drop(&mut self) {
        if self.destroy_on_drop {
            unsafe { memfaultd_queue_destroy(self.handle) }
        }
    }
}

#[derive(Debug)]
pub struct QueueMessage<'a> {
    pub msg: &'a [u8],
    queue: &'a Queue,
    processed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum QueueMessageType {
    RebootEvent = b'R',
    Attributes = b'A',
}
impl TryFrom<u8> for QueueMessageType {
    type Error = eyre::Error;

    fn try_from(v: u8) -> Result<QueueMessageType> {
        match v {
            b'R' => Ok(QueueMessageType::RebootEvent),
            b'A' => Ok(QueueMessageType::Attributes),
            _ => Err(eyre::eyre!("Invalid message.")),
        }
    }
}

impl<'a> QueueMessage<'a> {
    pub fn set_processed(&mut self, p: bool) {
        self.processed = p;
    }

    pub fn get_type(&self) -> Option<QueueMessageType> {
        if !self.msg.is_empty() {
            self.msg[0].try_into().ok()
        } else {
            None
        }
    }

    pub fn get_payload(&self) -> &[u8] {
        if self.msg.len() > 1 {
            &self.msg[1..]
        } else {
            &[]
        }
    }

    /// Returns payload discarding null-termination.
    /// Only works when the payload is known to be a null terminated string.
    pub fn get_payload_cstr(&self) -> Result<&str, eyre::Error> {
        Ok(CStr::from_bytes_with_nul(self.get_payload())?.to_str()?)
    }
}

impl<'a> Drop for QueueMessage<'a> {
    fn drop(&mut self) {
        if self.processed {
            unsafe {
                memfaultd_queue_complete_read(self.queue.handle);
            }
        }
    }
}

/// The message queued to send device attributes to Memfault. This contains the
/// timestamp at which the attributes were set and a null terminated JSON blob.
pub struct QueueMessageAttributes<'a> {
    pub timestamp: chrono::DateTime<Utc>,
    pub json: &'a str,
}
impl<'a> TryFrom<&'a QueueMessage<'a>> for QueueMessageAttributes<'a> {
    type Error = eyre::Error;

    fn try_from(v: &'a QueueMessage) -> Result<QueueMessageAttributes<'a>, eyre::Error> {
        if v.get_type() == Some(QueueMessageType::Attributes) && v.msg.len() > 5 {
            const SIZE_TIME_T: usize = std::mem::size_of::<time_t>();
            let bytes: [u8; SIZE_TIME_T] = v.msg[1..SIZE_TIME_T + 1].try_into()?;
            let timestamp_raw = time_t::from_ne_bytes(bytes);
            let json = std::ffi::CStr::from_bytes_with_nul(&v.msg[SIZE_TIME_T + 1..])?;

            // into() is required on 32 bit systems to convert a i32 to i64
            #[allow(clippy::useless_conversion)]
            let timestamp = Utc
                .timestamp_opt(timestamp_raw.into(), 0)
                .single()
                .ok_or(eyre::eyre!("Invalid timestamp."))?;
            Ok(QueueMessageAttributes {
                timestamp,
                json: json.to_str()?,
            })
        } else {
            Err(eyre::eyre!("Invalid message"))
        }
    }
}

#[cfg(test)]
mod tests {
    use eyre::eyre;
    use std::{ffi::CString, os::unix::prelude::OsStrExt, path::Path};

    use memfaultc_sys::{memfaultd_queue_init, memfaultd_queue_write};

    use super::*;

    impl Queue {
        /// Create a new queue with optional file backing. This queue will be destroyed when it comes out of scope.
        pub fn new<T: AsRef<Path>>(file: Option<T>, size: usize) -> Result<Self> {
            let handle = unsafe {
                memfaultd_queue_init(
                    match file {
                        Some(filepath) => {
                            CString::new(filepath.as_ref().as_os_str().as_bytes())?.as_ptr()
                        }
                        _ => std::ptr::null(),
                    },
                    size as libc::c_int,
                )
            };
            if handle.is_null() {
                Err(eyre!("Error initializing queue (NULL)."))
            } else {
                Ok(Queue {
                    handle,
                    destroy_on_drop: true,
                })
            }
        }

        /// Write a message to the queue
        pub fn write(&mut self, payload: &[u8]) -> bool {
            unsafe { memfaultd_queue_write(self.handle, payload.as_ptr(), payload.len() as u32) }
        }
    }

    #[test]
    fn create_queue() -> Result<()> {
        let mut queue = Queue::new::<&Path>(None, 1024)?;
        assert!(queue.read().is_none());
        Ok(())
    }

    #[test]
    fn read_write() -> Result<()> {
        let mut queue = Queue::new::<&Path>(None, 1024)?;
        assert!(queue.write(b"PAYLOAD"));

        {
            let m = queue.read();
            assert!(m.is_some());
            assert_eq!(m.unwrap().msg, b"PAYLOAD");
            // do not mark as processed.
        }

        // Make sure message has not been deleted.
        assert!(queue.read().is_some());
        Ok(())
    }

    #[test]
    fn read_drop() -> Result<()> {
        let mut queue = Queue::new::<&Path>(None, 1024)?;
        assert!(queue.write(b"PAYLOAD"));

        {
            let m = queue.read();
            m.unwrap().set_processed(true);
        }

        // Make sure message has not been deleted.
        assert!(queue.read().is_none());
        Ok(())
    }

    #[test]
    fn attributes() -> Result<()> {
        let mut queue = Queue::new::<&Path>(None, 1024)?;

        let mut buf = b"A\x8C\xA1\xC0\x63\x00\x00\x00\x00{\"sole_value\":\"x\"}\0".to_owned();

        buf[1..5].copy_from_slice(&0xDEADBEEFu32.to_ne_bytes());
        assert!(queue.write(&buf));

        let m = queue.read().unwrap();
        let attribute_post = QueueMessageAttributes::try_from(&m).unwrap();

        assert_eq!(attribute_post.timestamp.timestamp() as u32, 0xDEADBEEF);
        assert_eq!(attribute_post.json, r##"{"sole_value":"x"}"##);
        Ok(())
    }

    #[test]
    fn ascii_payload() -> Result<()> {
        let mut queue = Queue::new::<&Path>(None, 1024)?;

        let buf = b"cCargo.lock\0".to_owned();
        queue.write(&buf);

        let m = queue.read().unwrap();
        assert_eq!(m.get_payload_cstr().unwrap(), "Cargo.lock");
        Ok(())
    }
}
