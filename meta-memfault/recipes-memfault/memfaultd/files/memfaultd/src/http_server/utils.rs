//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::{eyre, Result};
use tiny_http::Header;

/// Wraps Header.from_bytes into something that returns a Result<> compatible with eyre::Result.
pub trait ConvenientHeader {
    fn from_strings(name: &str, value: &str) -> Result<Header>;
}
impl ConvenientHeader for Header {
    fn from_strings(name: &str, value: &str) -> Result<Header> {
        Header::from_bytes(name, value).map_err(|_e| eyre!("Invalid header ({}: {})", name, value))
    }
}
