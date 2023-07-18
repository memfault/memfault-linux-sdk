//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::Result;

use crate::util::ipc::send_flush_signal;

pub fn sync() -> Result<()> {
    send_flush_signal()
}
