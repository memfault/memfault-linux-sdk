//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::io::{copy, BufWriter};

use crate::config::Config;
use eyre::{eyre, Context, Result};

use crate::cli::memfaultctl::ExportArgs;

use crate::cli::memfaultd_client::{ExportDeleteResponse, ExportGetResponse, MemfaultdClient};

pub fn export(config: &Config, args: &ExportArgs) -> Result<()> {
    let client = MemfaultdClient::from_config(config)?;

    let delete_token = match client
        .export_get(&args.format)
        .wrap_err("Unable to fetch latest export")?
    {
        ExportGetResponse::NoData => {
            eprintln!("Nothing to export right now. You may want to try `memfaultctl sync`.");
            return Ok(());
        }
        ExportGetResponse::Data {
            delete_token,
            mut data,
        } => {
            let mut file = BufWriter::new(args.output.get_output_stream()?);
            copy(&mut data, &mut file).wrap_err("Unable to write server response")?;
            delete_token
        }
    };

    if !args.do_not_delete {
        match client
            .export_delete(delete_token)
            .wrap_err("Error while deleting data")?
        {
            ExportDeleteResponse::Ok => {
                eprintln!("Export saved and data cleared from memfaultd.");
                Ok(())
            }
            ExportDeleteResponse::ErrorWrongDeleteToken => {
                Err(eyre!("Unexpected response: wrong hash"))
            }
            ExportDeleteResponse::Error404 => {
                Err(eyre!("Unexpected response: 404 (no data to delete)"))
            }
        }
    } else {
        eprintln!("Export saved. Data preserved in memfaultd.");
        Ok(())
    }
}
