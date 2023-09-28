//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    fs::File,
    io::{stdout, Write},
    path::PathBuf,
};

use argh::FromArgValue;
use eyre::{Context, Result};

/// An Argh argument that can be either a `PathBuf` or a reference to `stdout` (`-`).
pub enum OutputArg {
    Stdout,
    File(PathBuf),
}

impl FromArgValue for OutputArg {
    fn from_arg_value(value: &str) -> Result<Self, String> {
        if value == "-" {
            Ok(OutputArg::Stdout)
        } else {
            Ok(OutputArg::File(value.into()))
        }
    }
}

impl OutputArg {
    /// Open the output stream designated by the user.
    pub fn get_output_stream(&self) -> Result<Box<dyn Write>> {
        let stream: Box<dyn Write> =
            match self {
                OutputArg::Stdout => Box::new(stdout()),
                OutputArg::File(path) => Box::new(File::create(path).wrap_err_with(|| {
                    format!("Error opening destination file {}", path.display())
                })?),
            };

        Ok(stream)
    }
}
