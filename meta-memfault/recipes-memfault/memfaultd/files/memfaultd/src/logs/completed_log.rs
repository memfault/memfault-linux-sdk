//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use crate::mar::manifest::CompressionAlgorithm;
use std::path::PathBuf;
use uuid::Uuid;

pub struct CompletedLog {
    pub path: PathBuf,
    pub cid: Uuid,
    pub next_cid: Uuid,
    pub compression: CompressionAlgorithm,
}
