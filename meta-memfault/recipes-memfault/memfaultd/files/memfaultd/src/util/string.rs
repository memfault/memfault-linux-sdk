//
// Copyright (c) Memfault, Inc.
// See License.txt for details
/// Simple implementation of remove-C-style-comments
/// Saves code space by not relying on an external library.
pub fn remove_comments(config_string: &str) -> String {
    let mut data = String::from(config_string);
    while let Some(index) = data.find("/*") {
        if let Some(index_end) = data.find("*/") {
            data = String::from(&data[..index]) + &data[index_end + 2..];
        } else {
            // No matching close. Keep everything
            break;
        }
    }
    data
}

pub trait Ellipsis {
    fn truncate_with_ellipsis(&mut self, len_bytes: usize);
}

impl Ellipsis for String {
    fn truncate_with_ellipsis(&mut self, len_bytes: usize) {
        const ELLIPSIS: &str = "…"; // Note: 3 bytes in UTF-8
        let max_len_bytes = len_bytes - ELLIPSIS.len();
        if self.len() <= max_len_bytes {
            return;
        }
        let idx = (0..=max_len_bytes)
            .rev()
            .find(|idx| self.is_char_boundary(*idx))
            .unwrap_or(0);
        self.truncate(idx);
        self.push_str(ELLIPSIS);
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use rstest::rstest;

    #[test]
    fn test_remove_comments() {
        assert_eq!(remove_comments(""), "");
        assert_eq!(remove_comments("hello world"), "hello world");
        assert_eq!(remove_comments("hello /* comment */ world"), "hello  world");
        assert_eq!(
            remove_comments("hello /* comment world"),
            "hello /* comment world"
        );
        assert_eq!(
            remove_comments("hello /* comment */world/* comment */"),
            "hello world"
        );
    }

    #[rstest]
    // No truncation:
    #[case("foobar", 10, "foobar")]
    // Truncation basic:
    #[case("foobar", 6, "foo…")]
    // Truncation inside a multi-byte character (smiling pile of poo is 4 bytes):
    #[case("f💩bar", 6, "f…")]
    // Panic: len_bytes too short to fit ellipsis:
    #[should_panic]
    #[case("foobar", 0, "…")]
    fn truncate_with_ellipsis(
        #[case] input: &str,
        #[case] len_bytes: usize,
        #[case] expected: &str,
    ) {
        let mut s = String::from(input);
        s.truncate_with_ellipsis(len_bytes);
        assert_eq!(&s, expected);
    }
}
