//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::borrow::Cow;
use std::io::{stdout, Write};
use std::path::Path;

use eyre::Result;

use crate::build_info::{BUILD_ID, GIT_COMMIT, VERSION};
use crate::config::{DeviceInfo, DeviceInfoWarning, JsonConfigs, MemfaultdConfig};

fn dump_config(
    writer: &mut impl Write,
    configs: &JsonConfigs,
    config_path: Option<&Path>,
) -> Result<()> {
    let path_str = config_path
        .map(Path::display)
        .map(|d| Cow::Owned(d.to_string()))
        .unwrap_or_else(|| Cow::Borrowed(MemfaultdConfig::DEFAULT_CONFIG_PATH));
    writeln!(writer, "Base configuration ({}):", path_str)?;
    writeln!(writer, "{}", serde_json::to_string_pretty(&configs.base)?)?;
    writeln!(writer)?;
    writeln!(writer, "Runtime configuration:")?;
    writeln!(
        writer,
        "{}",
        serde_json::to_string_pretty(&configs.runtime)?
    )?;
    Ok(())
}

type Device = (DeviceInfo, Vec<DeviceInfoWarning>);

fn dump_device_info(writer: &mut impl Write, device: &Device) -> Result<()> {
    let (device_info, warnings) = device;
    writeln!(writer, "Device configuration from memfault-device-info:")?;
    writeln!(writer, "  MEMFAULT_DEVICE_ID={}", device_info.device_id)?;
    writeln!(
        writer,
        "  MEMFAULT_HARDWARE_VERSION={}",
        device_info.hardware_version
    )?;

    if !warnings.is_empty() {
        writeln!(writer)?;
        for warning in warnings.iter() {
            writeln!(writer, "WARNING: {}", warning)?;
        }
    }

    Ok(())
}

struct Versions {
    version: &'static str,
    git_commit: &'static str,
    build_id: &'static str,
}

fn dump_version(writer: &mut impl Write, versions: &Versions) -> Result<()> {
    writeln!(writer, "Memfault version:")?;
    writeln!(writer, "  VERSION={}", versions.version)?;
    writeln!(writer, "  GIT COMMIT={}", versions.git_commit)?;
    writeln!(writer, "  BUILD ID={}", versions.build_id)?;
    Ok(())
}

fn dump_plugins(writer: &mut impl Write, plugins: &[&str]) -> Result<()> {
    writeln!(writer, "Plugin enabled:")?;
    for plugin in plugins {
        writeln!(writer, "  {}", plugin)?;
    }
    Ok(())
}

fn dump_settings(
    writer: &mut impl Write,
    configs: &JsonConfigs,
    config_path: Option<&Path>,
    device: &Device,
    versions: &Versions,
    plugins: &[&str],
) -> Result<()> {
    dump_config(writer, configs, config_path)?;
    writeln!(writer)?;
    dump_device_info(writer, device)?;
    writeln!(writer)?;
    dump_version(writer, versions)?;
    writeln!(writer)?;
    dump_plugins(writer, plugins)?;
    writeln!(writer)?;
    Ok(())
}

pub fn show_settings(config_path: Option<&Path>) -> Result<()> {
    let configs = MemfaultdConfig::parse_configs(config_path)?;
    let versions = Versions {
        version: VERSION,
        git_commit: GIT_COMMIT,
        build_id: BUILD_ID,
    };

    let enabled_plugins = [
        #[cfg(feature = "reboot")]
        "reboot",
        #[cfg(feature = "swupdate")]
        "swupdate",
        #[cfg(feature = "collectd")]
        "collectd",
        #[cfg(feature = "coredump")]
        "coredump",
        #[cfg(feature = "logging")]
        "logging",
    ];

    dump_settings(
        &mut stdout(),
        &configs,
        config_path,
        &DeviceInfo::load()?,
        &versions,
        &enabled_plugins,
    )
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::path::PathBuf;

    use insta::assert_snapshot;
    use serde_json::json;

    use super::*;

    #[test]
    fn test() {
        let configs = JsonConfigs {
            base: json!({"project_key": "xyz"}),
            runtime: json!({"enable_data_collection": true}),
        };
        let config_path = PathBuf::from("/etc/memfaultd.conf");

        let device =
            DeviceInfo::parse(b"MEMFAULT_DEVICE_ID=X\nMEMFAULT_HARDWARE_VERSION=Y\nblahblahblah\n")
                .unwrap();

        let versions = Versions {
            version: "1.2.3",
            git_commit: "abcdef",
            build_id: "123456",
        };

        let enabled_plugins = ["reboot", "coredump"];

        let output = Vec::new();
        let mut writer = Cursor::new(output);
        dump_settings(
            &mut writer,
            &configs,
            Some(&config_path),
            &device,
            &versions,
            &enabled_plugins,
        )
        .unwrap();

        let output = String::from_utf8(writer.into_inner()).unwrap();
        assert_snapshot!(output);
    }
}
