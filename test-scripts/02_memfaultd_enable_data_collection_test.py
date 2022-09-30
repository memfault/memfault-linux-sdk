#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import pexpect
from qemu import QEMU


def test_start(qemu: QEMU):
    qemu.exec_cmd("memfaultd --enable-data-collection")
    qemu.child().expect("Enabling data collection")
    qemu.systemd_wait_for_service_state("memfaultd.service", "active")

    qemu.exec_cmd("memfaultd --enable-data-collection")
    qemu.child().expect("Data collection state already set")

    qemu.exec_cmd("memfaultd --disable-data-collection")
    qemu.child().expect("Disabling data collection")
    qemu.systemd_wait_for_service_state("memfaultd.service", "active")

    qemu.exec_cmd("memfaultd --disable-data-collection")
    qemu.child().expect("Data collection state already set")

    # Check that the service restarted:
    qemu.exec_cmd("journalctl -u memfaultd.service")
    qemu.child().expect("Stopped memfaultd daemon")
    qemu.child().expect("Started memfaultd daemon")

    # Check that the service did not fail:
    # FIXME: pexpect does not have a "not expecting" API :/
    qemu.exec_cmd("journalctl -u memfaultd.service")
    output = b""
    while True:
        try:
            output += qemu.child().read_nonblocking(size=1024, timeout=0.5)
        except pexpect.TIMEOUT:
            break
    assert b"memfaultd.service: Scheduled restart job" not in output
