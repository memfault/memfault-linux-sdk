//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Memfaultd HTTP server
//! Used on device for communication with other programs and `memfaultctl`.
//!
//! Typically binds to 127.0.0.1 and only available locally.
//!
mod handler;
mod request_bodies;
mod server;
mod utils;

pub use handler::{HttpHandler, HttpHandlerResult};
pub use server::HttpServer;

pub use utils::{parse_query_params, ConvenientHeader};

pub use request_bodies::{MetricsRequest, SessionRequest};
