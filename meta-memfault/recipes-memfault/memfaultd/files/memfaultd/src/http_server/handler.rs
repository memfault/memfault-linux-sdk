//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::Result;
use tiny_http::{Request, ResponseBox};

/// Return type of the HttpHandler
pub enum HttpHandlerResult {
    /// Request was processed and response is proposed.
    Response(ResponseBox),
    /// An error occured while processing the request (will return 500).
    Error(String),
    /// Request not handled by this handler. Continue to next handler.
    NotHandled,
}

/// This little helper makes it possible to use the ? operator in handlers when
/// you have already checked method and path and know that they should handle
/// the request, possibly failing while doing so.
///
/// ```
/// # use eyre::Result;
/// use tiny_http::{Request, Response, ResponseBox};
/// use memfaultd::http_server::{HttpHandler, HttpHandlerResult};
///
/// struct CounterHandler {
///   counter: u32,
/// };
///
/// impl CounterHandler {
///   fn handle_read(&self) -> Result<ResponseBox> {
///     Ok(Response::from_string("42").boxed())
///   }
/// }
///
/// impl HttpHandler for CounterHandler {
///   fn handle_request(&self, r: &mut Request) -> HttpHandlerResult {
///     if r.url() == "/count" {
///       self.handle_read().into()
///     }
///     else {
///       HttpHandlerResult::NotHandled
///     }
///   }
/// }
/// ```
impl From<Result<ResponseBox>> for HttpHandlerResult {
    fn from(r: Result<ResponseBox>) -> Self {
        match r {
            Ok(response) => HttpHandlerResult::Response(response),
            Err(e) => HttpHandlerResult::Error(e.to_string()),
        }
    }
}

/// An HttpHandler can handle a request and send a response.
pub trait HttpHandler: Send {
    /// Handle a request and prepares the response.
    ///
    /// ```
    /// # use eyre::Result;
    /// use tiny_http::{ Request, Response, ResponseBox };
    /// use memfaultd::http_server::{HttpHandler, HttpHandlerResult};
    ///
    /// struct CounterHandler {
    ///   counter: u32,
    /// };
    ///
    /// impl HttpHandler for CounterHandler {
    ///   fn handle_request(&self, r: &mut Request) -> HttpHandlerResult {
    ///     HttpHandlerResult::Response(Response::empty(200).boxed())
    ///   }
    /// }
    ///
    /// ```
    fn handle_request(&self, request: &mut Request) -> HttpHandlerResult;
}

#[cfg(test)]
mod tests {
    use tiny_http::ResponseBox;

    use super::HttpHandlerResult;

    impl HttpHandlerResult {
        pub fn expect(self, m: &'static str) -> ResponseBox {
            match self {
                HttpHandlerResult::Response(response) => response,
                HttpHandlerResult::Error(e) => panic!("{}: HttpHandlerResult::Error({})", m, e),
                HttpHandlerResult::NotHandled => panic!("{}: HttpHandlerResult::Nothandled", m),
            }
        }
    }
}
