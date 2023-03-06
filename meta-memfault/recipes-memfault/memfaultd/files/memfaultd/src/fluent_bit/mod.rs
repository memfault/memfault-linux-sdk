//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! fluent-bit
//!
//! Provides FluentBitReceiver to start listening to TCP connections from
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
use chrono::{DateTime, Utc};
use eyre::Result;
use log::{trace, warn};
use rmp_serde::Deserializer;
use serde::{Deserialize, Serialize};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{Receiver, SyncSender};
use std::thread;
use std::{collections::HashMap, net::SocketAddr};
use threadpool::ThreadPool;

use crate::config::Config;

mod decode_time;

/// A TCP server compatible with fluent-bit tcp output plugin.
/// Incoming messages will be delivered on `receiver`.
pub struct FluentBitReceiver {
    pub receiver: Receiver<FluentdMessage>,
}

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

impl FluentBitReceiver {
    /// Create an instance and spawn threads to handle incoming connections.
    pub fn start(config: FluentBitConfig) -> Result<Self> {
        let (sender, receiver) = std::sync::mpsc::sync_channel(config.max_buffered_lines);

        let listener = TcpListener::bind(config.bind_address)?;
        thread::spawn(move || Self::run(listener, sender, config.max_connections));

        Ok(FluentBitReceiver { receiver })
    }

    fn run(
        listener: TcpListener,
        sender: SyncSender<FluentdMessage>,
        max_connections: usize,
    ) -> Result<()> {
        let pool = ThreadPool::new(max_connections);

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    trace!(
                        "Connection from {:?} - Threads {}/{}",
                        stream.peer_addr(),
                        pool.active_count(),
                        pool.max_count()
                    );
                    let thread_sender = sender.clone();
                    pool.execute(move || {
                        if let Err(e) = Self::handle_connection(stream, thread_sender) {
                            warn!("Error while handling connection: {}", e)
                        }
                    })
                }
                Err(e) => {
                    warn!("fluentbit listener error {}", e);
                    break;
                }
            }
        }
        trace!("done listening - waiting for pool to terminate");
        pool.join();
        trace!("pool joined.");

        Ok(())
    }

    fn handle_connection(stream: TcpStream, sender: SyncSender<FluentdMessage>) -> Result<()> {
        let mut de = Deserializer::new(stream);

        loop {
            match FluentdMessage::deserialize(&mut de) {
                Ok(msg) => sender.send(msg)?,
                Err(e) => {
                    match e {
                        rmp_serde::decode::Error::InvalidMarkerRead(e)
                            if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                        {
                            // silently ignore end of stream
                        }
                        _ => warn!("FluentD decoding error: {:?}", e),
                    }
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
    use std::{
        io::Write, net::Shutdown, sync::mpsc::sync_channel, thread::JoinHandle, time::Duration,
    };

    use crate::test_utils::setup_logger;
    use rstest::{fixture, rstest};

    use super::*;

    #[rstest]
    fn deserialize_bogus_message(_setup_logger: (), mut connection: FluentBitFixture) {
        connection.client.write_all("bogus".as_bytes()).unwrap();
        connection.client.shutdown(Shutdown::Both).unwrap();

        // Make sure there is nothing received
        let received = connection.receiver.recv_timeout(Duration::from_millis(10));
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

        // Make sure message is received
        let received = connection
            .receiver
            .recv_timeout(Duration::from_millis(10))
            .unwrap();
        assert_eq!(received.0, message.msg.0);
        assert_eq!(
            serde_json::to_string(&received.1).unwrap(),
            serde_json::to_string(&message.msg.1).unwrap()
        );
        connection.client.shutdown(Shutdown::Both).unwrap();

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

        // Make sure message is received
        let received = connection
            .receiver
            .recv_timeout(Duration::from_millis(10))
            .unwrap();
        assert_eq!(received.0, message.msg.0);
        assert_eq!(
            serde_json::to_string(&received.1).unwrap(),
            serde_json::to_string(&message.msg.1).unwrap()
        );

        // The handler should return without an error
        connection.client.shutdown(Shutdown::Both).unwrap();
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

        // Make sure two messages are received
        let received1 = connection
            .receiver
            .recv_timeout(Duration::from_millis(10))
            .unwrap();
        let received2 = connection
            .receiver
            .recv_timeout(Duration::from_millis(10))
            .unwrap();
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
        connection.client.shutdown(Shutdown::Both).unwrap();
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

        let thread = thread::spawn(move || FluentBitReceiver::handle_connection(server, sender));

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
