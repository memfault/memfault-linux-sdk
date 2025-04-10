//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::collections::HashMap;

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

/// Parses query parameters from a URL string
/// NOTE: Support for decoding URL encoded parameters is not currently implemented
pub fn parse_query_params(url_string: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();

    let query_string = url_string.split('?').nth(1).unwrap_or("");

    for param in query_string.split('&') {
        if param.is_empty() {
            continue;
        }

        let mut parts = param.split('=');
        let key = parts.next().unwrap_or("");
        let value = parts.next().unwrap_or("");

        params.insert(key.to_string(), value.to_string());
    }

    params
}
