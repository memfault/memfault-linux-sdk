//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use crate::build_info::{BUILD_ID, GIT_COMMIT, VERSION};

pub fn format_version() -> String {
    format!("VERSION={VERSION}\nGIT COMMIT={GIT_COMMIT}\nBUILD ID={BUILD_ID}")
}
