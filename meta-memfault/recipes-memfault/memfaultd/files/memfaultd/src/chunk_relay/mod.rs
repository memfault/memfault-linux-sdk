//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::{fs::File, io::Write};

use headroom::ChunksHeadroomCheck;
use hex;
use itertools::Itertools;
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
pub use headroom::ChunksHeadroomLimiter;

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
