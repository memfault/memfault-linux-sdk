//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! In loving memory of the practical extraction and report language.
use std::error::Error;

use log::error;

/// Prints the error message to the error log, and then panics.
pub fn die<E: Error>(e: E) -> ! {
    error!("Irrecoverable error: {:#}", e);
    panic!("Irrecoverable error: {:#}", e)
}

pub trait UnwrapOrDie<T> {
    fn unwrap_or_die(self) -> T;
}

impl<T, E: Error> UnwrapOrDie<T> for Result<T, E> {
    fn unwrap_or_die(self) -> T {
        match self {
            Ok(v) => v,
            Err(e) => die(e),
        }
    }
}
