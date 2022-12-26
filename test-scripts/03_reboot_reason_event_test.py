#
# Copyright (c) Memfault, Inc.
# See License.txt for details
from memfault_service_tester import MemfaultServiceTester
from qemu import QEMU

# Assumptions:
# - The machine/qemu is built with a valid project key of a project on app.memfault.com,
#   or whatever the underlying QEMU instance points at.
# - The MEMFAULT_E2E_* environment variables are set to match whatever the underlying
#   QEMU instance points at.


def test_reboot_reason_user_reset(
    qemu: QEMU, memfault_service_tester: MemfaultServiceTester, qemu_device_id: str
):
    # enable data collection, so that the reboot event can get captured
    qemu.exec_cmd("memfaultd --enable-data-collection")

    # Stream memfaultd's log
    qemu.exec_cmd("journalctl --follow --unit=memfaultd.service &")

    # Wait until it has been restarted after enabling data collection:
    qemu.child().expect("Stopped memfaultd daemon")
    qemu.child().expect("Started memfaultd daemon")

    qemu.exec_cmd("reboot")
    qemu.child().expect("reboot: Restarting system")
    qemu.child().expect(" login:")

    events = memfault_service_tester.poll_reboot_events_until_count(
        1, device_serial=qemu_device_id, timeout_secs=60
    )
    assert events
    assert events[0]["reason"] == 2  # User Reset


def test_reboot_reason_already_tracked(
    qemu: QEMU, memfault_service_tester: MemfaultServiceTester, qemu_device_id: str
):
    # enable data collection, so that the reboot event can get captured
    qemu.exec_cmd("memfaultd --enable-data-collection")

    # Stream memfaultd's log
    qemu.exec_cmd("journalctl --follow --unit=memfaultd.service &")

    # Wait until it has been restarted after enabling data collection:
    qemu.child().expect("Stopped memfaultd daemon")
    qemu.child().expect("Started memfaultd daemon")

    qemu.exec_cmd("systemctl restart memfaultd")

    qemu.child().expect("reboot:: boot_id already tracked")


def test_reboot_reason_api(
    qemu: QEMU, memfault_service_tester: MemfaultServiceTester, qemu_device_id: str
):
    # enable data collection, so that the reboot event can get captured
    qemu.exec_cmd("memfaultctl enable-data-collection")

    # Stream memfaultd's log
    qemu.exec_cmd("journalctl --follow --unit=memfaultd.service &")

    # Wait until it has been restarted after enabling data collection:
    qemu.child().expect("Stopped memfaultd daemon")
    qemu.child().expect("Started memfaultd daemon")

    # Reboot with code 4 "Low Power"
    qemu.exec_cmd("memfaultctl reboot --reason 4")
    qemu.child().expect("reboot: Restarting system")
    qemu.child().expect(" login:")

    events = memfault_service_tester.poll_reboot_events_until_count(
        1, device_serial=qemu_device_id, timeout_secs=60
    )
    assert events
    assert events[0]["reason"] == 4
