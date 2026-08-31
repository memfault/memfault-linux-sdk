//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::{fs::File, io::Write};

use base64::{prelude::BASE64_STANDARD, Engine};
use hex;
use itertools::Itertools;
use nom::{
    bytes::complete::{tag, take_until},
    character::complete::anychar,
    multi::many_till,
    sequence::preceded,
    IResult,
};
use sha2::{Digest, Sha256};

use eyre::{eyre, Context, Result};
use log::{debug, warn};
use ssf::{Handler, Service};

mod messages;
pub use messages::{PrepareMarEntriesMsg, RelayChunksMsg};

use crate::mar::{MarConfig, MarEntryBuilder, Metadata};
use crate::network::NetworkConfig;

mod handler;
pub use handler::ChunkRelayHttpHandler;

mod headroom;
pub use headroom::{ChunksHeadroomCheck, ChunksHeadroomLimiter};

use crate::util::patterns::check_base64_encoding;

pub struct ChunkRelayService {
    tmp_path: PathBuf,
    headroom_limiter: ChunksHeadroomLimiter,
    project_key: String,
}

impl ChunkRelayService {
    fn new(
        tmp_path: PathBuf,
        headroom_limiter: ChunksHeadroomLimiter,
        project_key: String,
    ) -> Self {
        Self {
            tmp_path,
            headroom_limiter,
            project_key,
        }
    }

    pub fn open(
        tmp_path: PathBuf,
        headroom_limiter: ChunksHeadroomLimiter,
        project_key: String,
    ) -> Result<Self> {
        fs::create_dir_all(&tmp_path).wrap_err_with(|| {
            format!(
                "Unable to create directory to store chunks: {}",
                tmp_path.display()
            )
        })?;
        Ok(Self::new(tmp_path, headroom_limiter, project_key))
    }

    /// a file identifier for a particular project_key + device_serial pair.
    /// sha256 encoding of project_key + ":_SERIAL_:" + device_serial
    fn file_id(project_key: &str, device_serial: &str) -> String {
        // can easily change encoding
        let bytes = Sha256::digest(String::from(project_key) + ":_SERIAL_:" + device_serial);
        hex::encode(bytes)
    }

    fn file(&mut self, project_key: Option<String>, device_serial: Option<String>) -> Result<File> {
        let project_key = project_key.unwrap_or(self.project_key.clone());
        let device_serial = device_serial.unwrap_or_else(|| String::from("_SRL"));

        let file_id = Self::file_id(&project_key, &device_serial);

        let path = self.tmp_path.join(file_id).with_extension("chunks");

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;

        if file.metadata()?.len() == 0 {
            writeln!(file, "{project_key}")?;
            writeln!(file, "{device_serial}")?;
        }
        Ok(file)
    }

    fn store_all_mar_entries(
        &mut self,
        network_config: &NetworkConfig,
        mar_config: &MarConfig,
    ) -> Result<()> {
        let staging_area = mar_config.tmp_staging_path();

        for entry in fs::read_dir(&self.tmp_path)? {
            match entry {
                Ok(chunks_path) => {
                    self.store_mar_entry(
                        chunks_path.path(),
                        &staging_area,
                        network_config,
                        mar_config,
                    )?;
                }
                Err(e) => {
                    return Err(eyre!(
                        "Invalid MAR entry found in chunks temporary path: {}",
                        e
                    ))
                }
            }
        }

        Ok(())
    }

    fn store_mar_entry(
        &mut self,
        chunks_path: PathBuf,
        staging_area: &Path,
        network_config: &NetworkConfig,
        mar_config: &MarConfig,
    ) -> Result<()> {
        let chunks_filename = chunks_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| eyre!("Invalid chunks filename: {}", chunks_path.display()))?
            .to_owned();

        let file = File::open(&chunks_path)?;
        let (key, serial) = BufReader::new(file)
            .lines()
            .take(2)
            .collect_tuple()
            .ok_or(eyre!("couldn't read first two lines of chunks file"))?;

        let mar_entry = MarEntryBuilder::new(staging_area)?
            .set_metadata(Metadata::new_chunks(chunks_filename, serial?, key?))
            .add_attachment(chunks_path)
            .map_err(|e| eyre!("Failed to stage chunks file: {}", e))?
            .save(network_config, mar_config)
            .map_err(|e| eyre!("Error building MAR entry: {}", e))?;

        debug!(
            "Generated MAR entry from chunks: {}",
            mar_entry.path.display()
        );

        Ok(())
    }
}

impl Service for ChunkRelayService {
    fn name(&self) -> &str {
        "ChunkRelayService"
    }
}

impl Handler<RelayChunksMsg> for ChunkRelayService {
    fn deliver(&mut self, m: RelayChunksMsg) -> <RelayChunksMsg as ssf::Message>::Reply {
        let mut file = match self.file(m.project_key, m.device_serial) {
            Ok(file) => file,
            Err(e) => {
                log::error!("Chunk relay: failed to open chunk file: {e:?}");
                return Err(e);
            }
        };

        for chunk in m.chunks.iter() {
            // check if there's enough space to begin with
            if !self.headroom_limiter.check_with_added_space(chunk.len())? {
                let errmsg = "ran out of space while writing chunks";
                warn!("{}", errmsg);
                return Err(eyre!("{}", errmsg));
            };
            if let Err(errmsg) = check_base64_encoding(chunk) {
                warn!("{}", errmsg);
            }
            if let Err(e) = writeln!(file, "MC:{}:", chunk) {
                log::error!("Chunk relay: failed to write chunk to file: {e}");
                return Err(e.into());
            }
        }

        Ok(())
    }
}

impl Handler<PrepareMarEntriesMsg> for ChunkRelayService {
    fn deliver(
        &mut self,
        m: PrepareMarEntriesMsg,
    ) -> <PrepareMarEntriesMsg as ssf::Message>::Reply {
        let network_config = m.network_config();
        let mar_config = m.mar_config();

        self.store_all_mar_entries(network_config, mar_config)
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum ChunksEncoding {
    Base64,
    Hex,
    Bin,
    SdkDataExport,
}

impl ChunksEncoding {
    pub fn to_base64(self, chunk: &String) -> Result<Vec<String>> {
        Ok(match self {
            ChunksEncoding::Base64 => match BASE64_STANDARD.decode(chunk) {
                Err(e) => return Err(eyre!("incorrect base64 encoding: {:.64}... ({})", chunk, e)),
                Ok(_) => vec![chunk.clone()],
            },
            ChunksEncoding::Hex => match hex::decode(chunk) {
                Err(e) => return Err(eyre!("incorrect hex encoding: {:.64}... ({})", chunk, e)),
                Ok(res) => vec![BASE64_STANDARD.encode(res)],
            },

            // last two are files to read
            ChunksEncoding::Bin => match fs::read(chunk) {
                Err(e) => return Err(eyre!("unable to read bin file: {:.64}... ({})", chunk, e)),
                Ok(bytes) => vec![BASE64_STANDARD.encode(bytes)],
            },
            ChunksEncoding::SdkDataExport => match fs::read_to_string(chunk) {
                Err(e) => {
                    return Err(eyre!(
                        "unable to read SDK data export file: {:.64}... ({})",
                        chunk,
                        e
                    ))
                }
                Ok(s) => s.lines().map(parse_mc_string).collect::<Result<Vec<_>>>()?,
            },
        })
    }
}

fn parse_mc_string(line: &str) -> Result<String> {
    let (_, parsed) = parse_mc_string_to_iresult(line)
        .map_err(|_e| eyre!("Failed to parse MC string: {}", line))?;
    check_base64_encoding(parsed)?;
    Ok(parsed.into())
}

fn parse_mc_string_to_iresult(input: &str) -> IResult<&str, &str> {
    preceded(many_till(anychar, tag("MC:")), take_until(":"))(input)
}

#[cfg(test)]
mod test {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case("MC:asdf:", "asdf")]
    #[case("mflt: MC:asdf:", "asdf")]
    #[case("I [211239845]: mflt: MC:asdf:", "asdf")]
    #[case("I [211239845] mflt: MC:asdf:", "asdf")]
    #[case("I [211239845]        mflt: MC:asdf:", "asdf")]
    #[case("I          [211239845]        mflt: MC:asdf:", "asdf")]
    fn test_parses_valid_mc_string(#[case] line: &str, #[case] expected: String) {
        let parsed = parse_mc_string(line).expect("valid MC string should parse");
        assert_eq!(parsed, expected);
    }

    #[rstest]
    #[case("6869", "aGk=")]
    fn test_translates_valid_hex_string(#[case] line: String, #[case] expected: String) {
        let translated = ChunksEncoding::Hex
            .to_base64(&line)
            .expect("valid hex encoding");
        assert_eq!(translated, vec![expected]);
    }

    #[rstest]
    #[case("aGk=")]
    #[case("asdf")]
    fn test_base64_left_unchanged(#[case] line: String) {
        let translated = ChunksEncoding::Base64
            .to_base64(&line)
            .expect("valid base64 encoding");
        assert_eq!(translated, vec![line]);
    }

    #[rstest]
    #[case("6869686968", ChunksEncoding::Base64)]
    #[case("aGk=", ChunksEncoding::Hex)]
    fn test_mismatched_format_errors(#[case] line: String, #[case] encoding: ChunksEncoding) {
        assert!(encoding.to_base64(&line).is_err());
    }
}
