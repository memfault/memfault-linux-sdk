#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import time
from typing import cast

import pexpect
import pytest

from .qemu import QEMU


@pytest.fixture()
def data_collection_enabled() -> bool:
    return False


def test_start(qemu: QEMU) -> None:
    qemu.exec_cmd("memfaultctl enable-data-collection")
    qemu.child().expect("Enabling data collection")
    qemu.systemd_wait_for_service_state("memfaultd.service", "active")

    qemu.exec_cmd("memfaultctl enable-data-collection")
    qemu.child().expect("Data collection is already enabled.")

    # Wait for the service to restart. If we disable immediately, there is a race condition where
    # memfaultd will try to read the config while the CLI is still writing to it.
    qemu.wait_for_memfaultd_start()

    qemu.exec_cmd("memfaultctl disable-data-collection")
    qemu.child().expect("Disabling data collection.")

    # Work-around for checking the state in the next line before memfaultd has even restarted:
    # TODO: track the journal logs instead of using `systemctl is-active`
    time.sleep(0.5)

    qemu.systemd_wait_for_service_state("memfaultd.service", "active")

    qemu.exec_cmd("memfaultctl disable-data-collection")
    qemu.child().expect("Data collection is already disabled.")

    # Check that the service restarted:
    qemu.exec_cmd("journalctl -u memfaultd.service")
    qemu.child().expect("Stopped memfaultd daemon")
    qemu.child().expect("Starting memfaultd daemon")

    # Check that the service did not fail:
    # TODO: pexpect does not have a "not expecting" API :/
    qemu.exec_cmd("journalctl -u memfaultd.service")
    output = b""
    while True:
        try:
            output += qemu.child().read_nonblocking(size=1024, timeout=cast(int, 0.5))
        except pexpect.TIMEOUT:
            break
    assert b"memfaultd.service: Scheduled restart job" not in output


def test_via_memfaultctl(qemu: QEMU) -> None:
    qemu.exec_cmd("memfaultctl enable-data-collection")
    qemu.child().expect("Enabling data collection.")
    qemu.systemd_wait_for_service_state("memfaultd.service", "active")

    qemu.exec_cmd("memfaultctl enable-data-collection")
    qemu.child().expect("Data collection is already enabled.")

    # Wait for the service to restart. If we disable immediately, there is a race condition where
    # memfaultd will try to read the config while the CLI is still writing to it.
    qemu.wait_for_memfaultd_start()

    qemu.exec_cmd("memfaultctl disable-data-collection")
    qemu.child().expect("Disabling data collection.")

    qemu.wait_for_memfaultd_start()

    qemu.exec_cmd("memfaultctl disable-data-collection")
    qemu.child().expect("Data collection is already disabled.")

    # Check that the service restarted:
    qemu.exec_cmd("journalctl -u memfaultd.service")
    qemu.child().expect("Stopped memfaultd daemon")
    qemu.child().expect("Starting memfaultd daemon")

    # Check that the service did not fail:
    # TODO: pexpect does not have a "not expecting" API :/
    qemu.exec_cmd("journalctl -u memfaultd.service")
    output = b""
    while True:
        try:
            output += qemu.child().read_nonblocking(size=1024, timeout=cast(int, 0.5))
        except pexpect.TIMEOUT:
            break
    assert b"memfaultd.service: Scheduled restart job" not in output
