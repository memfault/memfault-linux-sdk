//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::fmt::{Debug, Display, Formatter, Result as FmtResult};
use std::str::FromStr;

use eyre::{eyre, ErrReport, Result};
use serde::{Deserialize, Serialize};

use crate::{
    reboot::reason_codes::RebootReasonCode, util::patterns::alphanum_slug_dots_colon_is_valid,
};

#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum RebootReason {
    Code(RebootReasonCode),
    Custom(RebootReasonString),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RebootReasonString {
    unexpected: bool,
    name: String,
}

impl From<RebootReasonCode> for RebootReason {
    fn from(code: RebootReasonCode) -> RebootReason {
        RebootReason::Code(code)
    }
}

impl FromStr for RebootReasonString {
    type Err = ErrReport;
    fn from_str(s: &str) -> Result<RebootReasonString> {
        // Leading '!' indicates an unexpected reboot reason
        if let Some(stripped) = s.strip_prefix('!') {
            if stripped.is_empty() {
                Err(eyre!("\"!\" on its own is not a valid reboot reaosn!"))
            } else {
                match alphanum_slug_dots_colon_is_valid(stripped, 64) {
                    Ok(()) => Ok(RebootReasonString {
                        unexpected: true,
                        name: stripped.to_string(),
                    }),
                    Err(e) => Err(e),
                }
            }
        } else {
            match alphanum_slug_dots_colon_is_valid(s, 64) {
                Ok(()) => Ok(RebootReasonString {
                    unexpected: false,
                    name: s.to_string(),
                }),
                Err(e) => Err(e),
            }
        }
    }
}

impl FromStr for RebootReason {
    type Err = ErrReport;

    fn from_str(s: &str) -> Result<RebootReason> {
        match u32::from_str(s) {
            Ok(code) => match RebootReasonCode::from_repr(code) {
                Some(reset_code) => Ok(RebootReason::Code(reset_code)),
                None => Ok(RebootReason::Code(RebootReasonCode::Unknown)),
            },
            // If the reboot reason isn't parse-able to a u32, it's custom
            Err(_) => match RebootReasonString::from_str(s) {
                Ok(reason) => Ok(RebootReason::Custom(reason)),
                Err(e) => Err(eyre!("Failed to parse custom reboot reason: {}", e)),
            },
        }
    }
}

impl Display for RebootReason {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            RebootReason::Code(c) => write!(f, "{}", (*c as u32)),
            RebootReason::Custom(RebootReasonString { unexpected, name }) => {
                if *unexpected {
                    write!(f, "!{}", name)
                } else {
                    write!(f, "{}", name)
                }
            }
        }
    }
}

impl Debug for RebootReason {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            RebootReason::Code(c) => write!(f, "{} ({})", (*c as u32), c),
            RebootReason::Custom(RebootReasonString { unexpected, name }) => write!(
                f,
                "{} ({})",
                name,
                if *unexpected {
                    "unexpected reboot"
                } else {
                    "expected reboot"
                }
            ),
        }
    }
}
