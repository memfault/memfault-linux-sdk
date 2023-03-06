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

#[cfg(test)]
mod test {
    use super::*;

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
}
