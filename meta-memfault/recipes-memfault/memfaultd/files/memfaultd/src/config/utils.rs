//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::path::Path;

use serde_json::Value;

use crate::util::patterns::{
    alphanum_slug_dots_colon_is_valid, alphanum_slug_dots_colon_spaces_parens_slash_is_valid,
    alphanum_slug_is_valid,
};

pub fn software_type_is_valid(s: &str) -> eyre::Result<()> {
    alphanum_slug_dots_colon_is_valid(s, 128)
}

pub fn software_version_is_valid(s: &str) -> eyre::Result<()> {
    alphanum_slug_dots_colon_spaces_parens_slash_is_valid(s, 128)
}

pub fn hardware_version_is_valid(s: &str) -> eyre::Result<()> {
    alphanum_slug_dots_colon_is_valid(s, 128)
}

pub fn device_id_is_valid(id: &str) -> eyre::Result<()> {
    alphanum_slug_is_valid(id, 128)
}

pub fn filter_path_is_valid(path_str: &str) -> eyre::Result<()> {
    let path = Path::new(path_str);
    if !path.exists() {
        return Err(eyre::eyre!("Path {} doesn't exist", path_str));
    }
    if !path.is_absolute() {
        return Err(eyre::eyre!("Path {} isn't absolute", path_str));
    }

    Ok(())
}

/// Custom coredump attribute values must be a single scalar (string, number, or
/// boolean) and non-null so they can be stored as a per-trace attribute. Nested
/// values and nulls are rejected.
pub fn coredump_attribute_value_is_valid(value: &Value) -> eyre::Result<()> {
    match value {
        Value::String(_) | Value::Number(_) | Value::Bool(_) => Ok(()),
        Value::Null => Err(eyre::eyre!("value must not be null")),
        Value::Array(_) | Value::Object(_) => {
            Err(eyre::eyre!("value must be a string, number, or boolean"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use serde_json::json;

    #[rstest]
    #[case(json!("beta"), true)]
    #[case(json!(42), true)]
    #[case(json!(true), true)]
    #[case(json!(null), false)]
    #[case(json!(["a", "b"]), false)]
    #[case(json!({"nested": 1}), false)]
    fn coredump_attribute_value_is_valid_works(#[case] value: Value, #[case] expected: bool) {
        assert_eq!(coredump_attribute_value_is_valid(&value).is_ok(), expected);
    }

    #[rstest]
    // Minimum 1 character
    #[case("A", true)]
    // Allowed characters
    #[case(
        "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789abcdefghijklmnopqrstuvwxyz_-",
        true
    )]
    // Disallowed characters
    #[case("DEMO.1234", false)]
    #[case("DEMO 1234", false)]
    // Too short (0 characters)
    #[case("", false)]
    // Too long (129 characters)
    #[case("012345679012345679012345679012345679012345679012345679012345679012345679012345679012345679012345679012345679012345678901234567890", false)]
    fn device_id_is_valid_works(#[case] device_id: &str, #[case] expected: bool) {
        assert_eq!(device_id_is_valid(device_id).is_ok(), expected);
    }
}
