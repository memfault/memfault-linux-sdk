//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::{eyre, Error, Result};
use std::path::Path;
use std::{ffi::OsStr, path::PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(try_from = "PathBuf")]
/// A path that must be absolute. Use `AbsolutePath::try_from` to construct.
pub struct AbsolutePath(PathBuf);

impl TryFrom<PathBuf> for AbsolutePath {
    type Error = Error;

    fn try_from(path: PathBuf) -> Result<Self> {
        if path.is_absolute() {
            Ok(Self(path))
        } else {
            Err(eyre!("Path must be absolute: {:?}", path))
        }
    }
}
impl From<AbsolutePath> for PathBuf {
    fn from(p: AbsolutePath) -> PathBuf {
        p.0
    }
}
impl PartialEq<AbsolutePath> for PathBuf {
    fn eq(&self, other: &AbsolutePath) -> bool {
        *self == *other.0
    }
}

/// Splits the filename at the first dot.
/// This is similar to the nighly-only std::path::Path::file_prefix.
#[allow(dead_code)]
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
