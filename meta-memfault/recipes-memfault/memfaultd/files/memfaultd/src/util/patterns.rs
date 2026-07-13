//
// Copyright (c) Memfault, Inc.
// See License.txt for details
pub fn check_base64_encoding<T>(input: T) -> eyre::Result<()>
where
    T: AsRef<[u8]> + std::fmt::Display,
{
    use base64::{prelude::BASE64_STANDARD, Engine};
    if BASE64_STANDARD.decode(&input).is_err() {
        Err(eyre::eyre!("incorrect base64 encoding: {:.64}...", input))
    } else {
        Ok(())
    }
}

pub fn alphanum_slug_is_valid(s: &str, max_len: usize) -> eyre::Result<()> {
    match (
        (1..max_len).contains(&s.len()),
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

pub fn alphanum_slug_is_valid_and_starts_alpha(s: &str, max_len: usize) -> eyre::Result<()> {
    alphanum_slug_is_valid(s, max_len)?;
    if s.starts_with(char::is_alphabetic) {
        Ok(())
    } else {
        Err(eyre::eyre!("Must start with an alphabetic character"))
    }
}

pub fn alphanum_slug_dots_colon_is_valid(s: &str, max_len: usize) -> eyre::Result<()> {
    match (
        (1..max_len).contains(&s.len()),
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

pub fn alphanum_slug_dots_colon_spaces_parens_slash_is_valid(
    s: &str,
    max_len: usize,
) -> eyre::Result<()> {
    match ((1..max_len).contains(&s.len()), s.chars().all(|c| matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '+' | '.' | ':' | ' ' | '[' | ']' | '(' | ')' | '\\'))) {
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
    #[case("aaaaaa", false)]
    #[case("dGhpcyBzdHJpbmcgaXMgYmFzZTY0IGVuY29kZWQ=", true)]
    #[case("this string is not base64 encoded", false)]
    fn test_check_base64_encoding(#[case] input: &str, #[case] result: bool) {
        assert_eq!(check_base64_encoding::<&str>(input).is_ok(), result);
    }

    #[rstest]
    #[case("1.0.0-rc2", true)]
    #[case("qemuarm64", true)]
    #[case("1.0.0-$", false)]
    #[case("2.30^.1-rc4", false)]
    #[case("2.30\\.1-rc4", false)]
    #[case("spaces are invalid", false)]
    fn test_alphanum_slug_dots_colon_is_valid(#[case] input: &str, #[case] result: bool) {
        assert_eq!(alphanum_slug_dots_colon_is_valid(input, 64).is_ok(), result);
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
            alphanum_slug_dots_colon_spaces_parens_slash_is_valid(input, 64).is_ok(),
            result
        );
    }
}
