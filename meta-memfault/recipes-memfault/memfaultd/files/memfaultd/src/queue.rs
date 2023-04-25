//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::Result;
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
}
impl TryFrom<u8> for QueueMessageType {
    type Error = eyre::Error;

    fn try_from(v: u8) -> Result<QueueMessageType> {
        match v {
            b'R' => Ok(QueueMessageType::RebootEvent),
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
    fn ascii_payload() -> Result<()> {
        let mut queue = Queue::new::<&Path>(None, 1024)?;

        let buf = b"cCargo.lock\0".to_owned();
        queue.write(&buf);

        let m = queue.read().unwrap();
        assert_eq!(m.get_payload_cstr().unwrap(), "Cargo.lock");
        Ok(())
    }
}
