//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub fn serialize<S>(size: &usize, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if size % 1024 != 0 {
        return Err(serde::ser::Error::custom(
            "Cannot serialize non-multiple of 1024 to kib.",
        ));
    }
    (size / 1024).serialize(serializer)
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: Deserializer<'de>,
{
    let size = usize::deserialize(deserializer)?;
    Ok(size * 1024)
}

#[cfg(test)]
mod tests {

    #[test]
    fn serialize_error() {
        let mut serializer = serde_json::Serializer::new(std::io::stdout());
        let r = super::serialize(&1025, &mut serializer);
        assert!(r.is_err());
    }

    #[test]
    fn serialize_multiple_of_1024() {
        let mut buf = Vec::new();
        let mut serializer = serde_json::Serializer::new(&mut buf);
        let r = super::serialize(&43008, &mut serializer);
        assert!(r.is_ok());

        assert_eq!(&buf, b"42");
    }
}
