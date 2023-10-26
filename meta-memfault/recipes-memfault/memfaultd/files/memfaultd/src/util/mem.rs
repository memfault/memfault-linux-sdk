//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::mem::size_of;
use std::slice::{from_raw_parts, from_raw_parts_mut};

pub trait AsBytes {
    /// Returns a slice of bytes representing the raw memory of the object.
    /// # Safety
    /// It is on the caller to ensure the interpretation of the bytes is correct.
    unsafe fn as_bytes(&self) -> &[u8];

    /// Returns a mutable slice of bytes representing the raw memory of the object.
    /// # Safety
    /// The type must not contain any references, pointers or types that require
    /// validating invariants.
    unsafe fn as_mut_bytes(&mut self) -> &mut [u8];
}

impl<T: Sized> AsBytes for T {
    unsafe fn as_bytes(&self) -> &[u8] {
        from_raw_parts((self as *const T) as *const u8, size_of::<T>())
    }

    unsafe fn as_mut_bytes(&mut self) -> &mut [u8] {
        from_raw_parts_mut((self as *mut T) as *mut u8, size_of::<T>())
    }
}
