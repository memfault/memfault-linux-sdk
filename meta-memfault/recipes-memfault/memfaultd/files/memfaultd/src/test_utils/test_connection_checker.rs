//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{cell::RefCell, net::IpAddr, time::Duration};

use eyre::{eyre, Result};

use crate::util::can_connect::CanConnect;

#[derive(Debug, Clone, Copy)]
pub struct TestConnectionChecker {}

thread_local! {
    static CONNECTED: RefCell<bool>  = RefCell::new(true);
}

impl TestConnectionChecker {
    pub fn connect() {
        CONNECTED.with(|c| *c.borrow_mut() = true);
    }

    pub fn disconnect() {
        CONNECTED.with(|c| *c.borrow_mut() = false);
    }
}

impl CanConnect for TestConnectionChecker {
    fn new(_timeout: Duration) -> Self {
        Self {}
    }

    fn can_connect(&self, _ip: &IpAddr, _port: u16) -> Result<()> {
        let mut connected_status = true;
        CONNECTED.with(|c| connected_status = *c.borrow());
        if connected_status {
            Ok(())
        } else {
            Err(eyre!("MockPinger is set to disconnected"))
        }
    }
}
