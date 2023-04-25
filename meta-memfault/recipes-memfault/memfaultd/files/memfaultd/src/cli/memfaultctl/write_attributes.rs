//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use crate::mar::manifest::DeviceAttribute;
use crate::metrics::MetricStringKey;
use argh::FromArgValue;
use serde_json::Value;
use std::str::FromStr;

impl FromArgValue for DeviceAttribute {
    fn from_arg_value(value: &str) -> Result<Self, String> {
        let (key, value_str) = value
            .split_once('=')
            .ok_or("Each attribute should be specified as KEY=VALUE")?;

        // Let's ensure the key is valid first:
        let metric_key = MetricStringKey::from_str(key)?;
        let value = match serde_json::from_str::<Value>(value_str) {
            Ok(value) => {
                if value.is_array() || value.is_object() {
                    return Err("Invalid value: arrays or objects are not allowed".to_string());
                }
                value
            }
            // Default to string value:
            Err(_) => value_str.into(),
        };
        Ok(DeviceAttribute::new(metric_key, value))
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("")]
    #[case("KEY")]
    #[case("KEY:VALUE")]
    fn split_failed(#[case] input: &str) {
        assert!(DeviceAttribute::from_arg_value(input)
            .err()
            .unwrap()
            .contains("Each attribute should be specified as KEY=VALUE"));
    }

    #[rstest]
    fn invalid_key() {
        assert_eq!(
            DeviceAttribute::from_arg_value("\u{1F4A9}=smelly")
                .err()
                .unwrap(),
            "Invalid key: must be ASCII"
        );
    }

    #[rstest]
    #[case("key=[]")]
    #[case("key={}")]
    fn invalid_value(#[case] input: &str) {
        assert_eq!(
            DeviceAttribute::from_arg_value(input).err().unwrap(),
            "Invalid value: arrays or objects are not allowed"
        );
    }

    #[rstest]
    #[case("key=", ("key", "").try_into())]
    #[case("key=my_string", ("key", "my_string").try_into())]
    #[case("key=123", ("key", 123).try_into())]
    #[case("key=123.456", ("key", 123.456).try_into())]
    #[case("key=true", ("key", true).try_into())]
    #[case("key=false", ("key", false).try_into())]
    #[case("key=\"false\"", ("key", "false").try_into())]
    #[case("key=\"[]\"", ("key", "[]").try_into())]
    #[case("key=\"{}\"", ("key", "{}").try_into())]
    fn parsed_ok(#[case] input: &str, #[case] expected: Result<DeviceAttribute, String>) {
        assert_eq!(DeviceAttribute::from_arg_value(input), expected);
    }
}
