//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;
use std::fmt::{Debug, Formatter};
use std::str::FromStr;

/// Struct containing a valid metric / attribute key.
#[derive(PartialEq, Eq)]
#[repr(transparent)]
pub struct MetricStringKey {
    inner: String,
}

impl MetricStringKey {
    pub fn as_str(&self) -> &str {
        &self.inner
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
            inner: s.to_string(),
        })
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
}
