//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Parser for /etc/os-release.
//!
//! This file is used by various Linux distributions to store information about the operating system.
//! See documentation for more details: https://www.freedesktop.org/software/systemd/man/latest/os-release.html
//! Currently only the `ID` and `VERSION_ID` fields are parsed. Below is an example of the file:
//!
//! ```text
//! PRETTY_NAME="Ubuntu 22.04.3 LTS"
//! NAME="Ubuntu"
//! VERSION_ID="22.04"
//! VERSION="22.04.3 LTS (Jammy Jellyfish)"
//! VERSION_CODENAME=jammy
//! ID=ubuntu
//! ID_LIKE=debian
//! HOME_URL="https://www.ubuntu.com/"
//! SUPPORT_URL="https://help.ubuntu.com/"
//! BUG_REPORT_URL="https://bugs.launchpad.net/ubuntu/"
//! PRIVACY_POLICY_URL="https://www.ubuntu.com/legal/terms-and-policies/privacy-policy"
//! UBUNTU_CODENAME=jammy
//! ```

use eyre::Result;
use nom::{
    bytes::complete::{tag, take_until},
    character::complete::not_line_ending,
    sequence::separated_pair,
    IResult,
};

use std::collections::HashMap;

const OS_RELEASE_PATH: &str = "/etc/os-release";
const OS_RELEASE_ID: &str = "ID";
const OS_RELEASE_VERSION_ID: &str = "VERSION_ID";

pub struct EtcOsRelease {
    id: Option<String>,
    version_id: Option<String>,
}

impl EtcOsRelease {
    pub fn load() -> Result<Self> {
        let etc_os_release_str = std::fs::read_to_string(OS_RELEASE_PATH)?;
        Self::parse(&etc_os_release_str)
    }

    pub fn id(&self) -> Option<String> {
        self.id.clone()
    }

    pub fn version_id(&self) -> Option<String> {
        self.version_id.clone()
    }

    fn parse(etc_os_release_str: &str) -> Result<Self> {
        let parsed_map = etc_os_release_str
            .trim()
            .lines()
            .filter_map(|line| {
                let parse_result: IResult<&str, (&str, &str)> =
                    separated_pair(take_until("="), tag("="), not_line_ending)(line);

                // Silently ignore lines that don't parse correctly.
                parse_result.map(|(_, (key, value))| (key, value)).ok()
            })
            .collect::<HashMap<&str, &str>>();

        let id = parsed_map
            .get(OS_RELEASE_ID)
            .map(|s| s.trim_matches('\"').to_string());
        let version_id = parsed_map
            .get(OS_RELEASE_VERSION_ID)
            .map(|s| s.trim_matches('\"').to_string());

        Ok(Self { id, version_id })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use rstest::rstest;

    #[rstest]
    #[case::version_with_quotes("VERSION_ID=\"12.1\"\nNAME=\"Ubuntu\"", Some("12.1".to_string()))]
    #[case::version_without_quotes("VERSION_ID=12.1\nNAME=\"Ubuntu\"", Some("12.1".to_string()))]
    #[case::no_version("BAD_ID=\"12.1\"\nNAME=\"Ubuntu\"", None)]
    fn test_version_id_parse(#[case] etc_os_release: &str, #[case] expected: Option<String>) {
        let os_release = EtcOsRelease::parse(etc_os_release).unwrap();

        assert_eq!(os_release.version_id(), expected);
    }

    #[rstest]
    #[case::id_with_quotes("ID=\"ubuntu\"\nVERSION_ID=\"12.1\"", Some("ubuntu".to_string()))]
    #[case::id_without_quotes("ID=ubuntu\nVERSION_ID=\"12.1\"", Some("ubuntu".to_string()))]
    #[case::no_id("BAD_ID=ubuntu\nVERSION_ID=\"12.1\"", None)]
    fn test_id_parse(#[case] etc_os_release: &str, #[case] expected: Option<String>) {
        let os_release = EtcOsRelease::parse(etc_os_release).unwrap();

        assert_eq!(os_release.id(), expected);
    }
}
