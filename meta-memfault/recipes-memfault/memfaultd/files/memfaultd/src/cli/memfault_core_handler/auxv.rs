//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use scroll::{Pread, NATIVE};
use std::fmt::Debug;

#[cfg(target_pointer_width = "64")]
pub type AuxvUint = u64;

#[cfg(target_pointer_width = "32")]
pub type AuxvUint = u32;

#[derive(Eq, PartialEq)]
pub struct Auxv<'a> {
    data: &'a [u8],
}

#[derive(Eq, PartialEq, Debug)]
pub struct AuxvEntry {
    key: AuxvUint,
    value: AuxvUint,
}

impl<'a> Auxv<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    pub fn iter(&'a self) -> AuxvIterator<'a> {
        AuxvIterator::new(self)
    }
}

impl<'a> Debug for Auxv<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.iter().collect::<Vec<_>>().fmt(f)
    }
}

pub struct AuxvIterator<'a> {
    auxv: &'a Auxv<'a>,
    offset: usize,
}

impl<'a> AuxvIterator<'a> {
    fn new(auxv: &'a Auxv<'a>) -> Self {
        Self { auxv, offset: 0 }
    }
}

impl<'a> Iterator for AuxvIterator<'a> {
    type Item = AuxvEntry;

    fn next(&mut self) -> Option<Self::Item> {
        let mut read = || {
            self.auxv
                .data
                .gread_with::<AuxvUint>(&mut self.offset, NATIVE)
        };
        match (read(), read()) {
            (Ok(key), Ok(value)) => Some(AuxvEntry { key, value }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use rstest::rstest;
    use scroll::IOwrite;
    use std::io::Cursor;

    #[rstest]
    // Empty auxv:
    #[case(
        vec![],
        vec![]
    )]
    // Happy path:
    #[case(
        vec![1, 2, 3, 4],
        vec![AuxvEntry { key: 1, value: 2 }, AuxvEntry { key: 3, value: 4 }]
    )]
    // Partial entry at the end is ignored:
    #[case(
        vec![1, 2, 3],
        vec![AuxvEntry { key: 1, value: 2 }]
    )]
    fn test_auxv_iterator(#[case] values: Vec<AuxvUint>, #[case] expected: Vec<AuxvEntry>) {
        let mut cursor = Cursor::new(vec![]);
        for value in values {
            cursor.iowrite_with::<AuxvUint>(value, NATIVE).unwrap();
        }

        let buffer = cursor.into_inner();
        let auxv = Auxv::new(buffer.as_slice());
        assert_eq!(auxv.iter().collect::<Vec<_>>(), expected);
    }
}
