//
// Copyright (c) Memfault, Inc.
// See License.txt for details
mod report;
pub use report::{HrtReport, HRT_DEFAULT_MAX_SAMPLES_PER_MIN};

mod schema;
pub use schema::write_report_to_disk;
// Export for use in tests
#[cfg(test)]
pub use schema::HighResTelemetryV1;
