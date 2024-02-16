//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt::{Debug, Formatter};
use std::str::FromStr;
use std::{borrow::Cow, fmt::Display};

use eyre::{eyre, ErrReport, Result};

use crate::util::patterns::alphanum_slug_is_valid_and_starts_alpha;

/// Struct containing a valid session name
#[derive(Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
#[repr(transparent)]
pub struct SessionName {
    inner: String,
}

impl SessionName {
    pub fn as_str(&self) -> &str {
        &self.inner
    }
}

impl Debug for SessionName {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.inner, f)
    }
}

impl FromStr for SessionName {
    type Err = ErrReport;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match alphanum_slug_is_valid_and_starts_alpha(s, 64) {
            Ok(()) => Ok(Self {
                inner: s.to_string(),
            }),
            Err(e) => Err(eyre!("Invalid session name {}: {}", s, e)),
        }
    }
}

impl Display for SessionName {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.inner, f)
    }
}

impl Serialize for SessionName {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SessionName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<SessionName, D::Error> {
        let s: Cow<str> = Deserialize::deserialize(deserializer)?;
        let name: SessionName = str::parse(&s).map_err(serde::de::Error::custom)?;
        Ok(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::*;

    #[rstest]
    #[case("")]
    #[case("\u{1F4A9}")]
    #[case("Wi-fi Connected")]
    fn validation_errors(#[case] input: &str) {
        assert!(SessionName::from_str(input).is_err())
    }

    #[rstest]
    #[case("foo")]
    #[case("valid_session-name")]
    fn parsed_ok(#[case] input: &str) {
        assert!(SessionName::from_str(input).is_ok())
    }
}
