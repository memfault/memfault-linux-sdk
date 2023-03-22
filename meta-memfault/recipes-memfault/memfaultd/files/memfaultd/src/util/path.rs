//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::ffi::OsStr;
use std::path::Path;

/// Splits the filename at the first dot.
/// This is similar to the nighly-only std::path::Path::file_prefix.
pub fn file_prefix(path: &Path) -> Option<&OsStr> {
    let file_name = path.file_name()?;
    file_name.to_str()?.split('.').next().map(OsStr::new)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("/test", "test")]
    #[case("/test.log", "test")]
    #[case("/test.log.zlib", "test")]
    fn test_file_prefix(#[case] path: &str, #[case] expected: &str) {
        assert_eq!(file_prefix(Path::new(path)), Some(OsStr::new(expected)));
    }
}
