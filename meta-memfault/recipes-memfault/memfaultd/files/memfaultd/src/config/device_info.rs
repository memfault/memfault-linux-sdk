//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::fmt;
use std::process::Command;

#[derive(Debug)]
pub struct DeviceInfo {
    pub device_id: String,
    pub hardware_version: String,
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
    fn parse(output: &[u8]) -> eyre::Result<(DeviceInfo, Vec<DeviceInfoWarning>)> {
        let mut warnings = vec![];

        let mut di = DeviceInfo {
            device_id: String::new(),
            hardware_version: String::new(),
        };

        for line in std::str::from_utf8(output)?.lines() {
            if let Some((key, value)) = line.split_once('=') {
                match key {
                    "MEMFAULT_DEVICE_ID" => di.device_id = value.into(),
                    "MEMFAULT_HARDWARE_VERSION" => di.hardware_version = value.into(),
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
        match (di.device_id.is_empty(), di.hardware_version.is_empty()) {
            (true, true) => Err(eyre::eyre!(
                "Missing both MEMFAULT_DEVICE_ID and MEMFAULT_HARDWARE_VERSION."
            )),
            (false, true) => Err(eyre::eyre!("Missing MEMFAULT_HARDWARE_VERSION.")),
            (true, false) => Err(eyre::eyre!("Missing MEMFAULT_DEVICE_ID.")),
            (false, false) => Ok((di, warnings)),
        }
    }

    pub fn load() -> eyre::Result<(DeviceInfo, Vec<DeviceInfoWarning>)> {
        let user_output = Command::new("memfault-device-info").output()?;
        Self::parse(&user_output.stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}