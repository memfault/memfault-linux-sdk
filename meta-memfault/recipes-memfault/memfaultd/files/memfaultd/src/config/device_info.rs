//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::fmt::{self, Display};
use std::fs::read_to_string;
use std::process::Command;

use eyre::{eyre, Result};

use crate::config::utils::{
    device_id_is_valid, hardware_version_is_valid, software_type_is_valid,
    software_version_is_valid,
};
use crate::util::etc_os_release::EtcOsRelease;

const DEVICE_ID_PATH: &str = "/etc/machine-id";
const HARDWARE_VERSION_COMMAND: &str = "uname";
const HARDWARE_VERSION_ARGS: &[&str] = &["-n"];

#[cfg_attr(test, mockall::automock)]
/// Trait for providing default values for device info.
///
/// This is mostly a convenience for testing, as the default implementation
/// reads the software version from /etc/os-release.
pub trait DeviceInfoDefaults {
    /// Get the software version from the system.
    fn software_version(&self) -> Result<Option<String>>;

    /// Get the device ID from the system.
    fn device_id(&self) -> Result<String>;

    /// Get the hardware version from the system.
    fn hardware_version(&self) -> Result<String>;

    /// Get the software type from the system.
    fn software_type(&self) -> Result<Option<String>>;
}

/// Default implementation of DeviceInfoDefaults.
pub struct DeviceInfoDefaultsImpl {
    os_release: Option<EtcOsRelease>,
}

impl DeviceInfoDefaultsImpl {
    fn new(os_release: Option<EtcOsRelease>) -> Self {
        Self { os_release }
    }
}

impl DeviceInfoDefaults for DeviceInfoDefaultsImpl {
    fn software_version(&self) -> Result<Option<String>> {
        Ok(self.os_release.as_ref().and_then(|os| os.version_id()))
    }

    fn device_id(&self) -> Result<String> {
        let device_id = read_to_string(DEVICE_ID_PATH)?;
        if device_id.is_empty() {
            return Err(eyre!("Empty device id ({})", DEVICE_ID_PATH));
        }

        Ok(device_id)
    }

    fn hardware_version(&self) -> Result<String> {
        let output = Command::new(HARDWARE_VERSION_COMMAND)
            .args(HARDWARE_VERSION_ARGS)
            .output()?;

        Ok(String::from_utf8(output.stdout)?)
    }

    fn software_type(&self) -> Result<Option<String>> {
        Ok(self.os_release.as_ref().and_then(|os| os.id()))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum DeviceInfoValue {
    Configured(String),
    Default(String),
}

impl AsRef<str> for DeviceInfoValue {
    fn as_ref(&self) -> &str {
        match self {
            DeviceInfoValue::Configured(s) => s.as_ref(),
            DeviceInfoValue::Default(s) => s.as_ref(),
        }
    }
}

impl Display for DeviceInfoValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_ref())
    }
}

#[derive(Debug)]
pub struct DeviceInfo {
    pub device_id: String,
    pub hardware_version: String,
    pub software_version: Option<DeviceInfoValue>,
    pub software_type: Option<DeviceInfoValue>,
}

#[derive(PartialEq, Eq, Debug)]
pub struct DeviceInfoWarning {
    line: Option<String>,
    message: String,
}

impl std::fmt::Display for DeviceInfoWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> fmt::Result {
        match &self.line {
            Some(line) => write!(f, "Skipped line: '{}' ({})", line, self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

impl DeviceInfo {
    pub fn parse<T: DeviceInfoDefaults>(
        output: Option<&[u8]>,
        defaults: T,
    ) -> Result<(DeviceInfo, Vec<DeviceInfoWarning>)> {
        let mut warnings = vec![];

        let mut software_version = defaults
            .software_version()
            .unwrap_or_else(|_| {
                warnings.push(DeviceInfoWarning {
                    line: None,
                    message: "Failed to get default software version.".to_string(),
                });
                None
            })
            .map(DeviceInfoValue::Default);
        let mut device_id = defaults.device_id().map_or_else(
            |_| {
                warnings.push(DeviceInfoWarning {
                    line: None,
                    message: format!("Failed to open {}", DEVICE_ID_PATH),
                });
                None
            },
            |id| Some(id.trim().to_string()),
        );
        let mut hardware_version = defaults.hardware_version().map_or_else(
            |_| {
                warnings.push(DeviceInfoWarning {
                    line: None,
                    message: format!(
                        "Failed to to get hardware version from: '{}'",
                        HARDWARE_VERSION_COMMAND
                    ),
                });
                None
            },
            |hwv| Some(hwv.trim().to_string()),
        );
        let mut software_type = defaults
            .software_type()
            .unwrap_or_else(|_| {
                warnings.push(DeviceInfoWarning {
                    line: None,
                    message: "Failed to get default software_type.".to_string(),
                });
                None
            })
            .map(DeviceInfoValue::Default);

        match output {
            Some(output) => {
                for line in std::str::from_utf8(output)?.lines() {
                    if let Some((key, value)) = line.split_once('=') {
                        match key {
                            "MEMFAULT_DEVICE_ID" => device_id = Some(value.into()),
                            "MEMFAULT_HARDWARE_VERSION" => hardware_version = Some(value.into()),
                            "MEMFAULT_SOFTWARE_VERSION" => {
                                software_version = Some(DeviceInfoValue::Configured(value.into()))
                            }
                            "MEMFAULT_SOFTWARE_TYPE" => {
                                software_type = Some(DeviceInfoValue::Configured(value.into()))
                            }
                            _ => warnings.push(DeviceInfoWarning {
                                line: Some(line.into()),
                                message: "Unknown variable.".to_string(),
                            }),
                        }
                    } else {
                        warnings.push(DeviceInfoWarning {
                            line: Some(line.into()),
                            message: "Expect '=' separated key/value pairs.".to_string(),
                        })
                    }
                }
            }
            None => {
                warnings.push(DeviceInfoWarning {
                    line: None,
                    message: "No output from memfault-device-info.".to_string(),
                });
            }
        }

        let di = DeviceInfo {
            device_id: device_id.ok_or(eyre!("No device id supplied"))?,
            hardware_version: hardware_version.ok_or(eyre!("No hardware version supplied"))?,
            software_version,
            software_type,
        };

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
                    .map_or(Ok(()), |swv| software_version_is_valid(swv.as_ref())),
            ),
            (
                "MEMFAULT_SOFTWARE_TYPE",
                di.software_type
                    .as_ref()
                    .map_or(Ok(()), |swt| software_type_is_valid(swt.as_ref())),
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
        let user_output = Command::new("memfault-device-info").output().ok();
        let stdout = user_output.as_ref().map(|o| o.stdout.as_slice());

        let os_release = EtcOsRelease::load().ok();
        let di_defaults = DeviceInfoDefaultsImpl::new(os_release);
        Self::parse(stdout, di_defaults)
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
            software_version: Some(DeviceInfoValue::Configured(software_version.into())),
            software_type: Some(DeviceInfoValue::Configured(software_type.into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[test]
    fn test_empty() {
        let mut di_defaults = MockDeviceInfoDefaults::new();
        di_defaults
            .expect_software_type()
            .returning(|| Err(eyre!("")));
        di_defaults.expect_software_version().returning(|| Ok(None));
        di_defaults
            .expect_hardware_version()
            .returning(|| Err(eyre!("")));
        di_defaults
            .expect_device_id()
            .returning(|| Ok("123ABC".into()));
        let r = DeviceInfo::parse(Some(b""), di_defaults);
        assert!(r.is_err())
    }

    #[test]
    fn test_with_warnings() {
        let mut di_defaults = MockDeviceInfoDefaults::new();
        di_defaults.expect_software_type().returning(|| Ok(None));
        di_defaults.expect_software_version().returning(|| Ok(None));
        di_defaults
            .expect_device_id()
            .returning(|| Ok("123ABC".into()));
        di_defaults
            .expect_hardware_version()
            .returning(|| Ok("Hardware".into()));
        let r = DeviceInfo::parse(
            Some(b"MEMFAULT_DEVICE_ID=X\nMEMFAULT_HARDWARE_VERSION=Y\nblahblahblah\n"),
            di_defaults,
        );
        assert!(r.is_ok());

        let (di, warnings) = r.unwrap();
        assert_eq!(di.device_id, "X");
        assert_eq!(di.hardware_version, "Y");
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            warnings[0],
            DeviceInfoWarning {
                line: Some("blahblahblah".into()),
                message: "Expect '=' separated key/value pairs.".to_string()
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
        let mut di_defaults = MockDeviceInfoDefaults::new();
        di_defaults.expect_software_type().returning(|| Ok(None));
        di_defaults.expect_software_version().returning(|| Ok(None));
        di_defaults
            .expect_hardware_version()
            .returning(|| Ok("Hardware".into()));
        di_defaults
            .expect_device_id()
            .returning(|| Ok("123ABC".into()));
        let r = DeviceInfo::parse(Some(output), di_defaults);
        assert!(r.is_ok());

        let (di, warnings) = r.unwrap();
        assert_eq!(di.device_id, "123ABC");
        assert_eq!(di.hardware_version, "1.0.0");
        assert_eq!(
            di.software_version,
            sw_version.map(DeviceInfoValue::Configured)
        );
        assert_eq!(di.software_type, sw_type.map(DeviceInfoValue::Configured));

        assert_eq!(warnings.len(), 0);
    }

    #[rstest]
    #[case::default_with_no_response(
        Some("1.2.3".to_string()),
        Some(DeviceInfoValue::Default("1.2.3".to_string())),
        b""
    )]
    #[case::default_with_response(
        Some("1.2.3".to_string()),
        Some(DeviceInfoValue::Configured("1.2.4".to_string())),
        b"MEMFAULT_SOFTWARE_VERSION=1.2.4"
    )]
    #[case::no_default_with_response(
        None,
        Some(DeviceInfoValue::Configured("1.2.4".to_string())),
        b"MEMFAULT_SOFTWARE_VERSION=1.2.4"
    )]
    #[case::no_default_no_response(None, None, b"")]
    fn test_with_default_swv(
        #[case] software_version_default: Option<String>,
        #[case] expected: Option<DeviceInfoValue>,
        #[case] output: &[u8],
    ) {
        // Required device info parameters that will cause a panic if not present
        let mut output_required =
            b"MEMFAULT_DEVICE_ID=DEVICE\nMEMFAULT_HARDWARE_VERSION=HARDWARE\n".to_vec();
        output_required.extend(output);

        let mut di_defaults = MockDeviceInfoDefaults::new();
        di_defaults
            .expect_software_type()
            .returning(|| Err(eyre!("")));
        di_defaults
            .expect_software_version()
            .returning(move || Ok(software_version_default.clone()));
        di_defaults
            .expect_hardware_version()
            .returning(|| Err(eyre!("")));
        di_defaults.expect_device_id().returning(|| Err(eyre!("")));

        let (di, _warnings) = DeviceInfo::parse(Some(&output_required), di_defaults).unwrap();
        assert_eq!(di.software_version, expected);
    }

    #[rstest]
    #[case::default_with_no_response(Some("123ABC".to_string()), Some(DeviceInfoValue::Default("123ABC".to_string())), b"")]
    #[case::default_with_response(Some("123ABC".to_string()), Some(DeviceInfoValue::Configured("main".to_string())), b"MEMFAULT_SOFTWARE_TYPE=main")]
    #[case::no_default_with_response(None, Some(DeviceInfoValue::Configured("main".to_string())), b"MEMFAULT_SOFTWARE_TYPE=main")]
    #[case::no_default_no_response(None, None, b"")]
    fn test_with_default_sw_type(
        #[case] software_type_default: Option<String>,
        #[case] expected: Option<DeviceInfoValue>,
        #[case] output: &[u8],
    ) {
        // Required device info parameters that will cause a panic if not present
        let mut output_required =
            b"MEMFAULT_DEVICE_ID=DEVICE\nMEMFAULT_HARDWARE_VERSION=HARDWARE\n".to_vec();
        output_required.extend(output);

        let mut di_defaults = MockDeviceInfoDefaults::new();
        di_defaults
            .expect_software_version()
            .returning(|| Err(eyre!("")));
        di_defaults
            .expect_hardware_version()
            .returning(|| Err(eyre!("")));
        di_defaults.expect_device_id().returning(|| Err(eyre!("")));
        di_defaults
            .expect_software_type()
            .returning(move || Ok(software_type_default.clone()));

        let (di, _warnings) = DeviceInfo::parse(Some(&output_required), di_defaults).unwrap();
        assert_eq!(di.software_type, expected);
    }

    #[rstest]
    #[case::default_with_no_response(Some("123ABC".to_string()), Some("123ABC".to_string()), b"")]
    #[case::default_with_whitespace(Some("123ABC\n".to_string()), Some("123ABC".to_string()), b"")]
    #[case::default_with_response(Some("123ABC".to_string()), Some("DEVICE".to_string()), b"MEMFAULT_DEVICE_ID=DEVICE")]
    #[case::no_default_with_response(None, Some("DEVICE".to_string()), b"MEMFAULT_DEVICE_ID=DEVICE")]
    #[case::no_default_no_response(None, None, b"")]
    fn test_with_default_device_id(
        #[case] device_id_default: Option<String>,
        #[case] expected: Option<String>,
        #[case] output: &[u8],
    ) {
        // Required device info parameters that will cause a panic if not present
        let mut output_required = b"MEMFAULT_HARDWARE_VERSION=HARDWARE\n".to_vec();
        output_required.extend(output);

        let mut di_defaults = MockDeviceInfoDefaults::new();
        di_defaults
            .expect_software_type()
            .returning(|| Err(eyre!("")));
        di_defaults
            .expect_software_version()
            .returning(|| Err(eyre!("")));
        di_defaults
            .expect_hardware_version()
            .returning(|| Err(eyre!("")));
        di_defaults
            .expect_device_id()
            .returning(move || device_id_default.clone().ok_or(eyre!("")));

        let ret = DeviceInfo::parse(Some(&output_required), di_defaults);
        if let Some(expected) = expected {
            let (di, _warnings) = ret.unwrap();
            assert_eq!(di.device_id, expected);
        } else {
            assert!(ret.is_err());
        }
    }

    #[rstest]
    #[case::default_with_no_response(Some("123ABC".to_string()), Some("123ABC".to_string()), b"")]
    #[case::default_with_whitespace(Some("123ABC\n".to_string()), Some("123ABC".to_string()), b"")]
    #[case::default_with_response(Some("123ABC".to_string()), Some("HARDWARE".to_string()), b"MEMFAULT_HARDWARE_VERSION=HARDWARE")]
    #[case::no_default_with_response(None, Some("HARDWARE".to_string()), b"MEMFAULT_HARDWARE_VERSION=HARDWARE")]
    #[case::no_default_no_response(None, None, b"")]
    fn test_with_default_hardware_version(
        #[case] hardware_version_default: Option<String>,
        #[case] expected: Option<String>,
        #[case] output: &[u8],
    ) {
        // Required device info parameters that will cause a panic if not present
        let mut output_required = b"MEMFAULT_DEVICE_ID=DEVICE\n".to_vec();
        output_required.extend(output);

        let mut di_defaults = MockDeviceInfoDefaults::new();
        di_defaults
            .expect_software_type()
            .returning(|| Err(eyre!("")));
        di_defaults
            .expect_software_version()
            .returning(|| Err(eyre!("")));
        di_defaults.expect_device_id().returning(|| Err(eyre!("")));
        di_defaults
            .expect_hardware_version()
            .returning(move || hardware_version_default.clone().ok_or(eyre!("")));

        let ret = DeviceInfo::parse(Some(&output_required), di_defaults);
        if let Some(expected) = expected {
            let (di, _warnings) = ret.unwrap();
            assert_eq!(di.hardware_version, expected);
        } else {
            assert!(ret.is_err());
        }
    }

    #[rstest]
    fn test_with_no_device_info() {
        let expected_software_type = "SOFTWARE_TYPE".to_string();
        let expected_software_version = "SOFTWARE_VERSION".to_string();
        let expected_hardware_version = "HARDWARE_VERSION".to_string();
        let expected_device_id = "DEVICE_ID".to_string();

        let mut di_defaults = MockDeviceInfoDefaults::new();
        let default_software_type = expected_software_type.clone();
        di_defaults
            .expect_software_type()
            .returning(move || Ok(Some(default_software_type.clone())));
        let default_software_version = expected_software_version.clone();
        di_defaults
            .expect_software_version()
            .returning(move || Ok(Some(default_software_version.clone())));
        let default_hardware_version = expected_hardware_version.clone();
        di_defaults
            .expect_hardware_version()
            .returning(move || Ok(default_hardware_version.clone()));
        let default_device_id = expected_device_id.clone();
        di_defaults
            .expect_device_id()
            .returning(move || Ok(default_device_id.clone()));

        let r = DeviceInfo::parse(None, di_defaults).unwrap();

        assert_eq!(
            r.0.software_type,
            Some(DeviceInfoValue::Default(expected_software_type))
        );
        assert_eq!(
            r.0.software_version,
            Some(DeviceInfoValue::Default(expected_software_version))
        );
        assert_eq!(r.0.hardware_version, expected_hardware_version);
        assert_eq!(r.0.device_id, expected_device_id);
    }
}
