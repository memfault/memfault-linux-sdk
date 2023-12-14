//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    net::{IpAddr, Shutdown, SocketAddr, TcpStream},
    time::Duration,
};

use eyre::Result;

pub trait CanConnect {
    fn new(timeout: Duration) -> Self;
    fn can_connect(&self, ip_addr: &IpAddr, port: u16) -> Result<()>;
}

pub struct TcpConnectionChecker {
    timeout: Duration,
}

impl CanConnect for TcpConnectionChecker {
    fn new(timeout: Duration) -> Self {
        Self { timeout }
    }

    fn can_connect(&self, ip_addr: &IpAddr, port: u16) -> Result<()> {
        let socket = SocketAddr::new(*ip_addr, port);
        let stream = TcpStream::connect_timeout(&socket, self.timeout)?;
        stream.shutdown(Shutdown::Both)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        net::{IpAddr, TcpListener},
        str::FromStr,
        time::Duration,
    };

    use rstest::rstest;

    use super::{CanConnect, TcpConnectionChecker};

    #[rstest]
    fn test_localhost_reachable() {
        let ip_str = "127.0.0.1";
        let port = 19443;

        // Need a TCP listener to be running on the port
        // TcpConnectionChecker will try to create a connection with
        let listener = TcpListener::bind(format!("{}:{}", ip_str, port))
            .expect("Could not create TcpListener for testing!");

        let connection_checker = TcpConnectionChecker::new(Duration::from_secs(10));
        assert!(connection_checker
            .can_connect(
                &IpAddr::from_str(ip_str)
                    .unwrap_or_else(|_| panic!("{} should be parseable to an IP Address", ip_str)),
                port
            )
            .is_ok());
        drop(listener);
    }

    #[rstest]
    fn test_unreachable_ip_errors() {
        let ip_str = "127.0.0.1";
        let connection_checker = TcpConnectionChecker::new(Duration::from_secs(10));
        assert!(connection_checker
            .can_connect(
                &IpAddr::from_str(ip_str)
                    .unwrap_or_else(|_| panic!("{} should be parseable to an IP Address", ip_str)),
                443
            )
            .is_err());
    }
}
