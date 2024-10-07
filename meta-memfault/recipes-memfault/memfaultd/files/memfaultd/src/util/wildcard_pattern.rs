//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Very simple wildcard pattern implementation.
//! Only 1 wildcard position in the pattern is currently
//! supported and strings with an arbitrary amount of
//! characters greater than or equal to 0 in the wildcard
//! position will always match.
use std::fmt::{Display, Formatter, Result as FmtResult};

#[derive(Clone)]
pub struct WildcardPattern {
    prefix: String,
    suffix: String,
}

impl WildcardPattern {
    /// Construct a new Wildcard pattern with the following
    /// format, where * represents the wildcard position:
    /// "<prefix>*<suffix>"
    pub fn new(prefix: &str, suffix: &str) -> Self {
        Self {
            prefix: prefix.to_string(),
            suffix: suffix.to_string(),
        }
    }

    pub fn matches(&self, s: &str) -> bool {
        s.starts_with(self.prefix.as_str()) && s.ends_with(self.suffix.as_str())
    }
}

impl Display for WildcardPattern {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}*{}", self.prefix, self.suffix)
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("cat", "dog", "catcatdog")]
    #[case("Mem", "fault", "Memefault")]
    #[case("Hello, ", "!", "Hello, Bob!")]
    #[case("Zero Character", " Wildcard Match", "Zero Character Wildcard Match")]
    fn test_matches(#[case] prefix: &str, #[case] suffix: &str, #[case] s: &str) {
        assert!(WildcardPattern::new(prefix, suffix).matches(s))
    }

    #[rstest]
    #[case("cat", "dog", "cacatdog")]
    #[case("Mem", "fault", "MemFault")]
    #[case("Hello, ", "!", "Hello! Bob!")]
    fn test_nonmatches_do_not_match(#[case] prefix: &str, #[case] suffix: &str, #[case] s: &str) {
        assert!(!WildcardPattern::new(prefix, suffix).matches(s))
    }
}
