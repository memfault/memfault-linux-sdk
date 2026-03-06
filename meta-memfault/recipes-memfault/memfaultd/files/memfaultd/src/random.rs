//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use chrono::Utc;
use getrandom::Error;
use libc::{rand, srand};
use std::sync::{Mutex, Once};

/// One-time initialization of the random seed
static SEED_RAND: Once = Once::new();

/// Mutex to protect access to rand() for thread safety
static RAND_LOCK: Mutex<()> = Mutex::new(());

/// Custom implementation of `getrandom` used for older kernels
///
/// This implementation is thread-safe, using a mutex to serialize access
/// to the C library's `rand()` function.
///
/// # Safety
///
/// Pointer is checked for null before being dereferenced
pub unsafe fn custom_getrandom(dest: *mut u8, len: usize) -> Result<(), Error> {
    if len == 0 {
        return Ok(());
    }
    if dest.is_null() {
        return Err(Error::UNEXPECTED);
    }

    // Initialize seed once using timestamp
    SEED_RAND.call_once(|| {
        let timestamp = Utc::now().timestamp_subsec_micros();
        unsafe {
            srand(timestamp as _);
        }
    });

    let mut offset = 0;
    while offset < len {
        let random_int = {
            let _guard = RAND_LOCK.lock().expect("mutex poisoned");
            unsafe { rand() as u32 }
        };
        let bytes = random_int.to_ne_bytes();

        let remaining = len - offset;
        let chunk_size = remaining.min(4);

        for (i, byte) in bytes.iter().enumerate().take(chunk_size) {
            unsafe {
                *dest.add(offset + i) = *byte;
            }
        }
        offset += chunk_size;
    }

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    use std::ptr::null_mut;

    use rstest::rstest;

    #[rstest]
    #[case(vec![0u8; 1024])]
    #[case(vec![0u8; 3])]
    #[case(vec![0u8; 5])]
    #[case(vec![0u8; 0])]
    fn test_custom_getrandom(#[case] mut buf: Vec<u8>) {
        let result = std::panic::catch_unwind(move || unsafe {
            custom_getrandom(buf.as_mut_ptr(), buf.len())
        });
        assert!(result.is_ok());
        assert!(result.unwrap().is_ok());
    }

    #[rstest]
    fn test_custom_getrandom_null_ptr() {
        let result = std::panic::catch_unwind(move || unsafe { custom_getrandom(null_mut(), 42) });
        assert!(result.is_ok());
        assert!(result.unwrap().is_err());
    }

    #[test]
    fn test_custom_getrandom_thread_safety() {
        use std::thread;

        let handles: Vec<_> = (0..10)
            .map(|_| {
                thread::spawn(|| {
                    let mut buf = vec![0u8; 128];
                    for _ in 0..100 {
                        let result = unsafe { custom_getrandom(buf.as_mut_ptr(), buf.len()) };
                        assert!(result.is_ok());
                    }
                    buf
                })
            })
            .collect();

        for handle in handles {
            let buf = handle.join().expect("Thread panicked");
            assert!(
                buf.iter().any(|&b| b != 0),
                "Buffer should contain random data"
            );
        }
    }
}
