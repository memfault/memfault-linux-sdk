//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    io::Read,
    net::SocketAddr,
    path::Path,
    sync::{Arc, Mutex},
    thread::spawn,
};

use eyre::{eyre, Result};
use log::{debug, log_enabled, trace, warn};
use threadpool::ThreadPool;
use tiny_http::{Response, Server};

use crate::{
    collectd::payload::Payload,
    metrics::{InMemoryMetricStore, KeyedMetricReading},
    network::NetworkConfig,
};

/// A server that listens for collectd JSON pushes and stores them in memory.
#[derive(Clone)]
pub struct CollectdServer {
    metrics_store: Arc<Mutex<InMemoryMetricStore>>,
}

impl CollectdServer {
    pub fn new() -> Self {
        CollectdServer {
            metrics_store: Arc::new(Mutex::new(InMemoryMetricStore::new())),
        }
    }

    pub fn start(
        &self,
        data_collection_enabled: bool,
        listening_address: SocketAddr,
    ) -> Result<()> {
        let server = Server::http(listening_address).map_err(|e| {
            eyre!("Error starting server: could not bind to {listening_address}: {e}")
        })?;
        let metrics_store = self.metrics_store.clone();
        spawn(move || {
            debug!("HTTP Server started on {listening_address}");

            let pool = ThreadPool::new(4);

            for mut request in server.incoming_requests() {
                let metrics_store = metrics_store.clone();
                pool.execute(move || {
                    trace!(
                        "received request! method: {:?}, url: {:?}, headers: {:?}",
                        request.method(),
                        request.url(),
                        request.headers()
                    );
                    if request.url() == "/v1/collectd" {
                        if data_collection_enabled {
                            match Self::parse_request(&mut request.as_reader()) {
                                Ok(readings) => {
                                    let mut metrics_store = metrics_store.lock().unwrap();
                                    for reading in readings {
                                        if let Err(e) = metrics_store.add_metric(reading) {
                                            warn!("Invalid metric: {e}");
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!("Error parsing request: {}", e);
                                }
                            }
                        }

                        let _r = request.respond(Response::empty(200));
                    } else {
                        let _r = request.respond(Response::empty(404));
                    }
                })
            }
        });
        Ok(())
    }

    /// Dump the metrics to a MAR entry. This will empty the metrics store.
    pub fn dump_metrics_to_mar_entry(
        &mut self,
        mar_staging_area: &Path,
        network_config: &NetworkConfig,
    ) -> Result<()> {
        // Lock the store only long enough to create the HashMap
        let mar_builder = self
            .metrics_store
            .lock()
            .unwrap()
            .write_metrics(mar_staging_area)?;

        // Save to disk after releasing the lock
        let mar_entry = mar_builder
            .save(network_config)
            .map_err(|e| eyre!("Error building MAR entry: {}", e))?;
        debug!(
            "Generated MAR entry from CollectD metrics: {}",
            mar_entry.path.display()
        );
        Ok(())
    }

    /// Convert a collectd JSON push (Payload[]) into a list of MetricReading.
    fn parse_request(stream: &mut dyn Read) -> Result<Vec<KeyedMetricReading>> {
        let payloads: Vec<Payload> = if log_enabled!(log::Level::Debug) {
            let mut buf = vec![];
            stream.read_to_end(&mut buf)?;
            let s = String::from_utf8_lossy(&buf);
            trace!("Received JSON: {}", s);
            match serde_json::from_slice(&buf) {
                Ok(payloads) => payloads,
                Err(e) => {
                    debug!("Error parsing JSON: {}\n{}", e, String::from_utf8(buf)?);
                    return Err(eyre!("Error parsing JSON: {}", e));
                }
            }
        } else {
            serde_json::from_reader(stream)?
        };
        Ok(payloads
            .into_iter()
            .flat_map(Vec::<KeyedMetricReading>::from)
            .collect())
    }
}
