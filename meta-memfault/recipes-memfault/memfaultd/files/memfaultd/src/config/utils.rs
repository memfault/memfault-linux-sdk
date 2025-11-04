//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::path::Path;

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

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

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
