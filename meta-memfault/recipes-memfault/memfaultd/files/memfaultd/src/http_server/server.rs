//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{net::SocketAddr, sync::Arc, thread::spawn};

use eyre::{eyre, Result};
use log::{debug, trace, warn};
use threadpool::ThreadPool;
use tiny_http::{Request, Response, Server};

use crate::http_server::{HttpHandler, HttpHandlerResult};

/// A server that listens for collectd JSON pushes and stores them in memory.
#[derive(Clone)]
pub struct HttpServer {
    handlers: Arc<Vec<Box<dyn HttpHandler>>>,
}

impl HttpServer {
    pub fn new(handlers: Vec<Box<dyn HttpHandler>>) -> Self {
        HttpServer {
            handlers: Arc::new(handlers),
        }
    }

    pub fn start(&self, listening_address: SocketAddr) -> Result<()> {
        let server = Server::http(listening_address).map_err(|e| {
            eyre!("Error starting server: could not bind to {listening_address}: {e}")
        })?;
        let handlers = self.handlers.clone();
        spawn(move || {
            debug!("HTTP Server started on {listening_address}");

            let pool = ThreadPool::new(4);

            for request in server.incoming_requests() {
                let handlers = handlers.clone();
                pool.execute(move || Self::handle_request(&handlers, request))
            }
        });
        Ok(())
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
                    let _r = request.respond(Response::empty(500));
                    return;
                }
                HttpHandlerResult::NotHandled => { /* continue  */ }
            };
        }
        debug!("HTTP[404] {} {}", method, url);
        let _r = request.respond(Response::empty(404));
    }
}
