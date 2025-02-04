//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! fluent-bit
//!
//! Provides FluentBitConnectionHandler to handle to TCP connections from
//! fluent-bit. A threadpool is used to limit the number of active connections at
//! a given time.
//!
//! The start() function returns a multi-producer single-consumer channel in
//! which the messages will be delivered.
//!
//! Messages are deserialized into FluentdMessage instances.
//!
//! We set a limit on the number of messages in the channel. If messages are not
//! consumed, the FluentBitReceiver will start to apply backpressure on
//! fluent-bitbit server.
//!
use std::net::TcpStream;
use std::{collections::HashMap, net::SocketAddr};

use chrono::{DateTime, Utc};
use eyre::{eyre, Error, Result};
use log::warn;
use rmp_serde::Deserializer;
use serde::{Deserialize, Serialize};
use ssf::MsgMailbox;

use crate::{
    config::Config,
    logs::{
        log_collector::LogEntrySender,
        log_entry::{LogData, LogEntry},
        messages::LogEntryMsg,
    },
};
use crate::{
    logs::log_entry::LogValue,
    util::tcp_server::{TcpConnectionHandler, TcpNullConnectionHandler, ThreadedTcpServer},
};

mod decode_time;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FluentdValue {
    String(String),
    Float(f64),
}

impl FluentdValue {
    pub fn into_string(self) -> Option<String> {
        match self {
            FluentdValue::String(s) => Some(s),
            FluentdValue::Float(_) => None,
        }
    }

    pub fn into_float(self) -> Option<f64> {
        match self {
            FluentdValue::Float(f) => Some(f),
            FluentdValue::String(_) => None,
        }
    }
}

impl From<FluentdValue> for LogValue {
    fn from(value: FluentdValue) -> Self {
        match value {
            FluentdValue::String(s) => LogValue::String(s),
            FluentdValue::Float(f) => LogValue::Float(f),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FluentdMessage(
    #[serde(with = "decode_time")] pub DateTime<Utc>,
    pub HashMap<String, FluentdValue>,
);

impl TryFrom<FluentdMessage> for LogEntry {
    type Error = Error;

    fn try_from(mut value: FluentdMessage) -> Result<Self, Self::Error> {
        let message = value
            .1
            .remove("MESSAGE")
            .and_then(|v| v.into_string())
            .ok_or_else(|| eyre!("No message in log entry"))?;
        let pid = value.1.remove("_PID").and_then(|v| v.into_string());
        let systemd_unit = value
            .1
            .remove("_SYSTEMD_UNIT")
            .and_then(|v| v.into_string());
        let priority = value.1.remove("PRIORITY").and_then(|v| v.into_string());

        let extra_fields = value.1.into_iter().map(|(k, v)| (k, v.into())).collect();
        let data = LogData {
            message,
            pid,
            systemd_unit,
            priority,
            original_priority: None,
            extra_fields,
        };

        Ok(LogEntry { ts: value.0, data })
    }
}

#[derive(Clone)]
pub struct FluentBitConnectionHandler {
    sender: LogEntrySender,
    extra_fields: Vec<String>,
}

impl FluentBitConnectionHandler {
    /// Starts the fluent-bit server with a handler delivers parsed messages to a receiver channel.
    pub fn start(
        config: FluentBitConfig,
        sender: MsgMailbox<LogEntryMsg>,
        extra_fields: Vec<String>,
    ) -> Result<ThreadedTcpServer> {
        let sender = LogEntrySender::new(sender);
        let server = ThreadedTcpServer::start(
            config.bind_address,
            config.max_connections,
            FluentBitConnectionHandler {
                sender,
                extra_fields,
            },
        )?;
        Ok(server)
    }

    /// Starts the fluent-bit server with a handler that drops all data.
    /// This is used in case data collection is disabled. We want to keep servicing fluent-bit in
    /// this scenario, to avoid it retrying and buffering up data.
    pub fn start_null(config: FluentBitConfig) -> Result<ThreadedTcpServer> {
        ThreadedTcpServer::start(
            config.bind_address,
            config.max_connections,
            TcpNullConnectionHandler {},
        )
    }

    /// Convert a FluentdMessage into a serde_json::Value that we can log.
    ///
    /// Returns None when this message should be filtered out. This can happen
    /// if the message does not contain a `MESSAGE` field. Indicating that it is
    /// not a log message.
    fn convert_message(msg: FluentdMessage, extra_fields: &[String]) -> Result<LogEntry> {
        let mut log_entry = LogEntry::try_from(msg)?;
        log_entry.filter_extra_fields(extra_fields);

        Ok(log_entry)
    }
}

impl TcpConnectionHandler for FluentBitConnectionHandler {
    fn handle_connection(&self, stream: TcpStream) -> Result<()> {
        let mut de = Deserializer::new(stream);

        loop {
            match FluentdMessage::deserialize(&mut de) {
                Ok(msg) => {
                    let msg = FluentBitConnectionHandler::convert_message(msg, &self.extra_fields)?;
                    if self.sender.send_entry(msg).is_err() {
                        // An error indicates that the channel has been closed, we should
                        // kill this thread.
                        break;
                    }
                }
                Err(e) => {
                    match e {
                        rmp_serde::decode::Error::InvalidMarkerRead(e)
                            if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                        {
                            // silently ignore end of stream
                        }
                        _ => warn!("FluentD decoding error: {:?}", e),
                    }
                    // After any deserialization error, we want to kill the connection.
                    break;
                }
            }
        }
        Ok(())
    }
}

pub struct FluentBitConfig {
    bind_address: SocketAddr,
    max_connections: usize,
}

impl From<&Config> for FluentBitConfig {
    fn from(config: &Config) -> Self {
        Self {
            bind_address: config.config_file.fluent_bit.bind_address,
            max_connections: config.config_file.fluent_bit.max_connections,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::{io::Write, net::Shutdown, thread, thread::JoinHandle, time::Duration};

    use rstest::{fixture, rstest};
    use ssf::ServiceMock;

    use crate::test_utils::setup_logger;

    use super::*;

    #[rstest]
    #[cfg_attr(not(target_os = "linux"), allow(unused_variables, unused_mut))]
    fn deserialize_bogus_message(_setup_logger: (), mut connection: FluentBitFixture) {
        // this test can flake on macOS. Run it only on Linux to avoid flaking Mac CI
        #[cfg(target_os = "linux")]
        {
            connection.client.write_all("bogus".as_bytes()).unwrap();
            connection.client.shutdown(Shutdown::Both).unwrap();

            // Make sure there is nothing received
            let received = connection.service.take_messages();
            assert!(received.is_empty());

            // The handler should return without an error
            assert!(connection.thread.join().is_ok());
        }
    }

    #[rstest]
    fn deserialize_one_message(
        _setup_logger: (),
        mut connection: FluentBitFixture,
        message: FluentBitMessageFixture,
    ) {
        connection.client.write_all(&message.bytes).unwrap();
        connection.client.shutdown(Shutdown::Both).unwrap();

        thread::sleep(Duration::from_millis(5));
        // Make sure message is received
        let received = connection.service.take_messages();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].entry.ts, message.msg.0);
        assert_eq!(
            serde_json::to_string(&received[0].entry.data).unwrap(),
            serde_json::to_string(&message.msg.1).unwrap()
        );

        // The handler should return without an error
        assert!(connection.thread.join().is_ok());
    }

    #[rstest]
    fn deserialize_one_message_received_in_two_parts(
        _setup_logger: (),
        mut connection: FluentBitFixture,
        message: FluentBitMessageFixture,
    ) {
        let buf1 = &message.bytes[0..10];
        let buf2 = &message.bytes[10..];

        connection.client.write_all(buf1).unwrap();
        connection.client.flush().unwrap();
        // Make sure the other thread has time to do something
        thread::sleep(Duration::from_millis(5));
        connection.client.write_all(buf2).unwrap();
        connection.client.shutdown(Shutdown::Both).unwrap();

        thread::sleep(Duration::from_millis(5));
        // Make sure message is received
        let received = connection.service.take_messages();
        assert_eq!(received.len(), 1);

        assert_eq!(received.len(), 1);
        assert_eq!(received[0].entry.ts, message.msg.0);
        assert_eq!(
            serde_json::to_string(&received[0].entry.data).unwrap(),
            serde_json::to_string(&message.msg.1).unwrap()
        );

        // The handler should return without an error
        assert!(connection.thread.join().is_ok());
    }

    #[rstest]
    fn deserialize_two_concatenated_messages(
        _setup_logger: (),
        mut connection: FluentBitFixture,
        message: FluentBitMessageFixture,
        #[from(message)] message2: FluentBitMessageFixture,
    ) {
        let mut buf = message.bytes.clone();
        buf.extend(message2.bytes);
        connection.client.write_all(&buf).unwrap();
        connection.client.shutdown(Shutdown::Both).unwrap();

        thread::sleep(Duration::from_millis(5));
        // Make sure two messages are received
        let received = connection.service.take_messages();
        assert_eq!(received.len(), 2);
        received
            .into_iter()
            .zip([&message.msg, &message2.msg].iter())
            .for_each(|(log, msg)| {
                assert_eq!(log.entry.ts, msg.0);
                assert_eq!(
                    serde_json::to_string(&log.entry.data).unwrap(),
                    serde_json::to_string(&msg.1).unwrap()
                );
            });

        // The handler should return without an error
        assert!(connection.thread.join().is_ok());
    }

    #[rstest]
    /// Test the new data format with metadata associated to the timestamp (fluent-bit 2.1+)
    fn deserialize_timestamp_with_metadata(_setup_logger: (), mut connection: FluentBitFixture) {
        let buf = [
            0xDD, 0x00, 0x00, 0x00, 0x02, 0xDD, 0x00, 0x00, 0x00, 0x02, 0xD7, 0x00, 0x64, 0x1B,
            0xD5, 0xF9, 0x1E, 0x4A, 0xFD, 0x58, 0xDF, 0x00, 0x00, 0x00, 0x00, 0xDF, 0x00, 0x00,
            0x00, 0x15, 0xAA, 0x5F, 0x54, 0x52, 0x41, 0x4E, 0x53, 0x50, 0x4F, 0x52, 0x54, 0xA6,
            0x73, 0x74, 0x64, 0x6F, 0x75, 0x74, 0xAA, 0x5F, 0x53, 0x54, 0x52, 0x45, 0x41, 0x4D,
            0x5F, 0x49, 0x44, 0xD9, 0x20, 0x65, 0x62, 0x33, 0x37, 0x39, 0x30, 0x37, 0x62, 0x63,
            0x33, 0x34, 0x61, 0x34, 0x31, 0x64, 0x63, 0x62, 0x34, 0x61, 0x37, 0x65, 0x36, 0x63,
            0x37, 0x33, 0x61, 0x30, 0x32, 0x61, 0x66, 0x30, 0x32, 0xA8, 0x50, 0x52, 0x49, 0x4F,
            0x52, 0x49, 0x54, 0x59, 0xA1, 0x36, 0xAF, 0x53, 0x59, 0x53, 0x4C, 0x4F, 0x47, 0x5F,
            0x46, 0x41, 0x43, 0x49, 0x4C, 0x49, 0x54, 0x59, 0xA1, 0x33, 0xB1, 0x53, 0x59, 0x53,
            0x4C, 0x4F, 0x47, 0x5F, 0x49, 0x44, 0x45, 0x4E, 0x54, 0x49, 0x46, 0x49, 0x45, 0x52,
            0xA4, 0x67, 0x65, 0x74, 0x68, 0xA4, 0x5F, 0x50, 0x49, 0x44, 0xA3, 0x37, 0x37, 0x38,
            0xA4, 0x5F, 0x55, 0x49, 0x44, 0xA4, 0x31, 0x30, 0x30, 0x35, 0xA4, 0x5F, 0x47, 0x49,
            0x44, 0xA4, 0x31, 0x30, 0x30, 0x35, 0xA5, 0x5F, 0x43, 0x4F, 0x4D, 0x4D, 0xA4, 0x67,
            0x65, 0x74, 0x68, 0xA4, 0x5F, 0x45, 0x58, 0x45, 0xB3, 0x2F, 0x75, 0x73, 0x72, 0x2F,
            0x6C, 0x6F, 0x63, 0x61, 0x6C, 0x2F, 0x62, 0x69, 0x6E, 0x2F, 0x67, 0x65, 0x74, 0x68,
            0xA8, 0x5F, 0x43, 0x4D, 0x44, 0x4C, 0x49, 0x4E, 0x45, 0xD9, 0x9D, 0x2F, 0x75, 0x73,
            0x72, 0x2F, 0x6C, 0x6F, 0x63, 0x61, 0x6C, 0x2F, 0x62, 0x69, 0x6E, 0x2F, 0x67, 0x65,
            0x74, 0x68, 0x20, 0x2D, 0x2D, 0x6D, 0x61, 0x69, 0x6E, 0x6E, 0x65, 0x74, 0x20, 0x2D,
            0x2D, 0x64, 0x61, 0x74, 0x61, 0x64, 0x69, 0x72, 0x20, 0x2F, 0x64, 0x61, 0x74, 0x61,
            0x2D, 0x65, 0x74, 0x68, 0x2F, 0x67, 0x65, 0x74, 0x68, 0x20, 0x2D, 0x2D, 0x61, 0x75,
            0x74, 0x68, 0x72, 0x70, 0x63, 0x2E, 0x6A, 0x77, 0x74, 0x73, 0x65, 0x63, 0x72, 0x65,
            0x74, 0x20, 0x2F, 0x64, 0x61, 0x74, 0x61, 0x2D, 0x65, 0x74, 0x68, 0x2F, 0x6A, 0x77,
            0x74, 0x73, 0x65, 0x63, 0x72, 0x65, 0x74, 0x2F, 0x6A, 0x77, 0x74, 0x2E, 0x68, 0x65,
            0x78, 0x20, 0x2D, 0x2D, 0x6D, 0x65, 0x74, 0x72, 0x69, 0x63, 0x73, 0x20, 0x2D, 0x2D,
            0x6D, 0x65, 0x74, 0x72, 0x69, 0x63, 0x73, 0x2E, 0x61, 0x64, 0x64, 0x72, 0x20, 0x31,
            0x32, 0x37, 0x2E, 0x30, 0x2E, 0x30, 0x2E, 0x31, 0x20, 0x2D, 0x2D, 0x6D, 0x65, 0x74,
            0x72, 0x69, 0x63, 0x73, 0x2E, 0x70, 0x6F, 0x72, 0x74, 0x20, 0x36, 0x30, 0x36, 0x31,
            0xAE, 0x5F, 0x43, 0x41, 0x50, 0x5F, 0x45, 0x46, 0x46, 0x45, 0x43, 0x54, 0x49, 0x56,
            0x45, 0xA1, 0x30, 0xB0, 0x5F, 0x53, 0x45, 0x4C, 0x49, 0x4E, 0x55, 0x58, 0x5F, 0x43,
            0x4F, 0x4E, 0x54, 0x45, 0x58, 0x54, 0xAB, 0x75, 0x6E, 0x63, 0x6F, 0x6E, 0x66, 0x69,
            0x6E, 0x65, 0x64, 0x0A, 0xAF, 0x5F, 0x53, 0x59, 0x53, 0x54, 0x45, 0x4D, 0x44, 0x5F,
            0x43, 0x47, 0x52, 0x4F, 0x55, 0x50, 0xD9, 0x22, 0x2F, 0x73, 0x79, 0x73, 0x74, 0x65,
            0x6D, 0x2E, 0x73, 0x6C, 0x69, 0x63, 0x65, 0x2F, 0x67, 0x65, 0x74, 0x68, 0x2D, 0x6D,
            0x61, 0x69, 0x6E, 0x6E, 0x65, 0x74, 0x2E, 0x73, 0x65, 0x72, 0x76, 0x69, 0x63, 0x65,
            0xAD, 0x5F, 0x53, 0x59, 0x53, 0x54, 0x45, 0x4D, 0x44, 0x5F, 0x55, 0x4E, 0x49, 0x54,
            0xB4, 0x67, 0x65, 0x74, 0x68, 0x2D, 0x6D, 0x61, 0x69, 0x6E, 0x6E, 0x65, 0x74, 0x2E,
            0x73, 0x65, 0x72, 0x76, 0x69, 0x63, 0x65, 0xAE, 0x5F, 0x53, 0x59, 0x53, 0x54, 0x45,
            0x4D, 0x44, 0x5F, 0x53, 0x4C, 0x49, 0x43, 0x45, 0xAC, 0x73, 0x79, 0x73, 0x74, 0x65,
            0x6D, 0x2E, 0x73, 0x6C, 0x69, 0x63, 0x65, 0xB6, 0x5F, 0x53, 0x59, 0x53, 0x54, 0x45,
            0x4D, 0x44, 0x5F, 0x49, 0x4E, 0x56, 0x4F, 0x43, 0x41, 0x54, 0x49, 0x4F, 0x4E, 0x5F,
            0x49, 0x44, 0xD9, 0x20, 0x35, 0x33, 0x30, 0x33, 0x35, 0x36, 0x66, 0x36, 0x33, 0x63,
            0x64, 0x31, 0x34, 0x64, 0x64, 0x61, 0x62, 0x30, 0x66, 0x65, 0x64, 0x31, 0x65, 0x38,
            0x30, 0x64, 0x36, 0x61, 0x30, 0x35, 0x65, 0x63, 0xA8, 0x5F, 0x42, 0x4F, 0x4F, 0x54,
            0x5F, 0x49, 0x44, 0xD9, 0x20, 0x62, 0x36, 0x36, 0x36, 0x31, 0x61, 0x66, 0x39, 0x36,
            0x33, 0x64, 0x38, 0x34, 0x61, 0x62, 0x37, 0x39, 0x62, 0x33, 0x38, 0x32, 0x33, 0x63,
            0x62, 0x65, 0x66, 0x35, 0x39, 0x66, 0x37, 0x33, 0x64, 0xAB, 0x5F, 0x4D, 0x41, 0x43,
            0x48, 0x49, 0x4E, 0x45, 0x5F, 0x49, 0x44, 0xD9, 0x20, 0x35, 0x61, 0x32, 0x63, 0x39,
            0x38, 0x66, 0x38, 0x35, 0x31, 0x64, 0x32, 0x34, 0x33, 0x64, 0x33, 0x38, 0x36, 0x61,
            0x62, 0x62, 0x35, 0x64, 0x37, 0x62, 0x39, 0x31, 0x32, 0x64, 0x64, 0x31, 0x66, 0xA9,
            0x5F, 0x48, 0x4F, 0x53, 0x54, 0x4E, 0x41, 0x4D, 0x45, 0xA7, 0x66, 0x72, 0x61, 0x63,
            0x74, 0x61, 0x6C, 0xA7, 0x4D, 0x45, 0x53, 0x53, 0x41, 0x47, 0x45, 0xD9, 0xC2, 0x49,
            0x4E, 0x46, 0x4F, 0x20, 0x5B, 0x30, 0x33, 0x2D, 0x32, 0x33, 0x7C, 0x30, 0x34, 0x3A,
            0x33, 0x30, 0x3A, 0x34, 0x39, 0x2E, 0x35, 0x30, 0x38, 0x5D, 0x20, 0x49, 0x6D, 0x70,
            0x6F, 0x72, 0x74, 0x65, 0x64, 0x20, 0x6E, 0x65, 0x77, 0x20, 0x70, 0x6F, 0x74, 0x65,
            0x6E, 0x74, 0x69, 0x61, 0x6C, 0x20, 0x63, 0x68, 0x61, 0x69, 0x6E, 0x20, 0x73, 0x65,
            0x67, 0x6D, 0x65, 0x6E, 0x74, 0x20, 0x20, 0x20, 0x20, 0x20, 0x62, 0x6C, 0x6F, 0x63,
            0x6B, 0x73, 0x3D, 0x31, 0x20, 0x20, 0x20, 0x20, 0x74, 0x78, 0x73, 0x3D, 0x36, 0x30,
            0x30, 0x20, 0x20, 0x20, 0x20, 0x20, 0x6D, 0x67, 0x61, 0x73, 0x3D, 0x32, 0x34, 0x2E,
            0x36, 0x39, 0x37, 0x20, 0x20, 0x65, 0x6C, 0x61, 0x70, 0x73, 0x65, 0x64, 0x3D, 0x33,
            0x31, 0x36, 0x2E, 0x35, 0x39, 0x34, 0x6D, 0x73, 0x20, 0x20, 0x20, 0x20, 0x6D, 0x67,
            0x61, 0x73, 0x70, 0x73, 0x3D, 0x37, 0x38, 0x2E, 0x30, 0x30, 0x37, 0x20, 0x20, 0x6E,
            0x75, 0x6D, 0x62, 0x65, 0x72, 0x3D, 0x31, 0x36, 0x2C, 0x38, 0x38, 0x37, 0x2C, 0x38,
            0x38, 0x36, 0x20, 0x68, 0x61, 0x73, 0x68, 0x3D, 0x30, 0x37, 0x63, 0x33, 0x65, 0x36,
            0x2E, 0x2E, 0x63, 0x34, 0x36, 0x37, 0x33, 0x39, 0x20, 0x64, 0x69, 0x72, 0x74, 0x79,
            0x3D, 0x31, 0x30, 0x32, 0x32, 0x2E, 0x34, 0x30, 0x4D, 0x69, 0x42,
        ];

        connection.client.write_all(&buf).unwrap();

        // Make sure one messages is received
        let _received1 = connection.service.take_messages();

        // The handler should return without an error
        connection.client.shutdown(Shutdown::Both).unwrap();
        assert!(connection.thread.join().is_ok());
    }

    struct FluentBitFixture {
        client: TcpStream,
        thread: JoinHandle<Result<()>>,
        service: ServiceMock<LogEntryMsg>,
    }

    #[fixture]
    fn connection() -> FluentBitFixture {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let local_address = listener.local_addr().unwrap();
        let service = ServiceMock::new();
        let sender = service.mbox.clone();

        let client = TcpStream::connect(local_address).unwrap();
        let (server, _) = listener.accept().unwrap();

        let sender = LogEntrySender::new(sender);
        let handler = FluentBitConnectionHandler {
            sender,
            extra_fields: vec![],
        };
        let thread = thread::spawn(move || handler.handle_connection(server));

        FluentBitFixture {
            client,
            thread,
            service,
        }
    }

    struct FluentBitMessageFixture {
        msg: FluentdMessage,
        bytes: Vec<u8>,
    }

    #[fixture]
    fn message() -> FluentBitMessageFixture {
        let msg = FluentdMessage(
            Utc::now(),
            HashMap::from([(
                "MESSAGE".to_owned(),
                FluentdValue::String("something happened on the way to the moon".into()),
            )]),
        );
        let bytes = rmp_serde::to_vec(&msg).unwrap();
        FluentBitMessageFixture { msg, bytes }
    }
}
