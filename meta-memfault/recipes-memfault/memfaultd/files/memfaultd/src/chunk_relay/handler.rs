//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::{eyre, Result};
use log::error;
use ssf::MsgMailbox;
use std::{io::Read, str::from_utf8};
use tiny_http::{Method, Request, Response};

use crate::chunk_relay::RelayChunksMsg;
use crate::http_server::{ChunksRequest, HttpHandler, HttpHandlerResult};

pub struct ChunkRelayHttpHandler {
    relay_service_mbox: MsgMailbox<RelayChunksMsg>,
    data_collection_enabled: bool,
}

const MAX_BODY_BYTES: u64 = 5 * 1024;

impl ChunkRelayHttpHandler {
    pub fn new(
        relay_service_mbox: MsgMailbox<RelayChunksMsg>,
        data_collection_enabled: bool,
    ) -> Self {
        Self {
            relay_service_mbox,
            data_collection_enabled,
        }
    }

    fn parse_request(stream: &mut dyn Read) -> Result<ChunksRequest> {
        let mut buf = Vec::with_capacity((MAX_BODY_BYTES) as usize);
        let mut limited = stream.take(MAX_BODY_BYTES + 1);
        limited.read_to_end(&mut buf)?;
        if buf.len() > MAX_BODY_BYTES as usize {
            return Err(eyre!("Request body too large (max {MAX_BODY_BYTES} bytes)"));
        }
        let body = from_utf8(&buf)?;
        Ok(serde_json::from_str(body)?)
    }

    fn relay_chunks(&self, chunks_request: &ChunksRequest) -> Result<()> {
        match self
            .relay_service_mbox
            .send_and_forget(RelayChunksMsg::from(chunks_request))
        {
            Ok(()) => Ok(()),
            Err(e) => Err(eyre!("Failed to send chunks to mailbox: {:?}", e)),
        }
    }
}

impl HttpHandler for ChunkRelayHttpHandler {
    fn handle_request(&self, request: &mut Request) -> HttpHandlerResult {
        if request.url() != "/v1/chunks" || *request.method() != Method::Post {
            return HttpHandlerResult::NotHandled;
        }

        if self.data_collection_enabled {
            match Self::parse_request(request.as_reader()) {
                Ok(chunks_request) => match self.relay_chunks(&chunks_request) {
                    Ok(()) => (),
                    Err(e) => {
                        error!("Failed to relay chunks: {:?}", e);
                        return HttpHandlerResult::Error(format!(
                            "Failed to relay chunks: {:?}",
                            e
                        ));
                    }
                },
                Err(e) => {
                    error!("Failed to parse chunks relay request: {:?}", e);
                    return HttpHandlerResult::Error(format!(
                        "Failed to parse chunks relay request: {:?}",
                        e
                    ));
                }
            }
        } else {
            log::trace!("Data collection disabled, dropping chunks");
        }
        HttpHandlerResult::Response(Response::empty(200).boxed())
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use tiny_http::{Method, TestRequest};

    use crate::http_server::{HttpHandler, HttpHandlerResult};

    use super::ChunkRelayHttpHandler;

    #[rstest]
    #[case(r#"{"project_key":null,"device_serial":null,"chunks":["aGk="]}"#)]
    #[case(r#"{"project_key":"1234-5678","device_serial":"ABCDEFGHIJ","chunks":["aGk="]}"#)]
    #[case(r#"{"project_key":"1234-5678","device_serial":"ABCDEFGHIJ","chunks":["aGk=", "non-base64"]}"#)]
    fn ignores_chunks_when_data_collection_is_off(#[case] body: &'static str) {
        let mut mock = ssf::ServiceMock::new();
        let handler = ChunkRelayHttpHandler::new(mock.mbox.clone(), false);
        let r = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/v1/chunks")
            .with_body(body);
        assert!(matches!(
            handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));

        assert_eq!(mock.take_messages().len(), 0)
    }

    #[rstest]
    #[case(
        vec![
            r#"{"project_key":null,"device_serial":null,"chunks":["aGk="]}"#,
        ])]
    #[case(
        vec![
            r#"{"project_key":null,"device_serial":null,"chunks":["aGk="]}"#,
            r#"{"project_key":null,"device_serial":null,"chunks":["aGk="]}"#,
        ])]
    #[case(
        vec![
            r#"{"project_key":null,"device_serial":null,"chunks":["aGk="]}"#,
            r#"{"project_key":null,"device_serial":null,"chunks":["aGk=", "non-base64"]}"#,
            r#"{"project_key":null,"device_serial":null,"chunks":["aGk="]}"#,
        ])]
    fn accepts_chunks_when_data_collection_is_on(#[case] msgs: Vec<&'static str>) {
        let mut mock = ssf::ServiceMock::new();
        let handler = ChunkRelayHttpHandler::new(mock.mbox.clone(), true);
        for body in &msgs {
            let r = TestRequest::new()
                .with_method(Method::Post)
                .with_path("/v1/chunks")
                .with_body(body);
            assert!(matches!(
                handler.handle_request(&mut r.into()),
                HttpHandlerResult::Response(_)
            ));
        }

        assert_eq!(mock.take_messages().len(), msgs.len())
    }
}
