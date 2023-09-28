//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::{eyre, Result};
use strum_macros::EnumString;

#[derive(EnumString, Default, PartialEq, Eq, Debug)]
#[strum(serialize_all = "kebab-case")]
/// Supported export formats
pub enum ExportFormat {
    #[default]
    /// Default format is a MAR file (zip archive)
    Mar,
    /// Chunk is a memfault-specific format (it's a container on top of MAR).
    /// It's mostly useful when you are already exporting memfault chunks from a
    /// MCU device and want the Linux data in the same format.
    Chunk,
    /// Memfault chunks do not include a header to identify the format. They
    /// also do not include the length of the data. This format adds a 'CHNK'
    /// header and a length field.
    ChunkWrapped,
}

const CONTENT_TYPE_ZIP: &str = "application/zip";
const CONTENT_TYPE_CHUNK: &str = "application/vnd.memfault.chunk";
const CONTENT_TYPE_CHUNK_WRAPPED: &str = "application/vnd.memfault.chunk-wrapped";

impl ExportFormat {
    pub fn to_content_type(&self) -> &'static str {
        match self {
            ExportFormat::Mar => CONTENT_TYPE_ZIP,
            ExportFormat::Chunk => CONTENT_TYPE_CHUNK,
            ExportFormat::ChunkWrapped => CONTENT_TYPE_CHUNK_WRAPPED,
        }
    }

    fn from_mime_type(value: &str) -> Option<Self> {
        match value {
            "*/*" => Some(Self::Mar),
            CONTENT_TYPE_ZIP => Some(Self::Mar),
            CONTENT_TYPE_CHUNK => Some(Self::Chunk),
            CONTENT_TYPE_CHUNK_WRAPPED => Some(Self::ChunkWrapped),
            _ => None,
        }
    }

    pub fn from_accept_header(value: &str) -> Result<Self> {
        value
            .split(',')
            .find_map(|mime_type| {
                let mime_type = mime_type.trim();
                Self::from_mime_type(mime_type)
            })
            .ok_or_else(|| eyre!("Requested format not supported (Accept: {})", value))
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("*/*", ExportFormat::Mar)]
    #[case(CONTENT_TYPE_ZIP, ExportFormat::Mar)]
    #[case(format!("{}, {}", CONTENT_TYPE_ZIP, CONTENT_TYPE_CHUNK), ExportFormat::Mar)]
    #[case(format!("{}, {}", CONTENT_TYPE_CHUNK, CONTENT_TYPE_ZIP), ExportFormat::Chunk)]
    fn test_accept_header_parser<H: Into<String>>(
        #[case] header_value: H,
        #[case] expected_format: ExportFormat,
    ) {
        assert_eq!(
            ExportFormat::from_accept_header(&header_value.into()).unwrap(),
            expected_format
        );
    }
}
