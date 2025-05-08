//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt::{Debug, Formatter};
use std::str::FromStr;
use std::{borrow::Cow, fmt::Display};

/// Struct containing a valid metric / attribute key.
#[derive(Clone, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct MetricStringKey {
    inner: String,
}

impl MetricStringKey {
    pub fn as_str(&self) -> &str {
        &self.inner
    }

    pub fn with_suffix(&self, suffix: &str) -> Self {
        Self {
            inner: (self.inner.clone() + suffix),
        }
    }
}

impl Debug for MetricStringKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.inner, f)
    }
}

impl FromStr for MetricStringKey {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !(1..=128).contains(&s.len()) {
            return Err("Invalid key: must be between 1 and 128 characters");
        }
        if !s.is_ascii() {
            return Err("Invalid key: must be ASCII");
        }
        Ok(Self {
            // Sanitize metric key by replacing any NUL (0x0) characters
            // with an empty string
            inner: s.replace('\0', ""),
        })
    }
}

/// Supports creating `MetricStringKey` from a `'static' string. This skips the
/// verification of validity.
/// Ideally we would replace this with a const-compatible verification of
/// validity but it' not available in our currently Rust min version.
impl From<&'static str> for MetricStringKey {
    fn from(value: &'static str) -> Self {
        Self {
            // FIXME: We could really optimize this use-case if inner was a Cow...
            inner: value.to_string(),
        }
    }
}

impl Display for MetricStringKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.inner, f)
    }
}

impl Ord for MetricStringKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.inner.cmp(&other.inner)
    }
}
impl PartialOrd for MetricStringKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.inner.cmp(&other.inner))
    }
}

impl Serialize for MetricStringKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for MetricStringKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<MetricStringKey, D::Error> {
        let s: Cow<str> = Deserialize::deserialize(deserializer)?;
        let key: MetricStringKey = str::parse(&s).map_err(serde::de::Error::custom)?;
        Ok(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::*;

    #[rstest]
    #[case("", "Invalid key: must be between 1 and 128 characters")]
    #[case("\u{1F4A9}", "Invalid key: must be ASCII")]
    fn validation_errors(#[case] input: &str, #[case] expected: &str) {
        let result: Result<MetricStringKey, &str> = str::parse(input);
        assert_eq!(result.err().unwrap(), expected);
    }

    #[rstest]
    #[case("foo")]
    #[case("weird valid.key-123$")]
    fn parsed_ok(#[case] input: &str) {
        let result: Result<MetricStringKey, &str> = str::parse(input);
        assert_eq!(result.ok().unwrap().as_str(), input);
    }

    #[rstest]
    fn null_gets_removed() {
        let result: Result<MetricStringKey, &str> = str::parse("foo\0");
        assert_eq!(result.ok().unwrap().as_str(), "foo");
    }
}
