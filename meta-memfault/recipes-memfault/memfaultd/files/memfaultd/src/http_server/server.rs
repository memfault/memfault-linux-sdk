//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{net::SocketAddr, thread::spawn};

use eyre::{eyre, Result};
use log::{debug, trace, warn};
use tiny_http::{Request, Response, Server};

use crate::http_server::{HttpHandler, HttpHandlerResult};

/// A server that listens for collectd JSON pushes and stores them in memory.
pub struct HttpServer {
    handlers: Option<Vec<Box<dyn HttpHandler>>>,
}

impl HttpServer {
    pub fn new(handlers: Vec<Box<dyn HttpHandler>>) -> Self {
        HttpServer {
            handlers: Some(handlers),
        }
    }

    pub fn start(&mut self, listening_address: SocketAddr) -> Result<()> {
        let server = Server::http(listening_address).map_err(|e| {
            eyre!("Error starting server: could not bind to {listening_address}: {e}")
        })?;

        if let Some(handlers) = self.handlers.take() {
            spawn(move || {
                debug!("HTTP Server started on {listening_address}");

                for request in server.incoming_requests() {
                    Self::handle_request(&handlers, request);
                }
            });
            Ok(())
        } else {
            Err(eyre!("HTTP Server already started"))
        }
    }

    fn handle_request(handlers: &[Box<dyn HttpHandler>], mut request: Request) {
        trace!(
            "HTTP request {:?} {:?}\n{:?}",
            request.method(),
            request.url(),
            request.headers()
        );

        let method = request.method().to_owned();
        let url = request.url().to_owned();
        for handler in handlers.iter() {
            match handler.handle_request(&mut request) {
                HttpHandlerResult::Response(response) => {
                    if let Err(e) = request.respond(response) {
                        warn!("HTTP: Error sending response {} {}: {:?}", method, url, e);
                    }
                    return;
                }
                HttpHandlerResult::Error(e) => {
                    warn!("HTTP: Error processing request {} {}: {}", method, url, e);
                    let _r = request
                        .respond(Response::empty(500).with_data(e.as_bytes(), Some(e.len())));
                    return;
                }
                HttpHandlerResult::NotHandled => { /* continue  */ }
            };
        }
        debug!("HTTP[404] {} {}", method, url);
        let _r = request.respond(Response::empty(404));
    }
}
