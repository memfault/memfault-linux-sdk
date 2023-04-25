//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::Result;
use log::{trace, warn};
use std::io::Read;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::thread;
use threadpool::ThreadPool;

/// A TCP server that spawns threads to handle incoming connections.
/// Incoming connections will be delegated to a `TcpConnectionHandler`.
pub struct ThreadedTcpServer {}

pub trait TcpConnectionHandler: Send + Sync + Clone + 'static {
    fn handle_connection(&self, stream: TcpStream) -> Result<()>;
}

impl ThreadedTcpServer {
    pub fn start(
        bind_address: SocketAddr,
        max_connections: usize,
        handler: impl TcpConnectionHandler,
    ) -> Result<Self> {
        let listener = TcpListener::bind(bind_address)?;
        thread::spawn(move || Self::run(listener, max_connections, handler));
        Ok(ThreadedTcpServer {})
    }

    fn run(
        listener: TcpListener,
        max_connections: usize,
        handler: impl TcpConnectionHandler,
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
                    let handler = handler.clone();
                    pool.execute(move || {
                        if let Err(e) = handler.handle_connection(stream) {
                            warn!("Error while handling connection: {}", e)
                        }
                    })
                }
                Err(e) => {
                    warn!("TCP server listener error {}", e);
                    break;
                }
            }
        }
        trace!("Done listening - waiting for pool to terminate");
        pool.join();
        trace!("Pool joined.");

        Ok(())
    }
}

/// Handler that reads and drops all data.
#[derive(Clone)]
pub struct TcpNullConnectionHandler {}

impl TcpConnectionHandler for TcpNullConnectionHandler {
    fn handle_connection(&self, mut stream: TcpStream) -> Result<()> {
        loop {
            let mut buf = [0; 8 * 1024];
            match stream.read(&mut buf) {
                Ok(0) => break, // EOF
                Ok(_) => {}     // drop the data
                Err(e) => {
                    warn!("TCP read error: {:?}", e);
                    break;
                }
            }
        }
        Ok(())
    }
}
