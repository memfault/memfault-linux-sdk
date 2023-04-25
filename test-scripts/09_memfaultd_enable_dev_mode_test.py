#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import pexpect
from qemu import QEMU


def test_start(qemu: QEMU):
    qemu.exec_cmd("memfaultctl enable-dev-mode")
    qemu.child().expect("Enabling developer mode")
    qemu.systemd_wait_for_service_state("memfaultctl.service", "active")

    qemu.exec_cmd("memfaultctl enable-dev-mode")
    qemu.child().expect("Developer mode is already enabled")

    # Wait for the service to restart. If we disable immediately, there is a race condition where
    # memfaultd will try to read the config while the CLI is still writing to it.
    qemu.wait_for_memfaultd_start()

    qemu.exec_cmd("memfaultctl disable-dev-mode")
    qemu.child().expect("Disabling developer mode")

    qemu.exec_cmd("memfaultctl disable-dev-mode")
    qemu.child().expect("Developer mode is already disabled")

    qemu.wait_for_memfaultd_start()

    # Check that the service restarted:
    qemu.exec_cmd("journalctl -u memfaultd.service")
    qemu.child().expect("Stopped memfaultd daemon")
    qemu.child().expect("Starting memfaultd daemon")
    qemu.child().expect("Starting with developer mode enabled")

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
