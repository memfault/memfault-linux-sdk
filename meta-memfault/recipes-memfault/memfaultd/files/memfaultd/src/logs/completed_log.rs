//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use crate::mar::CompressionAlgorithm;
use std::path::PathBuf;
use uuid::Uuid;

/// CompletedLog represents a log that has been rotated and is ready to be moved into the MAR
/// staging area.
pub struct CompletedLog {
    pub path: PathBuf,
    pub cid: Uuid,
    pub next_cid: Uuid,
    pub compression: CompressionAlgorithm,
}
