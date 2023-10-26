//
// Copyright (c) Memfault, Inc.
// See License.txt for details
pub fn software_type_is_valid(s: &str) -> eyre::Result<()> {
    alphanum_slug_dots_colon_is_valid(s)
}

pub fn software_version_is_valid(s: &str) -> eyre::Result<()> {
    alphanum_slug_dots_colon_spaces_parens_slash_is_valid(s)
}

pub fn hardware_version_is_valid(s: &str) -> eyre::Result<()> {
    alphanum_slug_dots_colon_is_valid(s)
}

pub fn device_id_is_valid(id: &str) -> eyre::Result<()> {
    alphanum_slug_is_valid(id)
}

fn alphanum_slug_is_valid(s: &str) -> eyre::Result<()> {
    match (
        (1..128).contains(&s.len()),
        s.chars()
            .all(|c| matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' )),
    ) {
        (true, true) => Ok(()),
        (false, _) => Err(eyre::eyre!("Must be with 1 and 128 characters long")),
        (_, false) => Err(eyre::eyre!(
            "Must only contain alphanumeric characters and - or _"
        )),
    }
}

fn alphanum_slug_dots_colon_is_valid(s: &str) -> eyre::Result<()> {
    match (
        (1..128).contains(&s.len()),
        s.chars()
            .all(|c| matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '+' | '.' | ':')),
    ) {
        (true, true) => Ok(()),
        (false, _) => Err(eyre::eyre!("Must be with 1 and 128 characters long")),
        (_, false) => Err(eyre::eyre!(
            "Must only contain alphanumeric characters, -,_,+,., and :"
        )),
    }
}

fn alphanum_slug_dots_colon_spaces_parens_slash_is_valid(s: &str) -> eyre::Result<()> {
    match ((1..128).contains(&s.len()), s.chars().all(|c| matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '+' | '.' | ':' | ' ' | '[' | ']' | '(' | ')' | '\\'))) {
        (true, true) => Ok(()),
        (false, _) => Err(eyre::eyre!("Must be with 1 and 128 characters long")),
        (_, false) => Err(eyre::eyre!("Must only contain alphanumeric characters, spaces, -,_,+,.,:,[,],(,), and \\")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case("1.0.0-rc2", true)]
    #[case("qemuarm64", true)]
    #[case("1.0.0-$", false)]
    #[case("2.30^.1-rc4", false)]
    #[case("2.30\\.1-rc4", false)]
    #[case("spaces are invalid", false)]
    fn test_alphanum_slug_dots_colon_is_valid(#[case] input: &str, #[case] result: bool) {
        assert_eq!(alphanum_slug_dots_colon_is_valid(input).is_ok(), result);
    }

    #[rstest]
    #[case("1.0.0-rc2", true)]
    #[case("qemuarm64", true)]
    #[case("1.0.0-$", false)]
    #[case("2.30^.1-rc4", false)]
    #[case("2.30\\.1-rc4", true)]
    #[case("spaces are valid", true)]
    fn test_alphanum_slug_dots_colon_spaces_parens_slash_is_valid(
        #[case] input: &str,
        #[case] result: bool,
    ) {
        assert_eq!(
            alphanum_slug_dots_colon_spaces_parens_slash_is_valid(input).is_ok(),
            result
        );
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
