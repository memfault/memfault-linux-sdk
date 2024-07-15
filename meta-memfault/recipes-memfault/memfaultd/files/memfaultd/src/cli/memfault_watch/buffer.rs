//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    io::{BufRead, Result, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use log::error;

pub(crate) static STOP_THREADS: AtomicBool = AtomicBool::new(false);

/// This function monitors the reader and calls `read_and_double_write` on the two inputs.
/// It handles the buffering and the quitting in the case where the `read()` call returns
/// an error (in the case of an OS-level problem retrieving the process's stream) and
/// also the case where the program no longer as any more bytes to read.
pub(crate) fn monitor_and_buffer(
    reader: impl BufRead,
    mut io_writer: &mut impl Write,
    file_writer: Arc<Mutex<impl Write>>,
) {
    // Create an initial buffer with a large starting capacity, to avoid the rapid
    // allocation that can occur when inserting over and over again one by one
    // Refs: https://blog.mozilla.org/nnethercote/2020/08/05/how-to-speed-up-the-rust-compiler-some-more-in-2020/
    // (under miscellaneous, discusses how rust-lang solved an adjacent issue by
    // making the default allocation a minimum of 4)

    // The default value of 240 was chosen as a rough estimate of the average line size
    // of a log. If there are lines significantly longer, the higher capacity will
    // persist after clearing.
    let mut buffer = Vec::with_capacity(240);
    let mut reader = reader;
    loop {
        // Here we catch the case where we need to stop the threads because of an error
        // reaching the process. We call join() in the upper loop, but this just waits
        // for each thread to stop naturally.
        if STOP_THREADS.load(Ordering::Relaxed) {
            break;
        }

        // Utilize the Take<impl BufRead> adapter to temporarily limit the amount we
        // can ever read from *this limiter*. This avoids the problem later on in
        // `read_and_double_write`, which calls `read_until` in order to read
        // uninterrupted until reaching a \n. If a program's output prints extremely long
        // lines, without ever printing a \n (for example, if a print subroutine breaks),
        // we still want to handle that by limiting to this value.
        let mut temporary_limiter = reader.take(8192);
        let read_result = read_and_double_write(
            &mut buffer,
            &mut temporary_limiter,
            &mut io_writer,
            file_writer.clone(),
        );

        //Here we catch the case where we successfully read zero bytes. This should
        // _only_ happen when there is _no more_ data to be read - because otherwise,
        // any implementation of `Read` should _always block_ upon trying to read,
        // waiting until there's _something_ to return (even Ok(1)).

        // TLDR: Ok(0) should only occur if the program exited. So, break.
        if let Ok(0) = read_result {
            break;
        }
        // Here we catch any `io`-related error when reading.
        if let Err(e) = read_result {
            error!("Error reading data: {e}");
            break;
        }
        // Clear the buffer - zeroes out the contents but does not affect the capacity
        // Leaving the capacity untouched for two reasons:
        // 1. Avoiding reallocating if lines are consistently longer than 240 bytes
        // 2. When considering the allocation, it's unlikely for some extremely long
        // line to hold a large buffer
        buffer.clear();
        // Here, we reset the reader to remove the limiter, by consuming the Take and
        // saving it back as the wrapped reader.
        reader = temporary_limiter.into_inner();
    }

    if let Err(e) = file_writer
        .lock()
        .expect("Failed to lock file writer")
        .flush()
    {
        error!("Error flushing file writer. {e}");
    }
}

/// This function is meant to be called in a loop - it reads as much as it can from the
/// `reader` into the buffer, and writes to each of the writers. It handles the case of
/// no data currently available, as well as a broken reader, by returning an
/// std::io::Result. The contained usize success value is the number of bytes read.
/// Returns an error in the case of a generic io::Error, whether in the read, or in
/// either of the two writes/flushes.
/// The file writer is not flushed, as it's wrapped in a BufWriter and only needs to be
/// flushed at the end.
#[inline(always)]
pub(crate) fn read_and_double_write(
    buffer: &mut Vec<u8>,
    reader: &mut impl BufRead,
    io_writer: &mut impl Write,
    file_writer: Arc<Mutex<impl Write>>,
) -> Result<usize> {
    // We use read_until here to continuously read bytes until we reach a newline.
    match reader.read_until(b'\n', buffer)? {
        0 => Ok(0),
        bytes_read => {
            io_writer.write_all(buffer)?;
            io_writer.flush()?;
            file_writer
                .lock()
                .expect("Failed to lock file writer")
                .write_all(buffer)?;
            Ok(bytes_read)
        }
    }
}
