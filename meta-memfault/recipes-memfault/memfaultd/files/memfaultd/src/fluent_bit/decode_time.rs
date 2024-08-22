//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//!    This module implements a custom deserializer for FluentBit timestamps.
//!
//!    From <https://github.com/fluent/fluentd/wiki/Forward-Protocol-Specification-v1#eventtime-ext-format>
//!
//!    > EventTime uses msgpack extension format of type 0 to carry nanosecond precision of time.
//!    >
//!    >   Client MAY send EventTime instead of plain integer representation of second since unix epoch.
//!    >   Server SHOULD accept both formats of integer and EventTime.
//!    >   Binary representation of EventTime may be fixext or ext(with length 8).

use core::fmt;
use std::{collections::HashMap, fmt::Formatter};

use chrono::{DateTime, TimeZone, Utc};
use serde::{
    de::{Error, Visitor},
    Deserialize, Deserializer, Serialize, Serializer,
};
use serde_bytes::ByteBuf;

/// Deserialize a FluentBit time which can be a u32 timestamp or an EventTime
pub fn deserialize<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    let tv = TimeVisitor {};
    deserializer.deserialize_any(tv)
}

/// Serialize a FluentBit time as an EventTime
pub fn serialize<S>(time: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    // From https://github.com/fluent/fluentd/wiki/Forward-Protocol-Specification-v1#eventtime-ext-format
    let mut buf = ByteBuf::with_capacity(8);
    buf.extend_from_slice(&i32::to_be_bytes(time.timestamp() as i32));
    buf.extend_from_slice(&i32::to_be_bytes(time.timestamp_subsec_nanos() as i32));

    FluentdTimeExtType((FLUENTD_TIME_EXT_TYPE, buf)).serialize(serializer)
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename = "_ExtStruct")]
/// This is how Msgpack Ext type is represented by rmp_serde:
/// See: https://docs.rs/rmp-serde/latest/rmp_serde/constant.MSGPACK_EXT_STRUCT_NAME.html
/// And https://docs.racket-lang.org/msgpack/index.html#%28part._.Message.Pack_extension_type%29
struct FluentdTimeExtType((i8, ByteBuf));
const FLUENTD_TIME_EXT_TYPE: i8 = 0;

/// Visit a FluentBit time which can be a u32 timestamp or an EventTime
/// (extended type with nanoseconds precision).
struct TimeVisitor {}

impl<'de> Visitor<'de> for TimeVisitor {
    type Value = DateTime<Utc>;

    fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
        write!(formatter, "an integer or a Ext/FixExt with length 8")
    }

    // Called when the time is provided as an unsigned 32 bit value.
    fn visit_u32<E>(self, v: u32) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Utc.timestamp_opt(v as i64, 0)
            .single()
            .ok_or_else(|| Error::custom("Invalid timestamp"))
    }

    // Called when the time is provided as an EventTime.
    fn visit_newtype_struct<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(EventTimeVisitor {})
    }

    //   Since fluent-bit 2.1, the timestamp is provided in a seq followed by a map of optional metadata.
    //   See https://github.com/fluent/fluent-bit/issues/6666
    //   Before:
    //     (ExtType(code=0, data=b'd\xc0\xee\x7f7a\xe4\x89'), {'rand_value': 13625873794586244841})
    //   After:
    //     ((ExtType(code=0, data=b'd\xc0\xeeT79\xc5g'), {}), {'rand_value': 1235066654796201019})
    //
    //   We currently just ignore the map of metadata.
    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        let a: FluentdTimeExtType = seq.next_element()?.ok_or_else(|| {
            Error::custom("Invalid timestamp - expected an extension type with tag 0")
        })?;
        if a.0 .0 != FLUENTD_TIME_EXT_TYPE {
            return Err(Error::custom("Invalid timestamp tag"));
        }

        let ts = bytes_to_timestamp(a.0 .1)?;
        let _ignored_metadata = seq.next_element::<HashMap<String, String>>()?;
        Ok(ts)
    }
}

/// Visit a FluentBit EventTime (an extended type with nanosecond precision).
struct EventTimeVisitor {}

impl<'de> Visitor<'de> for EventTimeVisitor {
    type Value = DateTime<Utc>;
    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(formatter, " a Ext/FixExt with length 8")
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        let tag = seq.next_element::<i8>()?;
        let buf = seq.next_element::<ByteBuf>()?;

        // Validate the tag value is 0 for a timestamp.
        match (tag, buf) {
            (Some(FLUENTD_TIME_EXT_TYPE), Some(bytes)) => bytes_to_timestamp(bytes),
            (Some(tag), _) => Err(serde::de::Error::custom(format!(
                "Invalid tag {} - expected 0.",
                tag
            ))),
            _ => Err(serde::de::Error::custom("Invalid event tag.")),
        }
    }
}

/// Convert a byte buffer to a timestamp.
fn bytes_to_timestamp<E>(bytes: ByteBuf) -> Result<DateTime<Utc>, E>
where
    E: serde::de::Error,
{
    if bytes.len() == 8 {
        // We verified that bytes is long enough so bytes[0..4] will
        // never fail. It will return a [u8] of length 4.
        // We still need `.try_into()` to convert [u8] into [u8; 4]
        // because the compiler cannot verify that the length is 4 at
        // compile time. #failproof™
        let seconds_bytes: [u8; 4] = bytes[0..4].try_into().expect("Failed to extract seconds");
        let nanoseconds_bytes: [u8; 4] = bytes[4..]
            .try_into()
            .expect("Failed to extract nanoseconds");
        Utc.timestamp_opt(
            u32::from_be_bytes(seconds_bytes) as i64,
            u32::from_be_bytes(nanoseconds_bytes),
        )
        .single()
        .ok_or_else(|| Error::custom("Invalid timestamp"))
    } else {
        Err(E::custom("Invalid buffer length."))
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::deserialize;

    // This test make sure we are able to deserialize the three documentated
    // variants of time encoding (the third argument specifies the variant to
    // use).
    #[rstest]
    #[case(0, 0, serialize_fixext8)]
    #[case(1675709515, 276*1_000_000, serialize_fixext8)]
    #[case(1675709515, 276*1_000_000, serialize_varext8)]
    #[case(1675709515, 0, serialize_integer)]
    fn decode_encoded_time(
        #[case] seconds: i32,
        #[case] nanoseconds: i32,
        #[case] serialize: fn(i32, i32) -> Vec<u8>,
    ) {
        let buf = serialize(seconds, nanoseconds);
        let mut deserializer = rmp_serde::Deserializer::new(&buf[..]);
        let t = deserialize(&mut deserializer).expect("should be deserializable");

        assert_eq!(t.timestamp(), seconds as i64);
        assert_eq!(
            t.timestamp_nanos() - t.timestamp() * 1_000_000_000,
            nanoseconds as i64
        );
    }

    #[rstest]
    fn decode_ext_buffer_too_small() {
        let buf = serialize_fixext8(1675709515, 0);
        let mut deserializer = rmp_serde::Deserializer::new(&buf[..(buf.len() - 2)]);

        let e = deserialize(&mut deserializer).err().unwrap();
        assert!(e.to_string().contains("unexpected end of file"),);
    }

    #[rstest]
    fn decode_ext_invalid_tag() {
        let mut buf = serialize_fixext8(1675709515, 0);
        buf[1] = 0x42;
        let mut deserializer = rmp_serde::Deserializer::new(&buf[..]);

        let e = deserialize(&mut deserializer).err().unwrap();
        assert!(e.to_string().contains("Invalid tag"),);
    }

    fn serialize_fixext8(seconds: i32, nanoseconds: i32) -> Vec<u8> {
        // From https://github.com/fluent/fluentd/wiki/Forward-Protocol-Specification-v1#eventtime-ext-format
        let mut buf = vec![0xd7, 0x00];
        buf.extend_from_slice(&i32::to_be_bytes(seconds));
        buf.extend_from_slice(&i32::to_be_bytes(nanoseconds));
        buf
    }

    fn serialize_varext8(seconds: i32, nanoseconds: i32) -> Vec<u8> {
        // From https://github.com/fluent/fluentd/wiki/Forward-Protocol-Specification-v1#eventtime-ext-format
        let mut buf = vec![0xC7, 0x08, 0x00];
        buf.extend_from_slice(&i32::to_be_bytes(seconds));
        buf.extend_from_slice(&i32::to_be_bytes(nanoseconds));
        buf
    }

    fn serialize_integer(seconds: i32, _nanoseconds: i32) -> Vec<u8> {
        // Fluentd spec says we should support time encoded as an integer
        // https://github.com/fluent/fluentd/wiki/Forward-Protocol-Specification-v1#eventtime-ext-format
        // Integers look like this: https://github.com/msgpack/msgpack/blob/master/spec.md#int-format-family
        let mut buf = vec![0xCE];
        buf.extend_from_slice(&i32::to_be_bytes(seconds));
        buf
    }
}
