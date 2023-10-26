//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::fmt;
use std::process::Command;

use crate::config::utils::{
    device_id_is_valid, hardware_version_is_valid, software_type_is_valid,
    software_version_is_valid,
};

#[derive(Debug)]
pub struct DeviceInfo {
    pub device_id: String,
    pub hardware_version: String,
    pub software_version: Option<String>,
    pub software_type: Option<String>,
}

#[derive(PartialEq, Eq, Debug)]
pub struct DeviceInfoWarning {
    line: String,
    message: &'static str,
}

impl std::fmt::Display for DeviceInfoWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Skipped line: '{}' ({})", self.line, self.message)
    }
}

impl DeviceInfo {
    pub fn parse(output: &[u8]) -> eyre::Result<(DeviceInfo, Vec<DeviceInfoWarning>)> {
        let mut warnings = vec![];

        let mut di = DeviceInfo {
            device_id: String::new(),
            hardware_version: String::new(),
            software_version: None,
            software_type: None,
        };

        for line in std::str::from_utf8(output)?.lines() {
            if let Some((key, value)) = line.split_once('=') {
                match key {
                    "MEMFAULT_DEVICE_ID" => di.device_id = value.into(),
                    "MEMFAULT_HARDWARE_VERSION" => di.hardware_version = value.into(),
                    "MEMFAULT_SOFTWARE_VERSION" => di.software_version = Some(value.into()),
                    "MEMFAULT_SOFTWARE_TYPE" => di.software_type = Some(value.into()),
                    _ => warnings.push(DeviceInfoWarning {
                        line: line.into(),
                        message: "Unknown variable.",
                    }),
                }
            } else {
                warnings.push(DeviceInfoWarning {
                    line: line.into(),
                    message: "Expect '=' separated key/value pairs.",
                })
            }
        }

        // Create vector of keys whose values have invalid characters
        let validation_errors: Vec<String> = [
            (
                "MEMFAULT_HARDWARE_VERSION",
                hardware_version_is_valid(&di.hardware_version),
            ),
            (
                "MEMFAULT_SOFTWARE_VERSION",
                di.software_version
                    .as_ref()
                    .map_or(Ok(()), |swv| software_version_is_valid(swv)),
            ),
            (
                "MEMFAULT_SOFTWARE_TYPE",
                di.software_type
                    .as_ref()
                    .map_or(Ok(()), |swt| software_type_is_valid(swt)),
            ),
            ("MEMFAULT_DEVICE_ID", device_id_is_valid(&di.device_id)),
        ]
        .iter()
        .filter_map(|(key, result)| match result {
            Err(e) => Some(format!("  Invalid {}: {}", key, e)),
            _ => None,
        })
        .collect();

        match validation_errors.is_empty() {
            true => Ok((di, warnings)),
            false => Err(eyre::eyre!("\n{}", validation_errors.join("\n"))),
        }
    }

    pub fn load() -> eyre::Result<(DeviceInfo, Vec<DeviceInfoWarning>)> {
        let user_output = Command::new("memfault-device-info").output()?;
        Self::parse(&user_output.stdout)
    }
}

#[cfg(test)]
impl DeviceInfo {
    pub fn test_fixture() -> Self {
        DeviceInfo {
            device_id: "001".to_owned(),
            hardware_version: "DVT".to_owned(),
            software_version: None,
            software_type: None,
        }
    }

    pub fn test_fixture_with_overrides(software_version: &str, software_type: &str) -> Self {
        DeviceInfo {
            device_id: "001".to_owned(),
            hardware_version: "DVT".to_owned(),
            software_version: Some(software_version.into()),
            software_type: Some(software_type.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[test]
    fn test_empty() {
        let r = DeviceInfo::parse(b"");
        assert!(r.is_err())
    }

    #[test]
    fn test_with_warnings() {
        let r =
            DeviceInfo::parse(b"MEMFAULT_DEVICE_ID=X\nMEMFAULT_HARDWARE_VERSION=Y\nblahblahblah\n");
        assert!(r.is_ok());

        let (di, warnings) = r.unwrap();
        assert_eq!(di.device_id, "X");
        assert_eq!(di.hardware_version, "Y");
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            warnings[0],
            DeviceInfoWarning {
                line: "blahblahblah".into(),
                message: "Expect '=' separated key/value pairs."
            }
        );
    }

    #[rstest]
    // Override software version
    #[case(b"MEMFAULT_DEVICE_ID=123ABC\nMEMFAULT_HARDWARE_VERSION=1.0.0\nMEMFAULT_SOFTWARE_VERSION=1.2.3\n", Some("1.2.3".into()), None)]
    // Override software type
    #[case(b"MEMFAULT_DEVICE_ID=123ABC\nMEMFAULT_HARDWARE_VERSION=1.0.0\nMEMFAULT_SOFTWARE_TYPE=test\n", None, Some("test".into()))]
    // Override both software version and type
    #[case(b"MEMFAULT_DEVICE_ID=123ABC\nMEMFAULT_HARDWARE_VERSION=1.0.0\nMEMFAULT_SOFTWARE_VERSION=1.2.3\nMEMFAULT_SOFTWARE_TYPE=test\n", Some("1.2.3".into()), Some("test".into()))]
    fn test_with_sw_version_and_type(
        #[case] output: &[u8],
        #[case] sw_version: Option<String>,
        #[case] sw_type: Option<String>,
    ) {
        let r = DeviceInfo::parse(output);
        assert!(r.is_ok());

        let (di, warnings) = r.unwrap();
        assert_eq!(di.device_id, "123ABC");
        assert_eq!(di.hardware_version, "1.0.0");
        assert_eq!(di.software_version, sw_version);
        assert_eq!(di.software_type, sw_type);

        assert_eq!(warnings.len(), 0);
    }
}
