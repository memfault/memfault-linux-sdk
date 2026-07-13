//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Checks the headroom of the chunks files directory.
//! Currently a very basic equivalent of the logs headroom checker,
//! but provides an easily extensible interface for later elaboration
//! if desired.

use eyre::Result;

use crate::util::disk_size::DiskSize;

pub trait ChunksHeadroomCheck {
    fn check_with_added_space(&mut self, chunk_len: usize) -> Result<bool>;
}
pub struct ChunksHeadroomLimiter {
    min_headroom: DiskSize,
    get_available_space: Box<dyn FnMut() -> Result<DiskSize> + Send>,
}

impl ChunksHeadroomLimiter {
    pub fn new<S: FnMut() -> Result<DiskSize> + Send + 'static>(
        min_headroom: DiskSize,
        get_available_space: S,
    ) -> Self {
        Self {
            min_headroom,
            get_available_space: Box::new(get_available_space),
        }
    }
}

impl ChunksHeadroomCheck for ChunksHeadroomLimiter {
    /// Checks if there's enough headroom to continue writing chunks.
    fn check_with_added_space(&mut self, chunk_len: usize) -> Result<bool> {
        let chunks_space = DiskSize {
            bytes: chunk_len as u64,
            // some transactions may add another inode, so it's reasonable to over-approximate here
            // furthermore if we're short on inodes we should probably not be writing to disk
            inodes: 1,
        };
        let needed_space = self.min_headroom + chunks_space;
        let available = (self.get_available_space)()?;
        Ok(available.exceeds(&needed_space))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    };

    use crate::util::disk_size::DiskSize;
    use rstest::{fixture, rstest};

    use super::*;

    #[rstest]
    fn returns_true_if_has_headroom(mut fixture: Fixture) {
        fixture.set_available_space(MIN_HEADROOM + 64);
        assert!(fixture.limiter.check_with_added_space(64).unwrap());
    }

    #[rstest]
    fn headroom_shortage(mut fixture: Fixture) {
        fixture.set_available_space(MIN_HEADROOM - 1);
        assert!(!fixture.limiter.check_with_added_space(64).unwrap());
    }

    #[rstest]
    fn inode_shortage(mut fixture: Fixture) {
        fixture.set_available_inodes(MIN_INODES - 1);
        assert!(!fixture.limiter.check_with_added_space(64).unwrap());
    }

    const MIN_HEADROOM: u64 = 1024;
    const MIN_INODES: u64 = 10;
    const INITIAL_AVAILABLE_SPACE: u64 = 1024 * 1024;
    const INITIAL_AVAILABLE_INODES: u64 = 100;

    struct Fixture {
        available_space: Arc<AtomicU64>,
        available_inodes: Arc<AtomicU64>,
        limiter: ChunksHeadroomLimiter,
    }
    impl Fixture {
        fn set_available_space(&mut self, available_space: u64) {
            self.available_space
                .store(available_space, Ordering::Relaxed)
        }
        fn set_available_inodes(&mut self, available_inodes: u64) {
            self.available_inodes
                .store(available_inodes, Ordering::Relaxed)
        }
    }
    #[fixture]
    fn fixture() -> Fixture {
        let available_space = Arc::new(AtomicU64::new(INITIAL_AVAILABLE_SPACE));
        let available_inodes = Arc::new(AtomicU64::new(INITIAL_AVAILABLE_INODES));

        let space = available_space.clone();
        let inodes = available_inodes.clone();

        Fixture {
            limiter: ChunksHeadroomLimiter::new(
                DiskSize {
                    bytes: MIN_HEADROOM,
                    inodes: MIN_INODES,
                },
                move || {
                    Ok(DiskSize {
                        bytes: space.load(Ordering::Relaxed),
                        inodes: inodes.load(Ordering::Relaxed),
                    })
                },
            ),
            available_inodes,
            available_space,
        }
    }
}
