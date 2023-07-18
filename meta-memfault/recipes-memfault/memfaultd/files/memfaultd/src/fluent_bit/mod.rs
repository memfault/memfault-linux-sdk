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
use std::sync::mpsc::{Receiver, SyncSender};
use std::{collections::HashMap, net::SocketAddr};

use chrono::{DateTime, Utc};
use eyre::Result;
use log::warn;
use rmp_serde::Deserializer;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::util::tcp_server::{TcpConnectionHandler, TcpNullConnectionHandler, ThreadedTcpServer};

mod decode_time;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FluentdValue {
    String(String),
    Float(f64),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FluentdMessage(
    #[serde(with = "decode_time")] pub DateTime<Utc>,
    pub HashMap<String, FluentdValue>,
);

#[derive(Clone)]
pub struct FluentBitConnectionHandler {
    sender: SyncSender<FluentdMessage>,
}

impl FluentBitConnectionHandler {
    /// Starts the fluent-bit server with a handler delivers parsed messages to a receiver channel.
    pub fn start(config: FluentBitConfig) -> Result<(ThreadedTcpServer, Receiver<FluentdMessage>)> {
        let (sender, receiver) = std::sync::mpsc::sync_channel(config.max_buffered_lines);
        let server = ThreadedTcpServer::start(
            config.bind_address,
            config.max_connections,
            FluentBitConnectionHandler { sender },
        )?;
        Ok((server, receiver))
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
}

impl TcpConnectionHandler for FluentBitConnectionHandler {
    fn handle_connection(&self, stream: TcpStream) -> Result<()> {
        let mut de = Deserializer::new(stream);

        loop {
            match FluentdMessage::deserialize(&mut de) {
                Ok(msg) => {
                    if self.sender.send(msg).is_err() {
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
    max_buffered_lines: usize,
    max_connections: usize,
}

impl From<&Config> for FluentBitConfig {
    fn from(config: &Config) -> Self {
        Self {
            bind_address: config.config_file.fluent_bit.bind_address,
            max_buffered_lines: config.config_file.fluent_bit.max_buffered_lines,
            max_connections: config.config_file.fluent_bit.max_connections,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::{
        io::Write, net::Shutdown, sync::mpsc::sync_channel, thread, thread::JoinHandle,
        time::Duration,
    };

    use rstest::{fixture, rstest};

    use crate::test_utils::setup_logger;

    use super::*;

    #[rstest]
    fn deserialize_bogus_message(_setup_logger: (), mut connection: FluentBitFixture) {
        connection.client.write_all("bogus".as_bytes()).unwrap();
        connection.client.shutdown(Shutdown::Both).unwrap();

        // Make sure there is nothing received
        let received = connection.receiver.recv();
        assert!(received.is_err());

        // The handler should return without an error
        assert!(connection.thread.join().is_ok());
    }

    #[rstest]
    fn deserialize_one_message(
        _setup_logger: (),
        mut connection: FluentBitFixture,
        message: FluentBitMessageFixture,
    ) {
        connection.client.write_all(&message.bytes).unwrap();
        connection.client.shutdown(Shutdown::Both).unwrap();

        // Make sure message is received
        let received = connection.receiver.recv().unwrap();
        assert_eq!(received.0, message.msg.0);
        assert_eq!(
            serde_json::to_string(&received.1).unwrap(),
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

        // Make sure message is received
        let received = connection.receiver.recv().unwrap();
        assert_eq!(received.0, message.msg.0);
        assert_eq!(
            serde_json::to_string(&received.1).unwrap(),
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

        // Make sure two messages are received
        let received1 = connection.receiver.recv().unwrap();
        let received2 = connection.receiver.recv().unwrap();
        assert_eq!(received1.0, message.msg.0);
        assert_eq!(
            serde_json::to_string(&received1.1).unwrap(),
            serde_json::to_string(&message.msg.1).unwrap()
        );

        assert_eq!(received2.0, message2.msg.0);
        assert_eq!(
            serde_json::to_string(&received2.1).unwrap(),
            serde_json::to_string(&message2.msg.1).unwrap()
        );

        // The handler should return without an error
        assert!(connection.thread.join().is_ok());
    }

    struct FluentBitFixture {
        client: TcpStream,
        thread: JoinHandle<Result<()>>,
        receiver: Receiver<FluentdMessage>,
    }

    #[fixture]
    fn connection() -> FluentBitFixture {
        let (sender, receiver) = sync_channel(1);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let local_address = listener.local_addr().unwrap();

        let client = TcpStream::connect(local_address).unwrap();
        let (server, _) = listener.accept().unwrap();

        let handler = FluentBitConnectionHandler { sender };
        let thread = thread::spawn(move || handler.handle_connection(server));

        FluentBitFixture {
            client,
            thread,
            receiver,
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
